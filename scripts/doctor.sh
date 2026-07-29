#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

expected=(
  apps/backend-api
  apps/web-server
  apps/ui-components
  apps/clients
  apps/interfaces
  apps/sync
  apps/infra
  apps/e2e
  apps/site
)

for path in "${expected[@]}"; do
  git config -f .gitmodules --get-regexp '^submodule\..*\.path$' |
    awk '{print $2}' |
    grep -Fx "$path" >/dev/null ||
    { echo "missing submodule declaration: $path" >&2; exit 1; }
  test -e "$path/.git" ||
    { echo "submodule is not initialized: $path" >&2; exit 1; }
done

if git submodule status --recursive | grep -E '^[+-]'; then
  echo "submodule pins are missing or do not match the index" >&2
  exit 1
fi

if git config -f .gitmodules --get-regexp '\.url$' |
  awk '{print $2}' |
  grep -Ev '^https://github\.com/file-tunnel/.+\.git$'; then
  echo "all top-level submodule URLs must be canonical GitHub HTTPS URLs" >&2
  exit 1
fi

echo "File Tunnel workspace is pinned and initialized."
