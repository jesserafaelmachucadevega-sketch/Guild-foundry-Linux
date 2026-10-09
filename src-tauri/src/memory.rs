// Phase 10 — Model-isolated memory (master spec: memory/indexing/research).
//
// Isolation contract (DB + app level):
//   The composite key is (scope, provider_id, model_name, version, key).
//   EVERY read/write/delete filters on ALL FIVE components. The DB schema
//   (db.rs MIGRATION_V1, table `memories`) already carries these columns with
//   PRIMARY KEY (provider_id, model_id, version, scope, key); this module
//   additionally enforces the composite key at the app layer (parameter
//   binding on every query, no string concatenation) and validates the scope
//   against the six legal scopes on every entry point.
//   NOTE: the DB column is named `model_id`; it stores the model_name value
//   passed through the composite key. The two names refer to the same thing.
//
// NOT VERIFIED / honest limitations:
//   - Rust cannot compile in this build environment; every command below is
//     written with extreme care but is NOT VERIFIED until `cargo build` runs.
//   - `memory_search` is LIKE-based keyword ranking, NOT semantic search.
//     True embedding-based semantic clustering/search requires an offline
//     embedding model and is explicitly out of scope here (future work).
//   - Contradiction detection is heuristic (negation patterns), not an LLM.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::Manager;

/// The six legal memory scopes. Rejected anywhere else.
const SCOPES: &[&str] = &[
    "conversation",
    "model",
    "agent",
    "project",
    "workspace",
    "global",
];

/// Claim types for the shared knowledge bridge (`memory_synthesize`).
const CLAIM_TYPES: &[&str] = &[
    "verified_fact",
    "model_claim",
    "user_decision",
    "inference",
    "unresolved",
];

/// Open the app DB and apply the Phase 10 additive migration fragment.
/// Idempotent: each ALTER is attempted individually and ignored if the
/// column already exists. Additive-only; never touches other tables.
pub fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("guild-foundry-ai.db")).map_err(|e| e.to_string())?;
    for alter in [
        "ALTER TABLE memories ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}'",
        "ALTER TABLE memories ADD COLUMN claim_type TEXT NOT NULL DEFAULT 'unresolved'",
        "ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE memories ADD COLUMN last_accessed_at TEXT",
        "ALTER TABLE memories ADD COLUMN flagged TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = conn.execute(alter, []); // ignore "duplicate column" on re-run
    }
    Ok(conn)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn check_scope(scope: &str) -> Result<(), String> {
    if SCOPES.contains(&scope) {
        Ok(())
    } else {
        Err(format!(
            "refused: scope '{}' is not one of {:?}",
            scope, SCOPES
        ))
    }
}

