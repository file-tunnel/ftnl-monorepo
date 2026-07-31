# File Tunnel MCP server

Rust stdio MCP server for safe File Tunnel inspection, transfer planning, and input validation.

This is the canonical implementation incubator for `file-tunnel/ftnl-mcp-server.rs` while the standalone repository is not yet available through the connected repository-creation surface. The code is kept in the File Tunnel organization—not in a ZIP—and is ready to split into the dedicated repository without changing its package boundary.

## Tools

- `file_tunnel_capabilities` — reports the current read-only capability and safety boundary.
- `file_tunnel_plan_transfer` — computes a bounded chunk plan without touching storage or the network.
- `file_tunnel_validate_object_key` — validates a relative object key and rejects traversal, control characters, empty segments, Windows separators, and oversized keys.

## Safety boundary

- JSON-RPC protocol output is written only to stdout.
- Diagnostics are written only to stderr.
- Input lines are capped at 1 MiB.
- No credentials, network calls, filesystem writes, or transfer mutations are accepted.
- Unknown tools and malformed arguments fail closed.
- Transfer sizes and chunk sizes are bounded before arithmetic.

## Run

```bash
cargo run --manifest-path apps/mcp-server/Cargo.toml
```

Send one JSON-RPC request per line. Example:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_tunnel_plan_transfer","arguments":{"size_bytes":10485760,"chunk_size_bytes":1048576}}}
```

## Next extraction step

Create `file-tunnel/ftnl-mcp-server.rs`, preserve this crate history, add it to `ftnl-monorepo` as `apps/mcp-server`, and replace generic protocol/config/safety helpers with a pinned reviewed `ORESoftware/mcp-rust-libs` release once that repository is published.
