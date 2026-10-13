// SPDX-License-Identifier: Apache-2.0
// cloud.rs — cloud-drive tools routed to connected MCP services.
//
// There is no bundled OAuth client for drive providers; the platform design
// (docs/agent-tab-design.md §4.3) is MCP-first: connect Google Drive,
// OneDrive, or Dropbox as an MCP server in the Connections tab, and
// cloud.drive_* routes to that server's tools. When nothing matches, the
// error names exactly what to connect and lists the candidate servers/tools
// so the model can route manually via mcp.call_tool — never a silent fake.

use std::collections::HashMap;

use serde_json::{json, Value};

#[derive(Debug, Clone)]
struct Candidate {
    score: i32,
    server_id: String,
    server_name: String,
    tool_name: String,
}

fn drive_keys(drive_id: &str) -> Vec<String> {
    let d = drive_id.trim().to_lowercase();
    match d.as_str() {
        "google" | "gdrive" | "google-drive" | "googledrive" => {
            vec!["google drive".into(), "drive".into(), "google".into()]
        }
        "onedrive" | "one-drive" | "microsoft" | "office365" => {
            vec!["onedrive".into(), "one drive".into(), "microsoft".into()]
        }
        "dropbox" => vec!["dropbox".into()],
        other => vec![other.to_string()],
    }
}

fn server_matches(keys: &[String], name: &str) -> bool {
    let n = name.to_lowercase();
    keys.iter().any(|k| n.contains(k.as_str())) || n.contains("drive")
}

fn tool_score(op: &str, tool_name: &str) -> i32 {
    let n = tool_name.to_lowercase();
    let fileish = n.contains("file") || n.contains("folder") || n.contains("drive");
    match op {
        "list" => {
            if n.contains("list") && fileish {
                3
            } else if n.contains("list") {
                2
            } else if n.contains("files") {
                1
            } else {
                0
            }
        }
        "read" => {
            if (n.contains("read") || n.contains("download") || n.contains("get")) && fileish {
                3
            } else if n.contains("read") || n.contains("download") {
                2
            } else {
                0
            }
        }
        "write" => {
            if (n.contains("upload")
                || n.contains("write")
                || n.contains("create")
                || n.contains("save"))
                && fileish
            {
                3
            } else if n.contains("upload") || n.contains("write") {
                2
            } else {
                0
            }
        }
        "search" => {
            if n.contains("search") {
                3
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn enabled_servers(app: &tauri::AppHandle) -> Result<Vec<(String, String)>, String> {
    let conn = crate::mcp::open_db(app)?;
    let mut out = Vec::new();
    let mut stmt = conn
        .prepare("SELECT id, name FROM mcp_servers WHERE enabled = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn enabled_tools(app: &tauri::AppHandle) -> Result<Vec<(String, String)>, String> {
    let conn = crate::mcp::open_db(app)?;
    let mut out = Vec::new();
    let mut stmt = conn
        .prepare("SELECT mcp_server, name FROM tools WHERE mcp_server IS NOT NULL AND enabled = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn candidates(app: &tauri::AppHandle, drive_id: &str, op: &str) -> Result<Vec<Candidate>, String> {
    let servers = enabled_servers(app)?;
    let tools = enabled_tools(app)?;
    let keys = drive_keys(drive_id);
    let mut out: Vec<Candidate> = Vec::new();
    for (sid, sname) in &servers {
        if !server_matches(&keys, sname) {
            continue;
        }
        for (tsid, tname) in &tools {
            if tsid != sid {
                continue;
            }
            let score = tool_score(op, tname);
            if score > 0 {
                out.push(Candidate {
                    score,
                    server_id: sid.clone(),
                    server_name: sname.clone(),
                    tool_name: tname.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(out)
}

pub fn drive_operation(
    app: &tauri::AppHandle,
    drive_id: &str,
    op: &str,
    args: &HashMap<String, Value>,
) -> Result<Value, String> {
    let found = candidates(app, drive_id, op)?;
    if found.is_empty() {
        let servers = enabled_servers(app)?;
        let tools = enabled_tools(app)?;
        let connected: Vec<Value> = servers
            .iter()
            .map(|(sid, sname)| {
                let names: Vec<String> = tools
                    .iter()
                    .filter(|(tsid, _)| tsid == sid)
                    .map(|(_, tname)| tname.clone())
                    .collect();
                json!({ "server_id": sid, "server": sname, "tools": names })
            })
            .collect();
        return Err(format!(
            "cloud drive '{}' has no connected tool for '{}'. Connect a matching service under Connections (Google Drive / OneDrive / Dropbox templates), then retry. Connected servers and tools: {}",
            drive_id,
            op,
            serde_json::to_string(&connected).unwrap_or_default()
        ));
    }
    let pick = &found[0];
    let mut passthrough = serde_json::Map::new();
    for (k, v) in args {
        if k != "drive_id" && k != "account_id" {
            passthrough.insert(k.clone(), v.clone());
        }
    }
    let result = tauri::async_runtime::block_on(crate::mcp::mcp_call_tool(
        app.clone(),
        pick.server_id.clone(),
        pick.tool_name.clone(),
        Value::Object(passthrough).to_string(),
    ))?;
    let alternatives: Vec<String> = found
        .iter()
        .skip(1)
        .take(5)
        .map(|c| format!("{} (on {})", c.tool_name, c.server_name))
        .collect();
    Ok(json!({
        "via": { "server": pick.server_name, "server_id": pick.server_id, "tool": pick.tool_name },
        "ok": result.ok,
        "approval_required": result.approval_required,
        "result": result.result,
        "other_candidates": alternatives,
    }))
}
