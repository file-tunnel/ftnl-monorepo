//! Cargo build/check runner helpers for the File Tunnel Rust services
//! (`ftnl-backend-api.rs`, `ftnl-web-server.rs`, and this server).
//!
//! These are **build-only**: they shell out to the `cargo` CLI inside a repo
//! under the org root and never mutate anything but the repo's own `target/`
//! cache. Pure argv/parse helpers live here so the tool wrappers in `server.rs`
//! stay thin and unit-testable.
//!
//! `cargo build`/`cargo check` can resolve network dependencies on a cold
//! `target/`; nothing here can publish, run, install, or otherwise mutate state
//! outside the build cache. Subcommands are allow-listed and `--offline` is
//! honored via `FTNL_CARGO_OFFLINE=1`.

/// Subcommands this server is willing to run — all build-only / read-only.
pub const ALLOWED_SUBCOMMANDS: &[&str] = &["build", "check", "test", "clippy", "fmt"];

/// Build the argv for a cargo invocation, rejecting anything not allow-listed.
/// `locked` adds `--locked` (fail if `Cargo.lock` would change); `offline` adds
/// `--offline` so a hermetic environment never reaches crates.io.
pub fn cargo_args(subcommand: &str, locked: bool, offline: bool) -> Result<Vec<String>, String> {
    if !ALLOWED_SUBCOMMANDS.contains(&subcommand) {
        return Err(format!(
            "cargo subcommand {subcommand:?} is not allowed here (build-only surface: {})",
            ALLOWED_SUBCOMMANDS.join(", ")
        ));
    }
    let mut args = vec![subcommand.to_string()];
    match subcommand {
        // fmt is checked, never rewriting files (read-only invariant).
        "fmt" => {
            args.push("--check".to_string());
        }
        "clippy" => {
            args.push("--all-targets".to_string());
        }
        _ => {}
    }
    // --locked/--offline are meaningless for `fmt`.
    if subcommand != "fmt" {
        if locked {
            args.push("--locked".to_string());
        }
        if offline {
            args.push("--offline".to_string());
        }
    }
    Ok(args)
}

/// Whether cargo should run offline (set `FTNL_CARGO_OFFLINE=1`).
pub fn offline_from_env() -> bool {
    std::env::var("FTNL_CARGO_OFFLINE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Keep only the interesting lines from cargo output: errors, warnings, and the
/// compiling/checking/finished progress markers. Keeps tool output tight.
pub fn filter_diagnostics(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            let low = t.to_ascii_lowercase();
            low.starts_with("error")
                || low.starts_with("warning")
                || low.contains("error[")
                || low.contains("error:")
                || low.contains("warning:")
                || t.starts_with("Compiling")
                || t.starts_with("Checking")
                || t.starts_with("Finished")
                || t.starts_with("Running")
                || t.starts_with("Updating")
                || t.starts_with("Downloading")
                || t.starts_with("Blocking")
                || t.contains("test result:")
                || t.contains("Diff in")
        })
        .collect();
    if lines.is_empty() {
        // Nothing matched our filter — fall back to the tail so the caller
        // still sees something actionable.
        lines = text.lines().rev().take(20).collect();
        lines.reverse();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_args_allows_build_check_and_test() {
        assert_eq!(cargo_args("build", false, false).unwrap(), vec!["build"]);
        assert_eq!(
            cargo_args("build", true, true).unwrap(),
            vec!["build", "--locked", "--offline"]
        );
        assert_eq!(cargo_args("check", false, false).unwrap(), vec!["check"]);
        assert_eq!(
            cargo_args("clippy", false, false).unwrap(),
            vec!["clippy", "--all-targets"]
        );
        // fmt is always --check and never takes --locked/--offline
        assert_eq!(
            cargo_args("fmt", true, true).unwrap(),
            vec!["fmt", "--check"]
        );
    }

    #[test]
    fn cargo_args_rejects_arbitrary_subcommands() {
        for bad in [
            "publish",
            "install",
            "run",
            "--version; rm -rf /",
            "yank",
            "login",
        ] {
            assert!(
                cargo_args(bad, false, false).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn filter_diagnostics_keeps_errors_and_progress() {
        let out = "\
Compiling ftnl-backend-api v0.1.0
warning: unused import
error[E0433]: failed to resolve
Finished dev [unoptimized + debuginfo] target(s) in 1.20s
Running unittests
test result: ok. 12 passed; 0 failed
";
        let s = filter_diagnostics(out);
        assert!(s.contains("Compiling ftnl-backend-api"));
        assert!(s.contains("error[E0433]"));
        assert!(s.contains("warning: unused import"));
        assert!(s.contains("test result: ok"));
    }
}
