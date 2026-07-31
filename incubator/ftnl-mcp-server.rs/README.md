# ftnl-mcp-server.rs

Rust Model Context Protocol server for the File Tunnel control-plane API.

This implementation is temporarily incubated in `ftnl-monorepo` because the connected GitHub write surface can update existing repositories but cannot create the intended standalone `file-tunnel/ftnl-mcp-server.rs` repository. The package is self-contained so it can be moved without changing its protocol or source layout once that repository exists.

## Tools

| Tool | Behavior |
|---|---|
| `health` | Calls `GET /healthz`. |
| `create_tunnel` | Calls `POST /v1/tunnels` and returns the pairing URI plus one-time desktop capability. |
| `get_tunnel` | Calls `GET /v1/tunnels/{id}` with the desktop bearer capability. |
| `cancel_tunnel` | Calls `DELETE /v1/tunnels/{id}` only when `confirm=true`. |

The API implementation remains authoritative. This server does not recreate File Tunnel lifecycle logic, persist capabilities, proxy file bytes, or automatically retry destructive operations.

## Run

Start the File Tunnel API, then configure an MCP client to launch:

```bash
FTNL_API_BASE_URL=http://127.0.0.1:8080 \
  cargo run --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml
```

Example client configuration:

```json
{
  "mcpServers": {
    "file-tunnel": {
      "command": "cargo",
      "args": [
        "run",
        "--quiet",
        "--manifest-path",
        "/absolute/path/to/ftnl-monorepo/incubator/ftnl-mcp-server.rs/Cargo.toml"
      ],
      "env": {
        "FTNL_API_BASE_URL": "http://127.0.0.1:8080"
      }
    }
  }
}
```

## Security boundary

- Desktop capabilities are accepted only as tool parameters and are sent in the `Authorization` header.
- The HTTP client refuses redirects so bearer capabilities cannot be forwarded to another origin.
- Base URLs containing credentials, query strings, fragments, or non-HTTP schemes are rejected.
- Responses are bounded to 64 KiB before entering the MCP result.
- Cancellation requires explicit confirmation and is never retried after an uncertain response.
- Pairing secrets and capabilities must not be copied into logs, issue bodies, analytics, or model-training data.

For production, run this server under a least-privilege local account and use HTTPS or a private network for remote APIs.

## Validate

```bash
cargo fmt --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml --all -- --check
cargo check --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml --all-targets
cargo test --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml --all-targets
cargo clippy --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml --all-targets
```

## Promotion checklist

1. Create `file-tunnel/ftnl-mcp-server.rs`.
2. Move this directory without rewriting history where practical.
3. Add the new repository as `apps/mcp-server` in `.gitmodules`.
4. Pin the verified standalone commit in the monorepo.
5. Enable repository-level release, dependency, security, and Linear/GitHub issue synchronization.
6. Remove this incubator copy after the submodule is proven.
