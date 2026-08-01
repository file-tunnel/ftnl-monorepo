//! Read-only Supabase (PostgREST) tools over the org's client-telemetry tables
//! `ftnl_client_log_snapshots` / `ftnl_client_log_entries`.
//!
//! Every File Tunnel client — the WASM/TypeScript upload portal and native UI
//! components (`supabase-js`) and the Dart/Flutter UI (`supabase_flutter`) —
//! streams a bounded, redacted log ring buffer straight into Supabase via the
//! `ingest_ftnl_client_log_snapshot` / `ingest_ftnl_client_log_entries` RPCs
//! (see `supabase/schema.sql`). Pairing secrets, capabilities, event tickets,
//! presigned URLs, and file bytes are NEVER part of that telemetry. These tools
//! read the data back for debugging. Auth is the `SUPABASE_SERVICE_ROLE_KEY`
//! (bypasses RLS) against `SUPABASE_URL`; both must be set. Responses are
//! bounded/truncated.

use serde_json::Value;

use crate::util;

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("ftnl-mcp-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

pub const SNAPSHOTS_TABLE: &str = "ftnl_client_log_snapshots";
pub const ENTRIES_TABLE: &str = "ftnl_client_log_entries";

/// (base_url, service_role_key) from the environment, with an actionable error.
pub fn config() -> Result<(String, String), String> {
    let url = std::env::var("SUPABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(
            "SUPABASE_URL is not set. Point it at the org's Supabase project \
             (https://<ref>.supabase.co) that receives client telemetry.",
        )?;
    let key = std::env::var("SUPABASE_SERVICE_ROLE_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(
            "SUPABASE_SERVICE_ROLE_KEY is not set. Use the project's service-role key \
             (Project Settings → API); it bypasses RLS for read-only debugging. Never log it.",
        )?;
    Ok((url.trim_end_matches('/').to_string(), key))
}

/// GET a PostgREST query and return the decoded JSON array.
async fn rest_get(path_and_query: &str) -> Result<Vec<Value>, String> {
    let (base, key) = config()?;
    let client = http_client()?;
    let url = format!("{base}/rest/v1/{path_and_query}");
    let resp = client
        .get(&url)
        .header("apikey", &key)
        .bearer_auth(&key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Supabase request failed: {e}"))?;
    let status = resp.status();
    let body = util::read_text_capped(resp).await?;
    if !status.is_success() {
        let snippet: String = body.chars().take(300).collect();
        return Err(format!(
            "Supabase PostgREST error (HTTP {status}): {snippet}"
        ));
    }
    match serde_json::from_str::<Value>(&body) {
        Ok(Value::Array(a)) => Ok(a),
        Ok(other) => Err(format!(
            "expected a JSON array from PostgREST, got: {other}"
        )),
        Err(e) => Err(format!("Supabase response was not JSON: {e}")),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// Recent client-log snapshots, optionally filtered by environment.
pub async fn sessions(environment: Option<&str>, limit: u32) -> Result<String, String> {
    let limit = limit.clamp(1, 200);
    let mut q = format!(
        "{SNAPSHOTS_TABLE}?select=id,session_id,environment,commit_id,log_entries_count,snapshot_taken_at&order=snapshot_taken_at.desc&limit={limit}"
    );
    if let Some(env) = environment {
        util::safe_segment(env, "environment")?;
        q.push_str(&format!("&environment=eq.{env}"));
    }
    let rows = rest_get(&q).await?;
    if rows.is_empty() {
        return Ok("(no client-log snapshots match)".to_string());
    }
    let mut out = format!("recent client-log sessions ({}):\n", rows.len());
    for r in &rows {
        out.push_str(&format!(
            "  {}  session={}  env={}  commit={}  entries={}  at={}\n",
            s(r, "id"),
            s(r, "session_id"),
            s(r, "environment"),
            s(r, "commit_id"),
            r.get("log_entries_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            s(r, "snapshot_taken_at"),
        ));
    }
    Ok(util::truncate_output(out))
}

/// Individual log entries for one session, optionally filtered to a level.
pub async fn tail(session_id: &str, level: Option<&str>, limit: u32) -> Result<String, String> {
    util::safe_opaque_id(session_id, "session_id")?;
    let limit = limit.clamp(1, 500);
    let mut q = format!(
        "{ENTRIES_TABLE}?select=client_timestamp,level,message,category,url&session_id=eq.{session_id}&order=client_timestamp.desc&limit={limit}"
    );
    if let Some(lvl) = level {
        util::safe_segment(lvl, "level")?;
        q.push_str(&format!("&level=eq.{lvl}"));
    }
    let rows = rest_get(&q).await?;
    if rows.is_empty() {
        return Ok(format!("(no entries for session {session_id})"));
    }
    let mut out = format!(
        "entries for session {session_id} ({}, newest first):\n",
        rows.len()
    );
    for r in &rows {
        let msg: String = s(r, "message").chars().take(240).collect();
        out.push_str(&format!(
            "  {}  [{}] {}{}\n",
            s(r, "client_timestamp"),
            s(r, "level"),
            msg,
            match s(r, "category") {
                "" => String::new(),
                c => format!("  ({c})"),
            }
        ));
    }
    Ok(util::truncate_output(out))
}

/// How `client_error_summary` aggregates the recent error/warn entries.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Message,
    Url,
    Category,
}

impl GroupBy {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "message" | "msg" => Ok(GroupBy::Message),
            "url" | "route" => Ok(GroupBy::Url),
            "category" | "cat" => Ok(GroupBy::Category),
            other => Err(format!(
                "invalid group_by {other:?} — use message | url | category"
            )),
        }
    }
    fn field(self) -> &'static str {
        match self {
            GroupBy::Message => "message",
            GroupBy::Url => "url",
            GroupBy::Category => "category",
        }
    }
    fn label(self) -> &'static str {
        match self {
            GroupBy::Message => "message",
            GroupBy::Url => "url/route",
            GroupBy::Category => "category",
        }
    }
}