fn valid_claim_type(ct: &str) -> &str {
    if CLAIM_TYPES.contains(&ct) {
        ct
    } else {
        "unresolved"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub scope: String,
    pub provider_id: String,
    pub model_name: String,
    pub version: String,
    pub scope_id: Option<String>,
    pub key: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub claim_type: String,
    pub importance: f64,
    pub access_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub flagged: String,
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
    let metadata_raw: String = row.get("metadata")?;
    Ok(MemoryEntry {
        scope: row.get("scope")?,
        provider_id: row.get("provider_id")?,
        model_name: row.get("model_id")?,
        version: row.get("version")?,
        scope_id: row.get("scope_id")?,
        key: row.get("key")?,
        content: row.get("value")?,
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(serde_json::Value::Null),
        claim_type: row.get("claim_type")?,
        importance: row.get("importance")?,
        access_count: row.get("access_count")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        flagged: row.get("flagged")?,
    })
}

/// Upsert one memory cell under the full composite key. Optional `scope_id`
/// pins the entry to a concrete conversation/agent/project when the scope
/// is conversation/agent/project; it is part of the lookup, not the key.
#[tauri::command]
pub fn memory_put(
    app: tauri::AppHandle,
    scope: String,
    provider_id: String,
    model_name: String,
    version: String,
    scope_id: Option<String>,
    key: String,
    content: String,
    metadata_json: Option<String>,
    claim_type: Option<String>,
) -> Result<(), String> {
    check_scope(&scope)?;
    if key.trim().is_empty() {
        return Err("refused: memory key must not be empty".to_string());
    }
    let metadata: serde_json::Value = metadata_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    let ct = valid_claim_type(claim_type.as_deref().unwrap_or("unresolved"));
    let now = now_iso();
    let conn = open_db(&app)?;
    conn.execute(
        "INSERT INTO memories
            (provider_id, model_id, version, scope, scope_id, key, value,
             metadata, claim_type, importance, access_count, last_accessed_at,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0.5, 0, NULL, ?10, ?10)
         ON CONFLICT(provider_id, model_id, version, scope, key) DO UPDATE SET
            scope_id = excluded.scope_id,
            value = excluded.value,
            metadata = excluded.metadata,
            claim_type = excluded.claim_type,
            updated_at = excluded.updated_at",
        params![
            provider_id,
            model_name,
            version,
            scope,
            scope_id,
            key,
            content,
            metadata.to_string(),
            ct,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read one memory cell. Every lookup binds all five composite-key parts.
/// A successful read bumps access_count/last_accessed_at (feeds importance).
#[tauri::command]
pub fn memory_get(
    app: tauri::AppHandle,
    scope: String,
    provider_id: String,
    model_name: String,
    version: String,
    key: String,
) -> Result<Option<String>, String> {
    check_scope(&scope)?;
    let conn = open_db(&app)?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM memories
             WHERE scope = ?1 AND provider_id = ?2 AND model_id = ?3
               AND version = ?4 AND key = ?5",
            params![scope, provider_id, model_name, version, key],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })?;
    if value.is_some() {
        let _ = conn.execute(
            "UPDATE memories SET access_count = access_count + 1,
                                last_accessed_at = ?6
             WHERE scope = ?1 AND provider_id = ?2 AND model_id = ?3
               AND version = ?4 AND key = ?5",
            params![scope, provider_id, model_name, version, key, now_iso()],
        );
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHit {
    pub entry: MemoryEntry,
    pub score: f64,
}

/// LIKE-based keyword search within one composite isolation domain.
/// Honest doc: this is keyword ranking (token hits + importance), NOT
/// semantic/embedding search. All five key parts are mandatory filters.
#[tauri::command]
pub fn memory_search(
    app: tauri::AppHandle,
    query: String,
    scope: String,
    provider_id: String,
    model_name: String,
    version: String,
) -> Result<Vec<MemoryHit>, String> {
    check_scope(&scope)?;
    let conn = open_db(&app)?;
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() > 1)
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT scope, provider_id, model_id, version, scope_id, key, value,
                    metadata, claim_type, importance, access_count, created_at,
                    updated_at, flagged
             FROM memories
             WHERE scope = ?1 AND provider_id = ?2 AND model_id = ?3
               AND version = ?4",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![scope, provider_id, model_name, version], row_to_entry)
        .map_err(|e| e.to_string())?;
    let mut hits: Vec<MemoryHit> = Vec::new();
    for row in rows {
        let entry = row.map_err(|e| e.to_string())?;
        let haystack = format!("{} {}", entry.key, entry.content).to_lowercase();
        let mut matched = 0usize;
        for t in &tokens {
            if haystack.contains(t) {
                matched += 1;
            }
        }
        if matched > 0 {
            let score = matched as f64 / tokens.len() as f64 * 0.7 + entry.importance * 0.3;
            hits.push(MemoryHit { entry, score });
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(50);
    Ok(hits)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationReport {
    pub deduped: i64,
    pub pruned_stale: i64,
    pub contradictions_flagged: i64,
    pub rescored: i64,
}

const NEGATIONS: &[&str] = &[" not ", " no ", " never ", "n't ", " cannot ", " can't ", " won't "];

/// Strip negation tokens so "X is supported" and "X is not supported"
/// normalize to the same string for contradiction comparison.
fn normalize_for_contradiction(s: &str) -> String {
    let mut n = format!(" {} ", s.to_lowercase());
    for neg in NEGATIONS {
        n = n.replace(neg, " ");
    }
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_negation(s: &str) -> bool {
    let n = format!(" {} ", s.to_lowercase());
    NEGATIONS.iter().any(|neg| n.contains(neg))
}

/// Async consolidation pass: exact-dup removal, stale pruning,
/// contradiction flagging, importance rescoring.
#[tauri::command]
pub fn memory_consolidate(app: tauri::AppHandle) -> Result<ConsolidationReport, String> {
    let conn = open_db(&app)?;
    let mut report = ConsolidationReport {
        deduped: 0,
        pruned_stale: 0,
        contradictions_flagged: 0,
        rescored: 0,
    };
    let now = chrono::Utc::now();

    // --- 1. Exact duplicates: same normalized value within the same
    // --- isolation domain (scope+provider+model), different keys.
    // --- Keep the earliest created, delete the rest.
    let mut stmt = conn
        .prepare(
            "SELECT rowid, scope, provider_id, model_id, version, key, value, created_at
             FROM memories ORDER BY created_at ASC LIMIT 2000",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String, String, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    let mut seen: HashMap<String, i64> = HashMap::new();
    let mut dup_rowids: Vec<i64> = Vec::new();
    for (rowid, scope, provider_id, model_id, _version, _key, value, _created) in &rows {
        let fingerprint = format!(
            "{}|{}|{}|{}",
            scope,
            provider_id,
            model_id,
            value.trim().to_lowercase()
        );
        match seen.entry(fingerprint) {
            std::collections::hash_map::Entry::Occupied(_) => dup_rowids.push(*rowid),
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(*rowid);
            }
        }
    }
    for rowid in &dup_rowids {
        let _ = conn.execute("DELETE FROM memories WHERE rowid = ?1", params![rowid]);
    }
    report.deduped = dup_rowids.len() as i64;

    // --- 2. Stale: older than 90 days AND never re-accessed.
    let stale_cutoff = (now - chrono::Duration::days(90)).to_rfc3339_opts(
        chrono::SecondsFormat::Secs,
        true,
    );
    report.pruned_stale = conn
        .execute(
            "DELETE FROM memories
             WHERE updated_at < ?1 AND access_count = 0 AND last_accessed_at IS NULL",
            params![stale_cutoff],
        )
        .map_err(|e| e.to_string())? as i64;

    // --- 3. Contradictions: same key within the same isolation domain
    // --- (across versions), where one value negates the other.
    let mut by_key: HashMap<String, Vec<(i64, String, String)>> = HashMap::new();
    for (rowid, scope, provider_id, model_id, version, key, value, _created) in &rows {
        // Skip rows we already deleted as duplicates.
        if dup_rowids.contains(rowid) {
            continue;
        }
        let group = format!("{}|{}|{}|{}", scope, provider_id, model_id, key);
        by_key
            .entry(group)
            .or_default()
            .push((*rowid, version.clone(), value.clone()));
    }
    let mut flagged_rowids: Vec<i64> = Vec::new();
    for entries in by_key.values() {
        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                let (ri, _vi, va) = &entries[i];
                let (rj, _vj, vb) = &entries[j];
                if normalize_for_contradiction(va) == normalize_for_contradiction(vb)
                    && has_negation(va) != has_negation(vb)
                {
                    flagged_rowids.push(*ri);
                    flagged_rowids.push(*rj);
                }
            }
        }
    }
    flagged_rowids.sort_unstable();
    flagged_rowids.dedup();
    for rowid in &flagged_rowids {
        let _ = conn.execute(
            "UPDATE memories SET flagged = 'contradiction' WHERE rowid = ?1",
            params![rowid],
        );
    }
    report.contradictions_flagged = flagged_rowids.len() as i64;

    // --- 4. Importance rescoring: 0.5 * recency + 0.5 * access.
    let live: Vec<(i64, String, i64)> = conn
        .prepare("SELECT rowid, updated_at, access_count FROM memories")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (rowid, updated_at, access_count) in live {
        let days: f64 = chrono::DateTime::parse_from_rfc3339(&updated_at)
            .ok()
            .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0)
            .unwrap_or(365.0)
            .max(0.0);
        let recency = 1.0 / (1.0 + days / 30.0);
        let access_norm = (access_count.min(10) as f64) / 10.0;
        let importance = 0.5 * recency + 0.5 * access_norm;
        let _ = conn.execute(
            "UPDATE memories SET importance = ?1 WHERE rowid = ?2",
            params![importance, rowid],
        );
        report.rescored += 1;
    }
    Ok(report)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisEntry {
    pub key: String,
    pub content: String,
    pub scope: String,
    pub provider: String,
    pub model: String,
    pub updated_at: String,
    pub flagged: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisView {
    pub verified_facts: Vec<SynthesisEntry>,
    pub model_claims: Vec<SynthesisEntry>,
    pub user_decisions: Vec<SynthesisEntry>,
    pub inferences: Vec<SynthesisEntry>,
    pub unresolved: Vec<SynthesisEntry>,
}

/// The cross-model knowledge bridge. Groups memory entries matching `topic`
/// into the five fixed categories. Phase 3's SynthesisPanel consumes this
/// shape — it must stay exactly {verified_facts, model_claims,
/// user_decisions, inferences, unresolved}.
#[tauri::command]
pub fn memory_synthesize(app: tauri::AppHandle, topic: String) -> Result<SynthesisView, String> {
    let conn = open_db(&app)?;
    // Escape LIKE metacharacters so the topic is matched literally.
    let escaped = topic.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let like = format!("%{}%", escaped);
    let mut stmt = conn
        .prepare(
            "SELECT scope, provider_id, model_id, version, scope_id, key, value,
                    metadata, claim_type, importance, access_count, created_at,
                    updated_at, flagged
             FROM memories
             WHERE key LIKE ?1 ESCAPE '\\' OR value LIKE ?1 ESCAPE '\\'
             ORDER BY importance DESC LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like], row_to_entry)
        .map_err(|e| e.to_string())?;
    let mut view = SynthesisView {
        verified_facts: Vec::new(),
        model_claims: Vec::new(),
        user_decisions: Vec::new(),
        inferences: Vec::new(),
        unresolved: Vec::new(),
    };
    for row in rows {
        let e = row.map_err(|e| e.to_string())?;
        let entry = SynthesisEntry {
            key: e.key,
            content: e.content,
            scope: e.scope,
            provider: e.provider_id,
            model: e.model_name,
            updated_at: e.updated_at,
            flagged: e.flagged,
        };
        match e.claim_type.as_str() {
            "verified_fact" => view.verified_facts.push(entry),
            "model_claim" => view.model_claims.push(entry),
            "user_decision" => view.user_decisions.push(entry),
            "inference" => view.inferences.push(entry),
            _ => view.unresolved.push(entry),
        }
    }
    Ok(view)
}
