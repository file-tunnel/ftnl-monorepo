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

<!-- BEGIN ores-agents-pointer: managed by ORESoftware/my-ai; edit there, not here -->

## Canonical agent instructions

Before doing anything else in this repository, also read:

    .ores/agents/AGENTS.md

That path is a symlink to `~/codes/oresoftware/my-ai/AGENTS.md`, whose canonical copy is
<https://github.com/ORESoftware/my-ai/blob/main/AGENTS.md>.

It exists at a fixed path *inside* the repository because some agents cannot walk up past
the repository root, so machine-wide instructions one or more directories above are
invisible to them. This pointer plus that path make the same file reachable from a working
directory anywhere in the tree.

The symlink is deliberately **not committed**: it names an absolute path that is only valid
on a machine with `~/codes/oresoftware/my-ai` checked out, so committing it would produce a
broken link for everyone else and for CI. `.ores/` is git-ignored for that reason. If
`.ores/agents/AGENTS.md` is missing on your machine, create it with:

    mkdir -p .ores/agents
    ln -sfn "$HOME/codes/oresoftware/my-ai/AGENTS.md" .ores/agents/AGENTS.md

or run `~/codes/oresoftware/my-ai/scripts/link-repo-agents.sh` once to do it for every git
repository under `~/codes`, and `--check` to verify them.

A missing `.ores/agents/AGENTS.md` is a setup gap on the reader's machine, never a reason to
skip the canonical instructions: fetch them from the URL above instead.

<!-- END ores-agents-pointer -->
