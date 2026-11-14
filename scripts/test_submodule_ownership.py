#!/usr/bin/env python3
"""Negative boundary tests for canonical monorepo submodule provenance."""
from __future__ import annotations

import contextlib
import io
import runpy
import unittest
from pathlib import Path

gate = runpy.run_path(
    str(Path(__file__).with_name("validate_zed_submodule_ownership.py"))
)
repo_from_url = gate["repo_from_url"]
validate_gitlink_path = gate["validate_gitlink_path"]


class SubmoduleProvenanceTests(unittest.TestCase):
    def test_canonical_https_repo_allowed(self) -> None:
        self.assertEqual(
            repo_from_url("https://github.com/file-tunnel/ftnl-sync.git"),
            "file-tunnel/ftnl-sync",
        )
        validate_gitlink_path("apps/sync")

    def test_untrusted_urls_are_rejected(self) -> None:
        for url in [
            "git@github.com:file-tunnel/ftnl-sync.git",
            "http://github.com/file-tunnel/ftnl-sync.git",
            "https://evil.example/file-tunnel/ftnl-sync.git",
            "https://github.com/elsewhere/ftnl-sync.git",
            "https://user:secret@github.com/file-tunnel/ftnl-sync.git",
            "https://github.com/file-tunnel/../ftnl-sync.git",
            "https://github.com/file-tunnel/.hidden.git",
            "https://github.com/file-tunnel/ftnl-sync.git?ref=main",
        ]:
            with self.subTest(url=url), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    repo_from_url(url)

    def test_unsafe_gitlink_paths_are_rejected(self) -> None:
        for path in [
            "apps/../escape",
            "apps/nested/child",
            "other/sync",
            "/apps/sync",
            "apps//sync",
            "apps/sync\\evil",
            "apps/sync:evil",
        ]:
            with self.subTest(path=path), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    validate_gitlink_path(path)


if __name__ == "__main__":
    unittest.main()
