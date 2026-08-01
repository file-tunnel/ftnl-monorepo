# Agent guidelines — ftnl-mcp-server.rs

MCP server (stdio) exposing read-only / build-only tools for the **file-tunnel**
org — secure, ephemeral, QR-connected cross-device file transfers. See README.md
for the tool table and env configuration.

## Hard rules

- **stdout is the MCP wire.** Never print to stdout in the binary path; all
  diagnostics go to stderr. A stray stdout write corrupts the JSON-RPC stream.
- **Tools stay read-only or build-only.** No deletes, moves, git history
  rewrites, writes to Supabase/Cloudflare/k8s, or transfer mutation (no tunnel
  creation, claiming, file declaration, or byte movement). `cargo_build` /
  `cargo_check` only run the `cargo` CLI (which touches the repo's own `target/`
  cache); the API probes are GETs against `/healthz` only. The tunnel lifecycle
  routes all fail closed without a capability, so this server never dials them.
- **Validate everything that reaches a CLI or URL.** Repo names go through
  `util::safe_segment` (no `/`, `\`, `..`, leading `.`); refs through
  `util::safe_token`; session ids through `util::safe_opaque_id` (also forbids
  `/`, `..`, `@`, `:` so it can't smuggle a path/scheme/PostgREST operator);
  hostnames through `util::safe_hostname`; record types through
  `util::safe_record_type`; operator base URLs through `util::safe_base_url`
  (http(s) only, no creds/query/fragment, blocks cloud-metadata hosts); cargo
  subcommands are allow-listed in `builds::cargo_args`. This is what stops
  flag/shell/path/scheme injection — keep the unit tests in sync when you touch it.
- **Never log secrets.** `CLOUDFLARE_API_TOKEN`, `SUPABASE_SERVICE_ROLE_KEY`,
  `GITHUB_TOKEN`, `FIDUCIA_TOKEN` are only ever attached as headers, never printed.
- **Bound every network read.** Use `util::read_*_capped` (size + time bounds);
  every reqwest client sets a timeout. A malformed upstream must yield a typed
  error, not a crash or an OOM.

## Where things live

- `src/server.rs` — the `#[tool_router]` impl; one tool per question. Tools
  return `Result<String, String>` so upstream errors reach the model as text.
  Also wires MCP resources/prompts into `ServerHandler`.
- `src/util.rs` — subprocess runner (timeout, kill-on-drop), output truncation,
  input validators, capped HTTP body readers, `FTNL_ROOT` resolution.
- `src/builds.rs` — build-only cargo argv builder + cargo-output diagnostics filter.
- `src/transfer.rs` — backend API route table + `/healthz` probe (default base
  `http://127.0.0.1:8080`).
- `src/interfaces.rs` — `ftnl-interfaces` contract listing + org repo/release inventory.
- `src/github.rs` — org CI runs, `.gitmodules` pin parser, github.io site check.
- `src/domains.rs` / `src/cloudflare.rs` — DoH/RDAP/TLS + Cloudflare v4 API.
- `src/supabase.rs` — PostgREST reads over the `ftnl_client_log_*` tables:
  `sessions`, `tail`, `trace`, and `error_summary` with a `GroupBy`.
- `src/status.rs` — `Health`/`Check`/`overall`/`format_rollup` for `stack_status`,
  plus the presence-only `capabilities_report` (`self_test`).
- `src/fiducia.rs` — read-only fiducia.cloud secrets-presence + lease-health probe.
- `src/resources.rs` / `src/prompts.rs` — the MCP resources (org map, docs,
  schema) and canned prompts, wired into `ServerHandler` in `server.rs`.
- `src/docs.rs` — embedded `org_map`, `architecture`, `telemetry_docs`. **Update
  these when repos, ports, or the deployment shape change** (last sync 2026-07).
- `supabase/schema.sql` — dpm-style declarative schema for the client-telemetry
  tables + ingest RPCs + RLS + realtime publication.

## Org context

File Tunnel = a desktop upload field paired to files on a phone. A tunnel UUID is
a routing identifier, not a credential; the one-time pairing secret lives in the
URL fragment (`#c=…`) and is exchanged once for a phone-scoped capability;
desktop and phone capabilities are separate (stored as SHA-256 digests); browser
WebSockets use one-time event tickets. `ftnl-monorepo` pins every deployable repo
as a git submodule under `apps/` (this server is `apps/mcp-server`). Contracts
live in `ftnl-interfaces` (OpenAPI/AsyncAPI/JSON Schema).

## Checks

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Smoke-test the wire without an MCP client:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | cargo run --quiet
```

## Syncing with the remote

"Sync" is a **two-way** exchange — pull the remote's commits down **and** push
yours up. Commit your work first (clean tree), `git fetch --all --prune`, then
`git pull` (fetch + merge) to integrate, then `git push`. Integrate with
**`git merge` / `git pull`** — **never `git rebase` to sync** (it rewrites
history and breaks shared branches).
