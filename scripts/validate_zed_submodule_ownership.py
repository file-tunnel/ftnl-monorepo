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
    # Keep the ownership gate aligned with doctor.sh: submodules must use
    # canonical HTTPS URLs in this organization, never SCP aliases, redirects,
    # credentials, extra paths, or traversal segments.
    parsed = urlparse(url)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "github.com"
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port is not None
        or parsed.query
        or parsed.fragment
        or not parsed.path.endswith(".git")
    ):
        fail("submodule URL must be canonical GitHub HTTPS without credentials")
    parts = parsed.path.removeprefix("/").removesuffix(".git").split("/")
    if (
        len(parts) != 2
        or parts[0] != "file-tunnel"
        or not parts[1]
        or parts[1].startswith(".")
        or not all(char.isascii() and (char.isalnum() or char in "._-") for char in parts[1])
    ):
        fail("submodule URL must target one file-tunnel repository")
    return "/".join(parts)


def validate_gitlink_path(path: str) -> None:
    parts = pathlib.PurePosixPath(path).parts
    if len(parts) != 2 or parts[0] != "apps" or parts[1] in {".", ".."}:
        fail(f"submodule must use one direct apps/ child: {path!r}")
    if path.startswith("/") or "//" in path or "\\\\" in path or "/../" in path:
        fail(f"unsafe submodule path: {path!r}")


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
    validate_gitlink_path(path)
    repo = repo_from_url(parser.get(section, "url"))
    if path in submodules:
        fail(f"duplicate submodule path: {path}")
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
