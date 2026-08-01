//! MCP resources: embedded File Tunnel org knowledge a client can load directly
//! (no tool call). The org map, the architecture doc, the client-telemetry doc,
//! and the declarative `supabase/schema.sql` are all served here so an agent can
//! pull them into context before touching the backend, the portal, or the
//! telemetry tables.
//!
//! Resources are read-only and static; the bytes are compiled into the binary.

use crate::docs;

/// The declarative telemetry schema, embedded at build time.
pub const SCHEMA_SQL: &str = include_str!("../supabase/schema.sql");

/// A resource the server exposes over `resources/list` / `resources/read`.
pub struct ResourceDef {
    pub uri: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub mime: &'static str,
    pub body: &'static str,
}

/// Every resource this server offers.
pub fn all() -> Vec<ResourceDef> {
    vec![
        ResourceDef {
            uri: "orgmap://file-tunnel",
            name: "org_map",
            title: "File Tunnel org map",
            description: "Repos (GitHub name → purpose), the capability model, the tunnel \
                          lifecycle, the backend (:8080) + portal (:3000), sync boundary, \
                          and shared conventions (Nix, formal methods, dpm, Cloudflare DNS).",
            mime: "text/markdown",
            body: docs::ORG_MAP,
        },
        ResourceDef {
            uri: "docs://architecture",
            name: "architecture",
            title: "File Tunnel architecture",
            description: "The two-capability model, the HTTP control/data plane routes, the \
                          ticket-authenticated realtime event stream, and the local-first \
                          sync state machine. Read before touching the backend or portal.",
            mime: "text/markdown",
            body: docs::ARCHITECTURE,
        },
        ResourceDef {
            uri: "docs://telemetry",
            name: "telemetry_docs",
            title: "File Tunnel client telemetry → Supabase",
            description: "How every File Tunnel client streams a redacted log ring buffer \
                          straight to Supabase (ingest_ftnl_client_log_* RPCs, \
                          ftnl_client_log_* tables, RLS, realtime) vs the separate \
                          backend→stdout/OTel server plane. Secrets/bytes are never logged.",
            mime: "text/markdown",
            body: docs::TELEMETRY_DOCS,
        },
        ResourceDef {
            uri: "schema://client-telemetry",
            name: "telemetry_schema_sql",
            title: "supabase/schema.sql (dpm-style)",
            description: "Declarative Postgres schema for the client-telemetry tables, \
                          ingest RPCs, RLS policies, size guardrails, and realtime \
                          publication. Applied with dpm.",
            mime: "application/sql",
            body: SCHEMA_SQL,
        },
    ]
}

/// Look up one resource by its URI.
pub fn get(uri: &str) -> Option<ResourceDef> {
    all().into_iter().find(|r| r.uri == uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resource_has_a_unique_uri_and_nonempty_body() {
        let all = all();
        let mut uris: Vec<&str> = all.iter().map(|r| r.uri).collect();
        uris.sort();
        let before = uris.len();
        uris.dedup();
        assert_eq!(before, uris.len(), "resource URIs must be unique");
        for r in &all {
            assert!(!r.body.is_empty(), "resource {} has empty body", r.uri);
            assert!(!r.description.is_empty());
        }
    }

    #[test]
    fn schema_resource_carries_the_telemetry_ddl() {
        let r = get("schema://client-telemetry").expect("schema resource");
        assert!(r.body.contains("ftnl_client_log_snapshots"));
        assert!(r.body.contains("ingest_ftnl_client_log_entries"));
        assert!(get("orgmap://file-tunnel").unwrap().body.contains("8080"));
        assert!(get("docs://architecture")
            .unwrap()
            .body
            .contains("capability"));
        assert!(get("nope://x").is_none());
    }
}
