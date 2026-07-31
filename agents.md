# File Tunnel monorepo agent instructions

These instructions apply to this repository and every directory beneath it.

## Repository role

- This repository is a pinned integration view; each application under
  `apps/` remains owned by its source repository.
- Make implementation changes in the owning repository first. Do not edit code
  inside a submodule as though the monorepo were its source of truth.
- Advance gitlinks only to exact commits that pass the owning repository's
  validation. Keep submodules detached, clean, recursively initialized, and
  configured with immutable HTTPS repository URLs.
- Preserve the Compose integration contract and never place credentials,
  capabilities, pairing secrets, or private deployment data in orchestration
  files.

## Validation

- Run `nix develop --command agent-check` before completing a change.
- Verify recursive submodule status whenever a gitlink changes.
- Do not commit dirty submodules, branch-tracking configuration, local
  submodule URLs, generated build trees, or machine-specific state.

## Git workflow

- Keep changes focused and reviewable.
- Pull and merge remote work before pushing; avoid git rebase in favor of git merge.
- Never discard unrelated or uncommitted user work.
