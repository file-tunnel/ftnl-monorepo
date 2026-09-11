#!/usr/bin/env python3
from __future__ import annotations

import configparser
import pathlib
import subprocess
import sys
import tomllib
from urllib.parse import urlparse

ROOT = pathlib.Path(__file__).resolve().parents[1]
STANDALONE_ONLY_REPOS = {
    "file-tunnel/ftnl-cli",
    "file-tunnel/ftnl-infra",
    "file-tunnel/ftnl-mcp-server.rs",
    "file-tunnel/ftnl-sidecar.rs",
}


def fail(message: str) -> None:
    print(f"submodule-ownership: {message}", file=sys.stderr)
    raise SystemExit(1)


def repo_from_url(url: str) -> str:
    normalized = url.strip()
    if normalized.startswith("git@github.com:"):
        path = normalized.split(":", 1)[1]
    else:
        parsed = urlparse(normalized)
        if parsed.hostname != "github.com":
            fail(f"unsupported non-GitHub submodule URL: {url}")
        path = parsed.path.lstrip("/")
    return path.removesuffix(".git")


with (ROOT / ".zpkg.toml").open("rb") as handle:
    manifest = tomllib.load(handle)
if manifest.get("package", {}).get("org") != "file-tunnel" or manifest.get("package", {}).get("name") != "ftnl-monorepo":
    fail("unexpected package identity")
if (ROOT / ".zpkg.lock").read_text(encoding="utf-8").strip() != "version = 1":
    fail(".zpkg.lock must contain exactly 'version = 1'")

dependencies = set(manifest.get("dependencies", {}))
if dependencies & STANDALONE_ONLY_REPOS:
    fail(f"standalone-only Zed dependencies are forbidden: {sorted(dependencies & STANDALONE_ONLY_REPOS)}")

parser = configparser.ConfigParser()
parser.read(ROOT / ".gitmodules", encoding="utf-8")
if not parser.sections():
    fail(".gitmodules must declare the application repositories")

submodules: dict[str, str] = {}
for section in parser.sections():
    path = parser.get(section, "path")
    repo = repo_from_url(parser.get(section, "url"))
    if repo in submodules.values():
        fail(f"duplicate submodule repository: {repo}")
    submodules[path] = repo

forbidden_submodules = set(submodules.values()) & STANDALONE_ONLY_REPOS
if forbidden_submodules:
    fail(f"standalone-only submodules are forbidden: {sorted(forbidden_submodules)}")

dual_owned = set(submodules.values()) & dependencies
if dual_owned:
    fail(f"repositories cannot be both submodules and Zed dependencies: {sorted(dual_owned)}")

index = subprocess.run(["git", "ls-files", "-s"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
index_modes: dict[str, str] = {}
for line in index.splitlines():
    metadata, path = line.split("\t", 1)
    index_modes[path] = metadata.split()[0]
for path in submodules:
    if index_modes.get(path) != "160000":
        fail(f"submodule path {path!r} is not a gitlink in the index")

print(f"submodule-ownership: ok ({len(submodules)} gitlinks, {len(dependencies)} Zed dependencies)")
