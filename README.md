# ftnl-monorepo

Pinned, reproducible application workspace for the File Tunnel platform.
Canonical applications under `apps/` keep their own package boundaries and CI
while this repository provides the tested integration view.

## Layout

```text
apps/
  backend-api/     file-tunnel/ftnl-backend-api.rs (git submodule)
  web-server/      file-tunnel/ftnl-web-server.rs (git submodule)
  ui-components/   file-tunnel/ftnl-ui-components (git submodule)
  clients/         file-tunnel/ftnl-clients (git submodule)
  interfaces/      file-tunnel/ftnl-interfaces (git submodule)
  sync/            file-tunnel/ftnl-sync (git submodule)
  e2e/             file-tunnel/ftnl-e2e (git submodule)
  site/            file-tunnel/file-tunnel.github.io (git submodule)
  mcp-server/      self-contained Rust MCP package pending extraction to
                   file-tunnel/ftnl-mcp-server.rs
```

[`file-tunnel/ftnl-infra`](https://github.com/file-tunnel/ftnl-infra) is a
standalone infrastructure repository with its own history, review surface, and
deployment lifecycle. It is deliberately absent from this monorepo; application
integration uses documented configuration, artifacts, and deployment contracts
rather than an infrastructure Git submodule.

This is an integration repository, not the source of truth for code copied out
of a submodule. Changes start in the owning application repository, are released
there, then this workspace advances the pin in one reviewable commit.

`apps/mcp-server` is the temporary exception. The connected GitHub write surface
can update existing repositories but cannot create its intended standalone
repository. The crate therefore remains self-contained, independently tested,
and ready to extract without changing its package boundary once
`file-tunnel/ftnl-mcp-server.rs` exists.

## Clone

```bash
git clone --recurse-submodules https://github.com/file-tunnel/ftnl-monorepo.git
cd ftnl-monorepo
nix develop --command agent-check
```

Nested application submodules are required because `ftnl-sync` pins the reviewed
`opto-sync-clients` and `syncer.c` revisions.

## Local integration

```bash
nix develop
docker compose up --build
```

The compose profile starts the Rust API and phone portal. Use the E2E
repository for the two-browser QR handoff contract.

Each application submodule owns its language toolchain and exposes the same
`nix develop --command agent-check` convention. The monorepo shell is kept
small and only carries orchestration, gitlink, and Compose tooling.

Run the validated read-only MCP server with:

```bash
cargo run --manifest-path apps/mcp-server/Cargo.toml
```

## Pin policy

- Every application gitlink points to an immutable commit tested by its own repository.
- Renovation of a pin is a normal pull request with the upstream commit link.
- Integration CI rejects uninitialized, dirty, or branch-tracking submodules.
- CI rejects any submodule path or URL that identifies an infrastructure repository.
- No submodule uses relative or mutable local URLs in committed `.gitmodules`.
- Security fixes advance affected pins promptly; this repo does not fork them.
- The MCP package remains read-only and fail-closed until authenticated API
  operations receive a separately reviewed capability and retry design.

MIT licensed.
