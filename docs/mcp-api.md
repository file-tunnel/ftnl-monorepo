# File Tunnel MCP API and route boundary

## Transport

`ftnl-mcp` is a **stdio-only** Model Context Protocol server implemented with the official Rust MCP SDK (`rmcp`). A client starts the executable and exchanges JSON-RPC/MCP frames over stdin and stdout. The SDK owns framing, initialization, capability negotiation, cancellation, discovery, request validation, and dispatch.

This repository serves **no HTTP, SSE, WebSocket, health, metrics-scrape, or browser routes**. It opens no listening port. Stdout is reserved exclusively for MCP frames; logs and diagnostics use stderr.

Several tools inspect routes exposed by other File Tunnel services. Those routes are remote read-only probe targets, not routes served by this executable:

- `GET /healthz` on the transfer backend;
- capability-scoped tunnel lifecycle routes described by `tunnel_api_routes` and `architecture_docs`;
- GitHub, Cloudflare, RDAP, DNS-over-HTTPS, Supabase, and Fiducia read APIs.

## Tool contract

The complete current catalog is discoverable through MCP `tools/list` and summarized in `README.md`. Tool families are:

| Family | Examples | Boundary |
| --- | --- | --- |
| Organization and Git | `org_map`, `org_overview`, `repo_status`, `recent_commits`, `search_code` | Local read-only repository inspection. |
| Build-only | `cargo_build`, `cargo_check` | Validated repository name and fixed Cargo command shapes; no publishing or deployment. |
| Transfer diagnostics | `transfer_api_health`, `tunnel_api_routes`, `stack_status` | Bounded, unauthenticated/read-only health and contract probes; no tunnel creation, claim, upload, download, or event-ticket mutation. |
| Contracts and releases | `interface_contracts`, `repo_inventory`, `monorepo_pins`, `org_ci_status` | Read-only GitHub metadata and interface artifacts. |
| Domain and DNS | `site_check`, `dns_lookup`, `domain_info`, `tls_cert_check`, `cloudflare_zones`, `cloudflare_dns_records` | Read-only network metadata; Cloudflare token must be read-scoped. |
| Client telemetry | `client_log_sessions`, `tail_client_logs`, `client_log_trace`, `client_error_summary`, `telemetry_docs` | Read-only, bounded Supabase diagnostics; secrets and capability values are never returned intentionally. |
| Coordination health | `fiducia_status` | Read-only secret-presence and lease/lock health. |

All tools are read-only or build-only. The server has no tool for deletion, moves, Git history rewriting, cloud/Kubernetes mutation, transfer mutation, capability issuance, tunnel creation, file-byte movement, or deployment.

## Inputs and outputs

User-controlled repository names, command filters, domains, hosts, record types, path segments, and URLs are validated before reaching a process or network boundary. Subprocess output is drained concurrently with fixed stdout/stderr limits and the child is killed and reaped on timeout or overflow. Error messages omit command argument values because they can contain private paths or operator input.

Network bodies are bounded before buffering where supported. Diagnostics should return compact metadata, not provider response bodies. File contents, transfer capabilities, pairing secrets, event tickets, presigned URLs, authorization headers, cookies, service-role keys, and environment snapshots are outside the MCP result contract.

## Environment

Operational variables and the tool families they enable are documented in `README.md`. Presence-only startup diagnostics never print values. OpenTelemetry behavior is defined in `OBSERVABILITY.md`.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
cargo build --locked --release
cargo audit --deny warnings
```
