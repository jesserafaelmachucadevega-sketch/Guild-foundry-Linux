// SPDX-License-Identifier: Apache-2.0
// cost.rs — token-usage ledger + editable pricing ("what has this cost me?").
//
// Every cloud completion/stream records (timestamp, provider, model, tokens)
// here (providers.rs calls `record`). Prices are user-editable per model, in
// per-million-token units; a model without a price is reported as *unpriced*
// rather than guessed, so the dashboard never invents a number.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::Manager;

fn open_conn(app: &tauri::AppHandle) -> Result<Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("guild-foundry-ai.db")).map_err(|e| e.to_string())?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    ensure_tables(&conn);
    Ok(conn)
}

fn ensure_tables(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS token_usage (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_token_usage_ts ON token_usage(ts);
        CREATE TABLE IF NOT EXISTS model_prices (
            model TEXT PRIMARY KEY,
            input_per_mtok REAL NOT NULL DEFAULT 0,
            output_per_mtok REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );",
    );
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Best-effort ledger write — a failed insert must never break a chat turn.
pub fn record(app: &tauri::AppHandle, provider_id: &str, model: &str, prompt: u64, completion: u64) {
    if prompt == 0 && completion == 0 {
        return;
    }
    if let Ok(conn) = open_conn(app) {
        let _ = conn.execute(
            "INSERT INTO token_usage (ts, provider_id, model, prompt_tokens, completion_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                now_iso(),
                provider_id,
                model,
                prompt as i64,
                completion as i64
            ],
        );
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub day: String,
    pub provider_id: String,
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
    pub priced: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceRow {
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

fn load_prices(conn: &Connection) -> Result<HashMap<String, (f64, f64)>, String> {
    let mut prices = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT model, input_per_mtok, output_per_mtok FROM model_prices")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (m, i, o) = row.map_err(|e| e.to_string())?;
        prices.insert(m, (i, o));
    }
    Ok(prices)
}

/// Aggregated usage for the last `days`, priced with the user's table.
#[tauri::command(rename_all = "snake_case")]
pub fn cost_summary(app: tauri::AppHandle, days: Option<u32>) -> Result<serde_json::Value, String> {
    let days = days.unwrap_or(30).clamp(1, 365);
    let conn = open_conn(&app)?;
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let prices = load_prices(&conn)?;

    let mut stmt = conn
        .prepare(
            "SELECT substr(ts, 1, 10) AS day, provider_id, model,
                    SUM(prompt_tokens) AS pt, SUM(completion_tokens) AS ct
             FROM token_usage
             WHERE ts >= ?1
             GROUP BY day, provider_id, model
             ORDER BY day DESC, (SUM(prompt_tokens) + SUM(completion_tokens)) DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![cutoff], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out: Vec<UsageRow> = Vec::new();
    let mut total_cost = 0.0f64;
    let mut total_prompt = 0i64;
    let mut total_completion = 0i64;
    let mut unpriced: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        let (day, provider_id, model, pt, ct) = row.map_err(|e| e.to_string())?;
        let (input_price, output_price) = prices.get(&model).copied().unwrap_or((0.0, 0.0));
        let priced = prices.contains_key(&model);
        if !priced {
            unpriced.insert(model.clone());
        }
        let cost = pt as f64 / 1_000_000.0 * input_price
            + ct as f64 / 1_000_000.0 * output_price;
        total_cost += cost;
        total_prompt += pt;
        total_completion += ct;
        out.push(UsageRow {
            day,
            provider_id,
            model,
            prompt_tokens: pt,
            completion_tokens: ct,
            cost,
            priced,
        });
    }

    Ok(serde_json::json!({
        "days": days,
        "rows": out,
        "totalCost": total_cost,
        "totalPrompt": total_prompt,
        "totalCompletion": total_completion,
        "unpricedModels": unpriced.into_iter().collect::<Vec<String>>(),
    }))
}

#[tauri::command]
pub fn cost_prices(app: tauri::AppHandle) -> Result<Vec<PriceRow>, String> {
    let conn = open_conn(&app)?;
    let mut stmt = conn
        .prepare("SELECT model, input_per_mtok, output_per_mtok FROM model_prices ORDER BY model")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PriceRow {
                model: r.get(0)?,
                input_per_mtok: r.get(1)?,
                output_per_mtok: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn cost_price_set(
    app: tauri::AppHandle,
    model: String,
    input_per_mtok: f64,
    output_per_mtok: f64,
) -> Result<(), String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("price needs a model name".to_string());
    }
    if !input_per_mtok.is_finite() || !output_per_mtok.is_finite()
        || input_per_mtok < 0.0 || output_per_mtok < 0.0
    {
        return Err("prices must be finite, non-negative numbers".to_string());
    }
    let conn = open_conn(&app)?;
    conn.execute(
        "INSERT INTO model_prices (model, input_per_mtok, output_per_mtok, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(model) DO UPDATE SET
            input_per_mtok = excluded.input_per_mtok,
            output_per_mtok = excluded.output_per_mtok,
            updated_at = excluded.updated_at",
        params![model, input_per_mtok, output_per_mtok, now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn cost_price_remove(app: tauri::AppHandle, model: String) -> Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute("DELETE FROM model_prices WHERE model = ?1", params![model.trim()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Wipe the usage ledger (prices are kept). The UI asks for confirmation.
#[tauri::command]
pub fn cost_usage_clear(app: tauri::AppHandle) -> Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute("DELETE FROM token_usage", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}
