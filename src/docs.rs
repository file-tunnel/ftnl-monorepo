//! Embedded, offline reference docs served by the `org_map`,
//! `architecture_docs`, and `telemetry_docs` tools (and as MCP resources) so
//! agents can orient without cloning everything. Update these when repos,
//! ports, or the deployment shape change (last sync 2026-07).

pub const ORG_MAP: &str = r#"# File Tunnel — org / architecture map

File Tunnel is a secure, ephemeral, QR-connected cross-device file transfer
platform: it bridges a desktop upload field to files that live on a phone.
"Scan once. Pick the files on your phone. Watch them arrive in the app already
open on your computer." Site: https://file-tunnel.github.io

## Repos (github.com/file-tunnel)

- ftnl-backend-api.rs — Rust/Axum control + data-plane API (default bind
  127.0.0.1:8080). Create tunnel, QR pairing, one-time claim, file declaration,
  streaming upload progress, download, cancellation, expiry metadata, snapshots,
  and ticket-authenticated WebSockets. The reference server stores metadata +
  bytes in memory; production plugs the same contract into Redis/Postgres for
  metadata and S3/R2/GCS multipart object storage. SOURCE OF TRUTH for the API.
- ftnl-web-server.rs — Rust mobile upload portal / install-free PWA (default
  127.0.0.1:3000, backend at FTNL_API_ORIGIN, default http://127.0.0.1:8080).
  No account, cookies, analytics, or third-party scripts; reads the pairing
  secret from the URL fragment (#c=…), strips it, and exchanges it for a
  phone-scoped capability held only in sessionStorage. Restrictive CSP +
  Referrer-Policy: no-referrer; file bytes are never cached.
- ftnl-interfaces — canonical, versioned contracts shared by every service,
  portal, UI component, and SDK: openapi/ftnl.openapi.yaml (HTTP control/data
  plane), asyncapi/ftnl.asyncapi.yaml (realtime events), schema/*.schema.json,
  fixtures/ (cross-language vectors), generated/ (Rust/TS/Dart/Gleam snapshots).
- ftnl-sync — local-first upload intent + resumability on a pinned revision of
  opto-sync-clients (git submodule). Persists job metadata + checkpoints in
  IndexedDB (web) / SQLite (native). Explicitly EXCLUDES file bytes, pairing
  secrets, capabilities, event tickets, and presigned URLs from sync envelopes.
- ftnl-clients — official Rust, TypeScript, Dart, and Gleam clients.
- ftnl-ui-components — native iOS, Android, Flutter, and Web UI components.
- ftnl-infra — Argo CD app-of-apps + Kubernetes GitOps manifests.
- ftnl-e2e — Playwright cross-browser tests for the full system.
- ftnl-monorepo — pinned integration superproject; pins every deployable repo as
  a git submodule under apps/ (apps/backend-api, apps/web-server, apps/interfaces,
  apps/sync, apps/clients, apps/ui-components, apps/infra, apps/e2e, apps/site,
  apps/mcp-server). Owns docker-compose + the Nix flake for the whole stack.
- file-tunnel.github.io — public Astro marketing + documentation site (Pages).
- ftnl-mcp-server.rs — this MCP server (Rust, rmcp stdio).

## The capability model (why a UUID grants nothing)

A tunnel UUID is only an ADDRESS. It grants no access.
- The QR encodes https://portal/t/{uuid}#c={secret}; URL fragments are never
  sent in HTTP requests, access logs, or referrers.
- The one-time pairing secret is exchanged once for a phone-scoped bearer
  capability and immediately invalidated.
- Desktop and phone capabilities are separate and stored only as SHA-256 digests.
- Browser WebSockets use one-time event tickets because the browser API cannot
  attach an Authorization header.
- Filenames remain metadata and are never used as storage paths.

## Core lifecycle

1. A desktop creates a tunnel and renders pairing_uri as a QR code.
2. A phone opens the portal and exchanges the fragment secret once.
3. The phone declares a file, uploads it, and emits progress transitions.
4. The desktop receives the same transitions over the event channel.
5. Completion, cancellation, expiry, or explicit deletion closes the tunnel.

## Conventions (shared org infra)

- Reproducible toolchain: a checked-in Nix flake (flake.lock) pins Rust, Node,
  Dart, Gleam, Erlang, and linters; `nix develop --command agent-check` is the
  standard validation entrypoint per repo.
- Formal methods: randomized Rust transition properties, Kani proofs over the
  capability policy, and TLC/Quint model checking guard the security invariants
  (see each repo's formal/ or docs/formal-methods.md).
- Migrations: dpm (github.com/declarative-migrations) — declarative, stateless,
  ORM-agnostic Postgres schema migration; no tracked migration files.
- Web/DNS: Cloudflare proxies the org sites (account alexander.d.mills@gmail.com);
  marketing on GitHub Pages. Squarespace typically holds REGISTRATION (no public
  DNS API → use RDAP + DoH). Secrets/locks: fiducia.cloud.
- This MCP server is stdio-only (no HTTP) and read-only / build-only.
"#;

pub const ARCHITECTURE: &str = r#"# File Tunnel architecture

## Two capabilities, one tunnel
The API is capability-based, not account-based. A DESKTOP capability can observe
and download from one tunnel; a PHONE capability can declare and upload files to
that tunnel. Pairing secrets are one-time credentials that live in the URL
fragment (#c=…), which browsers do not send in HTTP requests. Possession of a
tunnel UUID alone grants no access.

## HTTP control + data plane (ftnl-backend-api.rs, :8080)
  GET    /healthz                                   service health (unauthenticated)
  POST   /v1/tunnels                                create an ephemeral tunnel (desktop app)
  GET    /v1/tunnels/{id}                           point-in-time snapshot (capability)
  DELETE /v1/tunnels/{id}                           cancel + schedule content deletion
  POST   /v1/tunnels/{id}/claim                     redeem pairing secret → phone capability
  POST   /v1/tunnels/{id}/files                     declare upload metadata (phone)
  PUT    /v1/tunnels/{id}/files/{fid}/content       upload bytes (phone)
  GET    /v1/tunnels/{id}/files/{fid}/content       download completed bytes (desktop)
  POST   /v1/tunnels/{id}/event-tickets             mint a one-time WebSocket ticket
  GET    /v1/tunnels/{id}/events                    upgrade to a WebSocket via a ticket
Only /healthz is unauthenticated; every /v1 route fails closed without a bearer
capability (or, for /events, a one-time event ticket). Use the
`transfer_api_health` and `tunnel_api_routes` tools to probe.

## Realtime events
Desktop and phone see the SAME progress transitions. Because a browser
WebSocket cannot send an Authorization header, a client first mints a one-time,
short-lived event ticket over HTTP, then upgrades /events with it. Event kinds
follow ftnl-interfaces' AsyncAPI; clients must ignore unknown kinds (v1 additive).

## Local-first sync (ftnl-sync)
Upload intent and resumability are optimistic and offline-durable, delegated to a
pinned opto-sync-clients core (IndexedDB on web, SQLite on native). The synced
record carries only stable job/tunnel/file IDs, display-safe name/type/byte
count, lifecycle state + byte checkpoint, attempt count + redacted reason, and
hybrid logical timestamps for deterministic merge. File bytes, capabilities,
secrets, tickets, and presigned URLs are NEVER synced. State machine:
  queued → declaring → uploading ⇄ paused → available → imported
      └────────────→ failed ───────┘   └─────────────→ cancelled
Transitions are monotonic except a retry from failed/paused; a server snapshot
always becomes the new base, then unconfirmed local mutations are re-applied.

## Portal (ftnl-web-server.rs, :3000)
An install-free page opened by scanning the desktop QR. It reads #c=… once,
removes it from the visible URL, exchanges it for a phone capability kept in
sessionStorage, and uploads via XHR (reliable byte progress on mobile).
Restrictive CSP / Permissions-Policy / Referrer-Policy: no-referrer; portal HTML
is no-store and file bytes are never cached by the server or a service worker.
"#;

pub const TELEMETRY_DOCS: &str = r#"# File Tunnel client telemetry → Supabase

Two distinct telemetry planes — do not conflate them:

## 1. CLIENT telemetry → Supabase (what this server's client_log_* tools read)
Every File Tunnel client keeps a bounded, redacted ring buffer of log levels
plus device info and streams it DIRECTLY to Supabase — no app-server hop:
  - the WASM/TypeScript upload portal + web UI components use `supabase-js`;
  - the Dart/Flutter UI uses `supabase_flutter`.
Both call SECURITY DEFINER ingest RPCs (see supabase/schema.sql):
  - supabase.rpc('ingest_ftnl_client_log_snapshot', { payload })
  - supabase.rpc('ingest_ftnl_client_log_entries',  { entries })
Data lands in RLS-protected tables `ftnl_client_log_snapshots` and
`ftnl_client_log_entries`, which are added to the `supabase_realtime`
publication so dashboards can subscribe live. anon/authenticated roles get
INSERT-only via the RPCs; reads use the service-role key.
CRITICAL: pairing secrets, desktop/phone capabilities, event tickets, presigned
URLs, and file bytes are NEVER logged or placed in telemetry — only redacted,
display-safe metadata (job/tunnel/file IDs, lifecycle state, redacted reason
codes). Debug it with: client_log_sessions, tail_client_logs, client_log_trace,
client_error_summary.

## 2. SERVER (backend) telemetry → structured stdout + OTel (NOT Supabase)
ftnl-backend-api.rs follows the ORES observability contract: container
stdout/stderr is first-class telemetry (Promtail → Loki), with OTLP spans/metrics
where an OTEL endpoint is configured. That plane is separate from the
client→Supabase tables above; do not look for backend request logs in Supabase.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_mention_the_load_bearing_facts() {
        assert!(ORG_MAP.contains("ftnl-backend-api.rs"));
        assert!(ORG_MAP.contains("8080"));
        assert!(ORG_MAP.contains("apps/mcp-server"));
        assert!(ORG_MAP.contains("pairing secret"));
        assert!(ARCHITECTURE.contains("/v1/tunnels/{id}/claim"));
        assert!(ARCHITECTURE.contains("event ticket"));
        assert!(ARCHITECTURE.contains("capability"));
        assert!(TELEMETRY_DOCS.contains("ingest_ftnl_client_log_snapshot"));
        assert!(TELEMETRY_DOCS.contains("supabase_flutter"));
        assert!(TELEMETRY_DOCS.contains("NEVER logged"));
    }
}
