//! Probes and the route map for the File Tunnel **backend API**
//! (`ftnl-backend-api.rs`), the control + data plane for ephemeral tunnels.
//!
//! The backend is a Rust/Axum service (default bind `127.0.0.1:8080`) that
//! creates a tunnel, redeems a one-time pairing secret for a phone-scoped
//! capability, declares/uploads/downloads files, and streams progress events
//! over ticket-authenticated WebSockets. All probes here are read-only GETs
//! against the UNAUTHENTICATED `/healthz` route only; nothing creates a tunnel,
//! claims a capability, or moves bytes. A tunnel UUID is a routing identifier,
//! not a credential — but every capability-scoped route still fails closed
//! without a bearer capability, so this server never dials them.

use crate::util;

/// Base URL of the backend API to probe. Defaults to the reference server's
/// local bind; point `FTNL_API_URL` at a reachable instance (e.g. a
/// port-forwarded in-cluster service).
pub fn api_base_url() -> String {
    std::env::var("FTNL_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

/// The backend's documented HTTP surface. `(method, path, needs_capability, note)`.
/// Only the unauthenticated `/healthz` probe is actually dialed by
/// `tunnel_api_routes`; the capability-scoped routes are listed for reference
/// because they fail closed without a desktop/phone bearer capability (or, for
/// the event stream, a one-time event ticket).
pub const ROUTES: &[(&str, &str, bool, &str)] = &[
    (
        "GET",
        "/healthz",
        false,
        "service health (k8s probe, unauthenticated)",
    ),
    (
        "POST",
        "/v1/tunnels",
        true,
        "create an ephemeral tunnel (desktop application capability)",
    ),
    (
        "GET",
        "/v1/tunnels/{id}",
        true,
        "point-in-time tunnel snapshot (capability-scoped)",
    ),
    (
        "DELETE",
        "/v1/tunnels/{id}",
        true,
        "cancel a tunnel + schedule content deletion",
    ),
    (
        "POST",
        "/v1/tunnels/{id}/claim",
        true,
        "redeem the one-time pairing secret for a phone capability",
    ),
    (
        "POST",
        "/v1/tunnels/{id}/files",
        true,
        "declare upload metadata before sending bytes (phone)",
    ),
    (
        "PUT",
        "/v1/tunnels/{id}/files/{fid}/content",
        true,
        "upload file bytes (phone capability)",
    ),
    (
        "GET",
        "/v1/tunnels/{id}/files/{fid}/content",
        true,
        "download completed bytes (desktop capability)",
    ),
    (
        "POST",
        "/v1/tunnels/{id}/event-tickets",
        true,
        "mint a one-time, short-lived WebSocket event ticket",
    ),
    (
        "GET",
        "/v1/tunnels/{id}/events",
        true,
        "upgrade to a WebSocket using a one-time event ticket",
    ),
];

/// GET a path on the backend and return `(status, body)`.
async fn probe(base: &str, path: &str) -> Result<(u16, String), String> {
    let base =
        util::safe_base_url(base).map_err(|e| format!("invalid FTNL_API_URL/base_url: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("ftnl-mcp-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("building backend probe client: {e}"))?;
    let url = format!("{base}{path}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("backend GET failed: {e}"))?;
    let status = resp.status().as_u16();
    // The backend is network-influenced (arbitrary base_url); bound the body in
    // size and time so a hostile/buggy response can't OOM or hang the server.
    let body = util::read_text_capped(resp).await?;
    Ok((status, body))
}

/// Probe `/healthz` and summarize the result.
pub async fn health(base: &str) -> Result<String, String> {
    let base =
        util::safe_base_url(base).map_err(|e| format!("invalid FTNL_API_URL/base_url: {e}"))?;
    let (status, body) = probe(&base, "/healthz").await?;
    let short: String = body.chars().take(400).collect();
    Ok(format!(
        "GET {base}/healthz → HTTP {status}\n{}",
        if short.trim().is_empty() {
            "(empty body)".to_string()
        } else {
            short
        }
    ))
}

/// Probe the unauthenticated route(s) and list the capability-scoped surface.
pub async fn routes(base: &str) -> Result<String, String> {
    let base =
        util::safe_base_url(base).map_err(|e| format!("invalid FTNL_API_URL/base_url: {e}"))?;
    let mut out =
        format!("File Tunnel backend API at {base}\n\n## live probes (unauthenticated routes)\n");
    for (method, path, needs_cap, note) in ROUTES {
        if *needs_cap {
            continue;
        }
        match probe(&base, path).await {
            Ok((status, _)) => out.push_str(&format!(
                "  {method:4} {path:12} → HTTP {status}  ({note})\n"
            )),
            Err(e) => out.push_str(&format!("  {method:4} {path:12} → ERROR {e}\n")),
        }
    }
    out.push_str(
        "\n## capability-scoped routes (fail closed without a desktop/phone capability \
         or a one-time event ticket — never dialed here)\n",
    );
    for (method, path, needs_cap, note) in ROUTES {
        if *needs_cap {
            out.push_str(&format!("  {method:6} {path:40} {note}\n"));
        }
    }
    out.push_str(
        "\nnote: a tunnel UUID is a routing identifier, not a credential; pairing secrets \
         live in the URL fragment (#c=…) and are exchanged once for a scoped capability.\n",
    );
    Ok(util::truncate_output(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_url_is_the_backend_bind() {
        // Only assert the default when the override is absent.
        if std::env::var("FTNL_API_URL").is_err() {
            assert_eq!(api_base_url(), "http://127.0.0.1:8080");
        }
    }

    #[test]
    fn routes_table_covers_health_create_claim_and_events() {
        let paths: Vec<&str> = ROUTES.iter().map(|(_, p, _, _)| *p).collect();
        assert!(paths.contains(&"/healthz"));
        assert!(paths.contains(&"/v1/tunnels"));
        assert!(paths.contains(&"/v1/tunnels/{id}/claim"));
        assert!(paths.contains(&"/v1/tunnels/{id}/events"));
        // /healthz is the only unauthenticated probe.
        let open: Vec<&str> = ROUTES
            .iter()
            .filter(|(_, _, cap, _)| !cap)
            .map(|(_, p, _, _)| *p)
            .collect();
        assert_eq!(open, vec!["/healthz"]);
    }
}
