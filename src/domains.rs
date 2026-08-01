//! Registrar-agnostic domain tooling: DNS-over-HTTPS lookups, RDAP
//! registration data (registrar, expiry — covers Squarespace-registered
//! domains, which have no public DNS API), nameserver-host classification,
//! and TLS certificate expiry checks.
//!
//! Org note: Cloudflare proxies the File Tunnel websites; Squarespace typically
//! holds domain REGISTRATION while DNS is served by Cloudflare. The public
//! marketing/docs site is GitHub Pages at file-tunnel.github.io.

use std::time::Duration;

use serde_json::Value;

use crate::util;

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("ftnl-mcp-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Query Cloudflare's public DNS-over-HTTPS resolver (JSON API).
pub async fn doh_query(domain: &str, rtype: &str) -> Result<Value, String> {
    util::safe_hostname(domain)?;
    util::safe_record_type(rtype)?;
    let client = http_client()?;
    let resp = client
        .get("https://cloudflare-dns.com/dns-query")
        .query(&[("name", domain), ("type", rtype)])
        .header("accept", "application/dns-json")
        .send()
        .await
        .map_err(|e| format!("DoH request failed: {e}"))?;
    util::read_json_capped(resp)
        .await
        .map_err(|e| format!("DoH response: {e}"))
}

/// Render a DoH JSON answer as "name TTL data" lines.
pub fn format_doh(rtype: &str, v: &Value) -> String {
    match v.get("Answer").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list
            .iter()
            .map(|a| {
                format!(
                    "  {}  ttl={}  {}",
                    a.get("name").and_then(Value::as_str).unwrap_or("?"),
                    a.get("TTL").and_then(Value::as_u64).unwrap_or(0),
                    a.get("data").and_then(Value::as_str).unwrap_or("?"),
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => format!("  (no {rtype} records)"),
    }
}

/// Fetch registration data via the rdap.org bootstrap. Works for any
/// registrar, incl. Squarespace.
pub async fn rdap_lookup(domain: &str) -> Result<Value, String> {
    util::safe_hostname(domain)?;
    let client = http_client()?;
    let resp = client
        .get(format!("https://rdap.org/domain/{domain}"))
        .header("accept", "application/rdap+json")
        .send()
        .await
        .map_err(|e| format!("RDAP request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "RDAP lookup for {domain} returned {}",
            resp.status()
        ));
    }
    util::read_json_capped(resp)
        .await
        .map_err(|e| format!("RDAP response: {e}"))
}

/// Summarize an RDAP domain object: registrar, key events, status, nameservers.
pub fn summarize_rdap(v: &Value) -> String {
    let mut out = String::new();
    if let Some(name) = v.get("ldhName").and_then(Value::as_str) {
        out.push_str(&format!("domain: {}\n", name.to_lowercase()));
    }
    if let Some(registrar) = rdap_registrar(v) {
        out.push_str(&format!("registrar: {registrar}\n"));
    }
    for ev in v
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let action = ev.get("eventAction").and_then(Value::as_str).unwrap_or("?");
        let date = ev.get("eventDate").and_then(Value::as_str).unwrap_or("?");
        let mut line = format!("{action}: {date}");
        if action == "expiration" {
            if let Some(days) = days_until_rfc3339(date) {
                line.push_str(&format!("  ({days} days from now)"));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    if let Some(status) = v.get("status").and_then(Value::as_array) {
        let s: Vec<&str> = status.iter().filter_map(Value::as_str).collect();
        if !s.is_empty() {
            out.push_str(&format!("status: {}\n", s.join(", ")));
        }
    }
    let ns = rdap_nameservers(v);
    if !ns.is_empty() {
        out.push_str(&format!(
            "nameservers: {} → DNS host looks like: {}\n",
            ns.join(", "),
            classify_dns_host(&ns)
        ));
    }
    out
}

/// Pull the registrar's display name out of RDAP entities (vCard "fn").
pub fn rdap_registrar(v: &Value) -> Option<String> {
    for ent in v.get("entities")?.as_array()? {
        let is_registrar = ent
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|r| r.iter().any(|x| x.as_str() == Some("registrar")));
        if !is_registrar {
            continue;
        }
        let props = ent.get("vcardArray")?.as_array()?.get(1)?.as_array()?;
        for p in props {
            let p = p.as_array()?;
            if p.first()?.as_str() == Some("fn") {
                return p.get(3)?.as_str().map(String::from);
            }
        }
    }
    None
}

pub fn rdap_nameservers(v: &Value) -> Vec<String> {
    v.get("nameservers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| n.get("ldhName").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect()
}

/// Best-effort classification of who serves DNS, from nameserver names.
pub fn classify_dns_host(nameservers: &[String]) -> &'static str {
    let joined = nameservers.join(" ").to_lowercase();
    if joined.contains("cloudflare") {
        "Cloudflare"
    } else if joined.contains("squarespacedns") || joined.contains("googledomains") {
        "Squarespace (incl. ex-Google Domains)"
    } else if joined.contains("awsdns") {
        "AWS Route 53"
    } else if joined.contains("github") {
        "GitHub Pages"
    } else {
        "other/unknown"
    }
}

pub fn days_until_rfc3339(date: &str) -> Option<i64> {
    let t = chrono::DateTime::parse_from_rfc3339(date).ok()?;
    Some((t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days())
}

/// Check the TLS certificate served at host:port — subject, issuer, expiry.
pub async fn tls_cert_check(host: &str, port: u16) -> Result<String, String> {
    util::safe_hostname(host)?;
    // host/port are validated above; openssl s_client needs a pipeline.
    let script = format!(
        "echo | openssl s_client -connect {host}:{port} -servername {host} 2>/dev/null \
         | openssl x509 -noout -subject -issuer -enddate 2>&1"
    );
    let (ok, text) = util::run_cmd(None, "bash", &["-c", &script], Duration::from_secs(20)).await?;
    if !ok || !text.contains("notAfter=") {
        return Err(format!(
            "could not read a certificate from {host}:{port}\n{text}"
        ));
    }
    let mut out = format!("TLS certificate at {host}:{port}\n{}", text.trim());
    if let Some(end) = text
        .lines()
        .find(|l| l.starts_with("notAfter="))
        .and_then(util::parse_openssl_enddate)
    {
        let days = (end.and_utc() - chrono::Utc::now()).num_days();
        out.push_str(&format!(
            "\ndays until expiry: {days}{}",
            if days < 14 { "  ⚠ RENEW SOON" } else { "" }
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_rdap() -> Value {
        json!({
            "ldhName": "FILE-TUNNEL.APP",
            "status": ["client transfer prohibited"],
            "events": [
                {"eventAction": "registration", "eventDate": "2024-01-02T03:04:05Z"},
                {"eventAction": "expiration", "eventDate": "2999-01-02T03:04:05Z"}
            ],
            "entities": [{
                "roles": ["registrar"],
                "vcardArray": ["vcard", [
                    ["version", {}, "text", "4.0"],
                    ["fn", {}, "text", "Squarespace Domains II LLC"]
                ]]
            }],
            "nameservers": [
                {"ldhName": "ADA.NS.CLOUDFLARE.COM"},
                {"ldhName": "BOB.NS.CLOUDFLARE.COM"}
            ]
        })
    }

    #[test]
    fn rdap_summary_extracts_registrar_expiry_and_host() {
        let v = sample_rdap();
        assert_eq!(
            rdap_registrar(&v).as_deref(),
            Some("Squarespace Domains II LLC")
        );
        let s = summarize_rdap(&v);
        assert!(s.contains("domain: file-tunnel.app"));
        assert!(s.contains("registrar: Squarespace Domains II LLC"));
        assert!(s.contains("expiration: 2999-01-02T03:04:05Z"));
        assert!(s.contains("days from now"));
        assert!(s.contains("Cloudflare"));
    }

    #[test]
    fn classify_dns_host_variants() {
        assert_eq!(
            classify_dns_host(&["ada.ns.cloudflare.com".into()]),
            "Cloudflare"
        );
        assert!(classify_dns_host(&["dns1.squarespacedns.com".into()]).starts_with("Squarespace"));
        assert_eq!(
            classify_dns_host(&["ns-123.awsdns-45.org".into()]),
            "AWS Route 53"
        );
        assert_eq!(
            classify_dns_host(&["ns1.example.net".into()]),
            "other/unknown"
        );
    }

    #[test]
    fn format_doh_renders_answers_and_empty() {
        let v = json!({"Answer": [
            {"name": "file-tunnel.github.io", "TTL": 300, "type": 5, "data": "file-tunnel.github.io."}
        ]});
        let s = format_doh("CNAME", &v);
        assert!(s.contains("file-tunnel.github.io"));
        assert!(s.contains("ttl=300"));
        assert_eq!(format_doh("AAAA", &json!({})), "  (no AAAA records)");
    }

    #[test]
    fn days_until_rfc3339_parses() {
        assert!(days_until_rfc3339("2999-01-01T00:00:00Z").unwrap() > 300_000);
        assert!(days_until_rfc3339("not-a-date").is_none());
    }
}
