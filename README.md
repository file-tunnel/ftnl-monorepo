# ftnl-monorepo

Pinned, reproducible workspace for the File Tunnel platform. Each application
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
```

This is an integration repository, not the source of truth for code copied out
of a submodule. Changes start in the owning repository, are released there,
then this workspace advances the pin in one reviewable commit.

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

## Pin policy

- Every gitlink points to an immutable commit tested by its own repository.
- Renovation of a pin is a normal pull request with the upstream commit link.
- Integration CI rejects uninitialized, dirty, or branch-tracking submodules.
- No submodule uses relative or mutable local URLs in committed `.gitmodules`.
- Security fixes advance affected pins promptly; this repo does not fork them.

MIT licensed.
