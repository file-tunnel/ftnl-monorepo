//! Read-only fiducia.cloud integration. fiducia is the org's shared
//! secrets + distributed-locks/leases plane (secrets synced with GitHub Actions
//! secrets; fiducia-clients hold leases). This tool asks a fiducia endpoint
//! whether this org's required secrets are present and whether its locks/leases
//! look healthy — never fetching secret *values*, only presence/health. Ties
//! File Tunnel into the shared secrets/locks plane.
//!
//! Env: `FIDUCIA_URL` (https://…) + `FIDUCIA_TOKEN`. Optional
//! `FIDUCIA_REQUIRED_SECRETS` = comma-separated names to assert present
//! (default: the backend/creds this org itself consumes). The token is only ever
//! sent as a bearer header — never printed.

use serde_json::Value;

use crate::domains::http_client;
use crate::util;

pub struct FiduciaEnv {
    pub url: String,
    pub token: String,
    pub required: Vec<String>,
}

pub fn env() -> Result<FiduciaEnv, String> {
    let url = std::env::var("FIDUCIA_URL").map_err(|_| missing_env())?;
    let token = std::env::var("FIDUCIA_TOKEN").map_err(|_| missing_env())?;
    if token.trim().is_empty() {
        return Err(missing_env());
    }
    let url = util::safe_base_url(&url)?;
    let required = std::env::var("FIDUCIA_REQUIRED_SECRETS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            [
                "FTNL_DATABASE_URL",
                "FTNL_OBJECT_STORAGE_URL",
                "FTNL_CAPABILITY_SIGNING_KEY",
                "SUPABASE_SERVICE_ROLE_KEY",
                "CLOUDFLARE_API_TOKEN",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        });
    Ok(FiduciaEnv {
        url,
        token,
        required,
    })
}

fn missing_env() -> String {
    "FIDUCIA_URL and/or FIDUCIA_TOKEN are not set. fiducia.cloud is the org's shared \
     secrets + locks/leases plane; export the fiducia base URL (https://…) and a \
     read-scoped token in the environment the MCP server starts in. Optionally set \
     FIDUCIA_REQUIRED_SECRETS to a comma-separated list of secret names to assert \
     (default: the backend's FTNL_DATABASE_URL / FTNL_OBJECT_STORAGE_URL / \
     FTNL_CAPABILITY_SIGNING_KEY plus SUPABASE_SERVICE_ROLE_KEY / CLOUDFLARE_API_TOKEN). \
     The token is sent only as a bearer header and is never printed."
        .to_string()
}

/// GET a fiducia JSON endpoint with the bearer token and a 10s timeout.
async fn get(env: &FiduciaEnv, path: &str) -> Result<(reqwest::StatusCode, Value), String> {
    let client = http_client()?;
    let resp = client
        .get(format!("{}{}", env.url, path))
        .bearer_auth(&env.token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("fiducia request to {path} failed: {e}"))?;
    let status = resp.status();
    // Bounded read; fiducia tolerates a non-JSON/empty body as Null.
    let body: Value = util::read_json_capped(resp).await.unwrap_or(Value::Null);
    Ok((status, body))
}

/// Check secret presence + lock/lease health. Bounded, read-only.
pub async fn status_report(env: &FiduciaEnv) -> Result<String, String> {
    let mut out = format!("fiducia status at {}\n\n", env.url);

    // 1) health
    let (h_status, h_body) = get(env, "/health").await?;
    out.push_str(&format!("## /health — HTTP {h_status}\n"));
    if let Some(s) = h_body.get("status").and_then(Value::as_str) {
        out.push_str(&format!("  status: {s}\n"));
    } else if h_body.is_null() {
        out.push_str("  (no JSON body)\n");
    }

    // 2) required-secret presence (names + presence only; never values)
    let (s_status, s_body) = get(env, "/v1/secrets").await?;
    out.push_str(&format!(
        "\n## required secrets ({} checked)\n",
        env.required.len()
    ));
    let present_names = secret_names(&s_body);
    if s_status.is_success() {
        for name in &env.required {
            let ok = present_names.iter().any(|n| n == name);
            out.push_str(&format!(
                "  {name}: {}\n",
                if ok { "present" } else { "MISSING" }
            ));
        }
    } else {
        out.push_str(&format!(
            "  could not list secrets (HTTP {s_status}); the token may lack read scope\n"
        ));
    }

    // 3) lock / lease health
    let (l_status, l_body) = get(env, "/v1/leases").await?;
    out.push_str(&format!("\n## locks/leases — HTTP {l_status}\n"));
    out.push_str(&summarize_leases(&l_body));
    Ok(util::truncate_output(out))
}

/// Extract secret *names* from a fiducia secrets listing (values are ignored).
pub fn secret_names(body: &Value) -> Vec<String> {
    let arr = body
        .get("secrets")
        .and_then(Value::as_array)
        .or_else(|| body.as_array());
    let mut names = Vec::new();
    if let Some(list) = arr {
        for item in list {
            if let Some(n) = item.as_str() {
                names.push(n.to_string());
            } else if let Some(n) = item.get("name").and_then(Value::as_str) {
                names.push(n.to_string());
            } else if let Some(n) = item.get("key").and_then(Value::as_str) {
                names.push(n.to_string());
            }
        }
    }
    names
}

pub fn summarize_leases(body: &Value) -> String {
    let leases = body
        .get("leases")
        .and_then(Value::as_array)
        .or_else(|| body.as_array());
    match leases {
        Some(list) if !list.is_empty() => {
            let mut out = format!("  {} lease(s):\n", list.len());
            for l in list.iter().take(50) {
                let name = l
                    .get("name")
                    .or_else(|| l.get("key"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let holder = l.get("holder").and_then(Value::as_str).unwrap_or("-");
                let healthy = l
                    .get("healthy")
                    .and_then(Value::as_bool)
                    .or_else(|| l.get("expired").and_then(Value::as_bool).map(|e| !e));
                let state = match healthy {
                    Some(true) => "held",
                    Some(false) => "STALE/EXPIRED",
                    None => "?",
                };
                out.push_str(&format!("    {name}  holder={holder}  {state}\n"));
            }
            out
        }
        _ if body.is_null() => "  (no JSON body)\n".into(),
        _ => "  (no active leases reported)\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_env_is_actionable_and_leakfree() {
        let m = missing_env();
        assert!(m.contains("FIDUCIA_URL"));
        assert!(m.contains("FIDUCIA_TOKEN"));
        assert!(m.contains("never printed"));
    }

    #[test]
    fn secret_names_handles_shapes() {
        assert_eq!(
            secret_names(&json!({"secrets": ["A", "B"]})),
            vec!["A", "B"]
        );
        assert_eq!(
            secret_names(&json!([{"name": "X"}, {"key": "Y"}])),
            vec!["X", "Y"]
        );
        assert!(secret_names(&Value::Null).is_empty());
    }

    #[test]
    fn summarize_leases_flags_expired() {
        let s = summarize_leases(&json!({"leases": [
            {"name": "dpm-verify", "holder": "ci-42", "healthy": true},
            {"name": "release", "holder": "runner-9", "expired": true}
        ]}));
        assert!(s.contains("dpm-verify"));
        assert!(s.contains("held"));
        assert!(s.contains("STALE/EXPIRED"));
        assert!(summarize_leases(&json!({"leases": []})).contains("no active leases"));
    }
}