/// Recent error/warn entries grouped (by message / url / category), most
/// frequent first — a fast triage of what's breaking in the clients.
pub async fn error_summary(
    environment: Option<&str>,
    group_by: GroupBy,
    scan_limit: u32,
) -> Result<String, String> {
    let scan_limit = scan_limit.clamp(1, 1000);
    let mut q = format!(
        "{ENTRIES_TABLE}?select=level,message,url,category,environment&level=in.(error,warn)&order=client_timestamp.desc&limit={scan_limit}"
    );
    if let Some(env) = environment {
        util::safe_segment(env, "environment")?;
        q.push_str(&format!("&environment=eq.{env}"));
    }
    let rows = rest_get(&q).await?;
    Ok(util::truncate_output(summarize_errors(&rows, group_by)))
}

/// Pure: group error/warn rows by (level, chosen field) and rank by count.
pub fn summarize_errors(rows: &[Value], group_by: GroupBy) -> String {
    use std::collections::HashMap;
    if rows.is_empty() {
        return "(no error/warn entries match)".to_string();
    }
    let mut counts: HashMap<(String, String), u32> = HashMap::new();
    for r in rows {
        let level = s(r, "level").to_string();
        let raw = s(r, group_by.field());
        let key = raw.lines().next().unwrap_or("").trim();
        let key: String = if key.is_empty() {
            "(none)".to_string()
        } else {
            key.chars().take(160).collect()
        };
        *counts.entry((level, key)).or_insert(0) += 1;
    }
    let mut ranked: Vec<((String, String), u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out = format!(
        "error/warn summary by {} over {} recent entries ({} distinct):\n",
        group_by.label(),
        rows.len(),
        ranked.len()
    );
    for ((level, key), n) in ranked.iter().take(40) {
        out.push_str(&format!("  {n:5}×  [{level}]  {key}\n"));
    }
    out
}

/// Full ordered timeline (oldest → newest) of one session's entries — the
/// `client_log_trace`. Bounded, read-only.
pub async fn trace(session_id: &str, limit: u32) -> Result<String, String> {
    util::safe_opaque_id(session_id, "session_id")?;
    let limit = limit.clamp(1, 1000);
    let q = format!(
        "{ENTRIES_TABLE}?select=client_timestamp,level,message,category,url&session_id=eq.{session_id}&order=client_timestamp.asc&limit={limit}"
    );
    let rows = rest_get(&q).await?;
    if rows.is_empty() {
        return Ok(format!("(no entries for session {session_id})"));
    }
    let mut out = format!(
        "trace for session {session_id} ({} entries, oldest → newest):\n",
        rows.len()
    );
    for (i, r) in rows.iter().enumerate() {
        let msg: String = s(r, "message").chars().take(240).collect();
        let route = match s(r, "url") {
            "" => String::new(),
            u => format!("  @{u}"),
        };
        out.push_str(&format!(
            "  {:>4}. {}  [{}] {}{}{}\n",
            i + 1,
            s(r, "client_timestamp"),
            s(r, "level"),
            msg,
            match s(r, "category") {
                "" => String::new(),
                c => format!("  ({c})"),
            },
            route,
        ));
    }
    Ok(util::truncate_output(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_errors_are_actionable_when_unset() {
        if std::env::var("SUPABASE_URL").is_err() {
            let e = config().unwrap_err();
            assert!(e.contains("SUPABASE_URL"));
        }
    }

    #[test]
    fn summarize_errors_ranks_by_frequency() {
        let rows = vec![
            json!({"level": "error", "message": "boom\nstack line"}),
            json!({"level": "error", "message": "boom\ndifferent stack"}),
            json!({"level": "warn", "message": "slow request"}),
        ];
        let s = summarize_errors(&rows, GroupBy::Message);
        assert!(s.contains("by message"));
        assert!(s.contains("2 distinct"));
        let first_line = s.lines().nth(1).unwrap();
        assert!(first_line.contains("2×"));
        assert!(first_line.contains("[error]"));
        assert!(first_line.contains("boom"));
        assert_eq!(
            summarize_errors(&[], GroupBy::Message),
            "(no error/warn entries match)"
        );
    }

    #[test]
    fn summarize_errors_groups_by_url_and_category() {
        let rows = vec![
            json!({"level": "error", "message": "a", "url": "/t/abc", "category": "net"}),
            json!({"level": "error", "message": "b", "url": "/t/abc", "category": "ui"}),
            json!({"level": "error", "message": "c", "url": "/t/xyz", "category": "net"}),
        ];
        let by_url = summarize_errors(&rows, GroupBy::Url);
        assert!(by_url.contains("by url/route"));
        assert!(by_url.lines().nth(1).unwrap().contains("/t/abc"));
        let by_cat = summarize_errors(&rows, GroupBy::Category);
        assert!(by_cat.contains("by category"));
        assert!(by_cat.contains("net"));
        // missing field collapses to (none)
        let none = summarize_errors(&[json!({"level": "warn", "message": "x"})], GroupBy::Url);
        assert!(none.contains("(none)"));
    }

    #[test]
    fn group_by_parse_accepts_aliases_and_rejects_junk() {
        assert!(matches!(GroupBy::parse("message"), Ok(GroupBy::Message)));
        assert!(matches!(GroupBy::parse("URL"), Ok(GroupBy::Url)));
        assert!(matches!(GroupBy::parse("route"), Ok(GroupBy::Url)));
        assert!(matches!(GroupBy::parse("cat"), Ok(GroupBy::Category)));
        assert!(GroupBy::parse("user; drop table").is_err());
    }
}
