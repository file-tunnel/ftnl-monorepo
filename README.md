# ftnl-monorepo

Pinned, reproducible workspace for the File Tunnel platform. Each canonical application
under `apps/` is a git submodule that keeps its own release history, package
boundaries, and CI while this repository provides a tested integration view.

## Layout

```text
apps/
  backend-api/     file-tunnel/ftnl-backend-api.rs
  web-server/      file-tunnel/ftnl-web-server.rs
  ui-components/   file-tunnel/ftnl-ui-components
  clients/         file-tunnel/ftnl-clients
  interfaces/      file-tunnel/ftnl-interfaces
  sync/            file-tunnel/ftnl-sync
  infra/           file-tunnel/ftnl-infra
  e2e/             file-tunnel/ftnl-e2e
  site/            file-tunnel/file-tunnel.github.io

incubator/
  ftnl-mcp-server.rs/  self-contained Rust MCP server pending promotion to
                       file-tunnel/ftnl-mcp-server.rs
```

This is an integration repository, not the source of truth for code copied out
of a submodule. Changes start in the owning repository, are released there,
then this workspace advances the pin in one reviewable commit.

The MCP server is the temporary exception: the connected GitHub write surface
can update existing repositories but cannot create its intended standalone
repository. It therefore lives under `incubator/` with an explicit promotion
checklist and independent CI until `file-tunnel/ftnl-mcp-server.rs` exists.

## Clone

```bash
git clone --recurse-submodules https://github.com/file-tunnel/ftnl-monorepo.git
cd ftnl-monorepo
nix develop --command agent-check
```

Nested submodules are required because `ftnl-sync` pins the reviewed
`opto-sync-clients` and `syncer.c` revisions.

## Local integration

```bash
nix develop
docker compose up --build
```

The compose profile starts the Rust API and phone portal. Use the E2E
repository for the two-browser QR handoff contract.

Each submodule owns its language toolchain and exposes the same
`nix develop --command agent-check` convention. The monorepo shell is kept
small and only carries orchestration, gitlink, and Compose tooling.

Run the MCP server against the local API with:

```bash
FTNL_API_BASE_URL=http://127.0.0.1:8080 \
  cargo run --manifest-path incubator/ftnl-mcp-server.rs/Cargo.toml
```

## Pin policy

- Every gitlink points to an immutable commit tested by its own repository.
- Renovation of a pin is a normal pull request with the upstream commit link.
- Integration CI rejects uninitialized, dirty, or branch-tracking submodules.
- No submodule uses relative or mutable local URLs in committed `.gitmodules`.
- Security fixes advance affected pins promptly; this repo does not fork them.
- Incubated code must remain self-contained, independently tested, and carry a
  documented migration path into its canonical standalone repository.

MIT licensed.
