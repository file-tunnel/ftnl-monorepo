//! Read-only Cloudflare API v4 client (zones + DNS records). Auth comes from
//! the `CLOUDFLARE_API_TOKEN` env var — use a token scoped to Zone:Read +
//! DNS:Read so the server stays read-only even if prompted otherwise. The
//! token is only ever attached as a bearer header and never logged.

use serde_json::Value;

use crate::domains::http_client;

const API: &str = "https://api.cloudflare.com/client/v4";

pub fn token() -> Result<String, String> {
    std::env::var("CLOUDFLARE_API_TOKEN").map_err(|_| {
        "CLOUDFLARE_API_TOKEN is not set. Create a read-only token (Zone:Read, DNS:Read) \
         at dash.cloudflare.com → My Profile → API Tokens (Cloudflare account: \
         alexander.d.mills@gmail.com), then export it in the environment the MCP server \
         starts in."
            .to_string()
    })
}

pub async fn api_get(path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let token = token()?;
    let client = http_client()?;
    let resp = client
        .get(format!("{API}{path}"))
        .bearer_auth(token)
        .query(query)
        .send()
        .await
        .map_err(|e| format!("cloudflare request failed: {e}"))?;
    let status = resp.status();
    let body: Value = crate::util::read_json_capped(resp)
        .await
        .map_err(|e| format!("cloudflare response: {e}"))?;
    if !status.is_success() || body.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "cloudflare API error (HTTP {status}): {}",
            serde_json::to_string(&body.get("errors").cloned().unwrap_or(Value::Null))
                .unwrap_or_default()
        ));
    }
    Ok(body)
}

pub async fn zone_id_by_name(name: &str) -> Result<String, String> {
    let body = api_get("/zones", &[("name", name), ("per_page", "5")]).await?;
    body.get("result")
        .and_then(Value::as_array)
        .and_then(|zs| zs.first())
        .and_then(|z| z.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("no Cloudflare zone named {name:?} visible to this token"))
}

pub fn format_zones(body: &Value) -> String {
    match body.get("result").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list
            .iter()
            .map(|z| {
                format!(
                    "  {}  status={}  plan={}  id={}",
                    z.get("name").and_then(Value::as_str).unwrap_or("?"),
                    z.get("status").and_then(Value::as_str).unwrap_or("?"),
                    z.pointer("/plan/name")
                        .and_then(Value::as_str)
                        .unwrap_or("?"),
                    z.get("id").and_then(Value::as_str).unwrap_or("?"),
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "  (no zones visible to this token)".to_string(),
    }
}

pub fn format_records(body: &Value) -> String {
    match body.get("result").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list
            .iter()
            .map(|r| {
                let proxied = match r.get("proxied").and_then(Value::as_bool) {
                    Some(true) => "  [proxied]",
                    _ => "",
                };
                format!(
                    "  {:6} {}  →  {}  ttl={}{}",
                    r.get("type").and_then(Value::as_str).unwrap_or("?"),
                    r.get("name").and_then(Value::as_str).unwrap_or("?"),
                    r.get("content").and_then(Value::as_str).unwrap_or("?"),
                    r.get("ttl").and_then(Value::as_u64).unwrap_or(0),
                    proxied,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "  (no matching DNS records)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_zones_renders_fields() {
        let body = json!({"result": [
            {"name": "file-tunnel.app", "status": "active", "id": "abc123",
             "plan": {"name": "Free"}}
        ]});
        let s = format_zones(&body);
        assert!(s.contains("file-tunnel.app"));
        assert!(s.contains("status=active"));
        assert!(s.contains("plan=Free"));
        assert!(format_zones(&json!({"result": []})).contains("no zones"));
    }

    #[test]
    fn format_records_renders_proxied_flag() {
        let body = json!({"result": [
            {"type": "A", "name": "portal.file-tunnel.app", "content": "1.2.3.4", "ttl": 1, "proxied": true},
            {"type": "TXT", "name": "file-tunnel.app", "content": "v=spf1 -all", "ttl": 300, "proxied": false}
        ]});
        let s = format_records(&body);
        assert!(s.contains("portal.file-tunnel.app"));
        assert!(s.contains("[proxied]"));
        assert!(!s.lines().nth(1).unwrap().contains("[proxied]"));
    }

    #[test]
    fn token_error_is_actionable_when_unset() {
        if std::env::var("CLOUDFLARE_API_TOKEN").is_err() {
            let err = token().unwrap_err();
            assert!(err.contains("CLOUDFLARE_API_TOKEN"));
            assert!(err.contains("Zone:Read"));
        }
    }
}
