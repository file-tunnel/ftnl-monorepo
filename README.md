# ftnl-mcp-server.rs

An [MCP](https://modelcontextprotocol.io) server (stdio, Rust + [rmcp](https://crates.io/crates/rmcp))
with read-only / build-only tools for the **[file-tunnel](https://github.com/file-tunnel)**
org — *secure, ephemeral, QR-connected cross-device file transfers.* Site:
[file-tunnel.github.io](https://file-tunnel.github.io).

File Tunnel bridges a desktop upload field to files that live on a phone: scan a
short-lived QR, pick photos/files on the phone, and watch them arrive in the app
already open on the computer — no emailing yourself anything. The backend
(`ftnl-backend-api.rs`, Axum, default `:8080`) mints ephemeral tunnels; a tunnel
UUID is only an address and grants no access — a one-time pairing secret in the
URL fragment (`#c=…`) is exchanged once for a phone-scoped capability, and
separate desktop/phone capabilities plus one-time WebSocket event tickets gate
everything else. This server gives coding agents a safe toolbox for that org.

## Tools

| Tool | What it does |
| --- | --- |
| `org_map` | Embedded org/architecture map: repos, capability model, tunnel lifecycle, backend/portal ports, sync boundary, conventions. |
| `org_overview` | Git status across repos checked out under the org root. |
| `repo_status` | Detailed git status for one repo (branch, status, commits, worktrees, stashes). |
| `recent_commits` | `git log` for a repo. |
| `search_code` | `git grep -E` across the org (or one repo) — skips `build/` and `target/`. |
| `cargo_build` | **Build-only** `cargo build` in a repo under the org root. |
| `cargo_check` | **Build-only** `cargo check` in a repo under the org root. |
| `transfer_api_health` | Probe the backend's unauthenticated `GET /healthz`. |
| `tunnel_api_routes` | Map the tunnel/transfer API surface: probe `/healthz`, list the capability-scoped lifecycle routes. |
| `architecture_docs` | Doc: two-capability model, control/data plane routes, realtime events, sync state machine. |
| `interface_contracts` | List `ftnl-interfaces` OpenAPI/AsyncAPI/JSON-Schema/fixtures/generated bindings (GitHub contents API). |
| `repo_inventory` | Release/repo roll-up of the whole org (description, visibility, default branch, last push, latest release). |
| `monorepo_pins` | Read `ftnl-monorepo`'s submodule pins from `.gitmodules`. |
| `org_ci_status` | Latest GitHub Actions run per org repo. |
| `site_check` | Liveness/title check of the github.io marketing site. |
| `dns_lookup` | DNS-over-HTTPS records via Cloudflare 1.1.1.1. |
| `domain_info` | RDAP registrar/expiry/NS classification (covers Squarespace domains). |
| `tls_cert_check` | TLS cert subject/issuer/expiry at host:port. |
| `cloudflare_zones` | List Cloudflare zones (read-only token). |
| `cloudflare_dns_records` | List DNS records in a Cloudflare zone. |
| `client_log_sessions` | Recent client-telemetry sessions from Supabase. |
| `tail_client_logs` | Tail one session's log entries from Supabase (newest first, level filter). |
| `client_log_trace` | One session's FULL ordered timeline (oldest → newest). |
| `client_error_summary` | Group recent client error/warn entries by `message` \| `url`/`route` \| `category`. |
| `telemetry_docs` | Doc: the client→Supabase vs backend→stdout/OTel telemetry planes. |
| `stack_status` | **Flagship** GREEN/DEGRADED/RED readiness rollup: cargo toolchain + backend `/healthz` + Pages DNS + site + org CI. |
| `self_test` | Which env creds are present → which tool families are LIVE vs DEGRADED (presence only, never a value). |
| `fiducia_status` | Read-only check of fiducia.cloud secrets presence + lock/lease health. |

All tools are **read-only or build-only** — no deletes, moves, git history
rewrites, cloud/k8s writes, or transfer mutation (no tunnel creation, claiming,
or byte movement). User-supplied values that reach a CLI or URL are validated
against conservative charsets.

## Resources & prompts

Embedded org knowledge is also exposed as MCP **resources** (loadable without a
tool call) and canned **prompts**:

| Resource URI | Contents |
| --- | --- |
| `orgmap://file-tunnel` | Org/architecture map (repos, capability model, lifecycle, conventions). |
| `docs://architecture` | Two-capability model, control/data plane routes, realtime events, sync. |
| `docs://telemetry` | Client→Supabase vs backend→stdout/OTel telemetry planes. |
| `schema://client-telemetry` | The declarative `supabase/schema.sql` (dpm-style DDL). |

| Prompt | Purpose |
| --- | --- |
| `deploy_readiness` | Drive `stack_status` + corroborating tools into a GO/NO-GO. |
| `triage_client_errors` | Aggregate client errors (`group_by`) then trace the worst session. |
| `domain_audit` | RDAP/DNS/TLS/Cloudflare audit of a domain (default `file-tunnel.github.io`). |
| `transfer_api_review` | Probe the live backend + review the capability-scoped lifecycle against the contracts. |

## Build & test

```sh
cargo build --release
cargo test
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

Requires a stable Rust toolchain (see `rust-toolchain.toml`). `cargo_build` /
`cargo_check` additionally require the `cargo` CLI on `PATH` at runtime; they
emit a clear error if it is missing.

## Register with Claude

```sh
claude mcp add ftnl -- /absolute/path/to/target/release/ftnl-mcp
```

## Environment variables

| Var | Used by | Notes |
| --- | --- | --- |
| `FTNL_ROOT` | org/git, cargo tools | Org root on disk. Default `~/codes/file-tunnel`. |
| `FTNL_API_URL` | `transfer_api_health`, `tunnel_api_routes` | Backend base URL. Default `http://127.0.0.1:8080` (run `ftnl-backend-api.rs` locally or port-forward the service). |
| `FTNL_CARGO_OFFLINE` | `cargo_build`, `cargo_check` | Set `1` to pass `--offline`. |
| `GITHUB_TOKEN` / `GH_TOKEN` | `interface_contracts`, `repo_inventory`, `monorepo_pins`, `org_ci_status` | Optional; needed for private repos + higher rate limit. |
| `CLOUDFLARE_API_TOKEN` | `cloudflare_*` | Read-only token (Zone:Read + DNS:Read). Cloudflare account `alexander.d.mills@gmail.com`. |
| `SUPABASE_URL` | `client_log_*` | Supabase project URL (`https://<ref>.supabase.co`). |
| `SUPABASE_SERVICE_ROLE_KEY` | `client_log_*` | Service-role key (bypasses RLS for read-only debugging; never logged). |
| `FIDUCIA_URL` / `FIDUCIA_TOKEN` | `fiducia_status` | fiducia.cloud base URL + read-scoped token (secrets/leases plane). |
| `FIDUCIA_REQUIRED_SECRETS` | `fiducia_status` | Optional comma-separated secret names to assert present. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | telemetry | Optional; enable OTLP/gRPC trace+metric export. |

## Client telemetry

Every File Tunnel client streams a bounded, redacted log ring buffer **directly**
into Supabase (no app-server hop): the WASM/TypeScript upload portal + web UI via
`supabase-js`, the Dart/Flutter UI via `supabase_flutter`, both calling the
`ingest_ftnl_client_log_snapshot` / `ingest_ftnl_client_log_entries` RPCs. Data
lands in the RLS-protected `ftnl_client_log_snapshots` / `ftnl_client_log_entries`
tables (added to the `supabase_realtime` publication). Pairing secrets,
capabilities, event tickets, presigned URLs, and file bytes are **never** part of
the telemetry — only redacted, display-safe metadata. The declarative schema is in
[`supabase/schema.sql`](supabase/schema.sql) (dpm-managed). The `client_log_*`
tools read that data back; `telemetry_docs` explains it — including that the
backend itself uses a separate plane (structured stdout → Promtail/Loki, plus
OTLP), not Supabase.

## Notes

stdio-only: this server never serves HTTP; stdout is the MCP wire and all logs go
to stderr. See [`AGENTS.md`](AGENTS.md) for repo rules.

## OpenTelemetry

Set `OTEL_EXPORTER_OTLP_ENDPOINT` to export explicit OTLP/gRPC traces and
metrics; use `RUST_LOG` for filtering. Each MCP tool call gets a named span,
call counter, duration histogram, and error flag. Arguments, results, and
secrets are never recorded. JSON logs stay on stderr and stdout stays reserved
for MCP framing. Instrumentation is explicit Rust code — no monkey patching.
