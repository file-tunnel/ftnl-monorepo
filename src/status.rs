//! Operator-facing status helpers for the File Tunnel stack:
//!
//! * `Health` + `Check` — the rollup vocabulary for the aggregate `stack_status`
//!   tool (assembled in server.rs from the existing probes: cargo toolchain,
//!   backend API health, apex DNS, marketing site, org CI).
//! * `capabilities_report` — which env creds are present, so an operator sees
//!   at a glance which tool families are fully live vs degraded.

/// Rollup health level for a single check and for the aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Green,
    Degraded,
    Red,
    /// Not determinable — usually a missing credential, not a real failure.
    Unknown,
}

impl Health {
    pub fn tag(self) -> &'static str {
        match self {
            Health::Green => "GREEN",
            Health::Degraded => "DEGRADED",
            Health::Red => "RED",
            Health::Unknown => "UNKNOWN",
        }
    }
    /// Severity for reduction (higher = worse). UNKNOWN never worsens the roll-up.
    fn rank(self) -> u8 {
        match self {
            Health::Green => 0,
            Health::Unknown => 1,
            Health::Degraded => 2,
            Health::Red => 3,
        }
    }
}

pub struct Check {
    pub name: &'static str,
    pub health: Health,
    pub detail: String,
}

/// Reduce checks to an overall verdict. RED if any RED, else DEGRADED if any
/// DEGRADED, else GREEN (UNKNOWNs are surfaced but do not flip a green stack).
pub fn overall(checks: &[Check]) -> Health {
    let worst = checks.iter().map(|c| c.health).fold(Health::Green, |a, b| {
        if b.rank() > a.rank() {
            b
        } else {
            a
        }
    });
    if worst == Health::Unknown {
        Health::Green
    } else {
        worst
    }
}

pub fn format_rollup(checks: &[Check]) -> String {
    let verdict = overall(checks);
    let mut out = format!("File Tunnel stack status: {}\n\n", verdict.tag());
    for c in checks {
        out.push_str(&format!(
            "  [{}] {}: {}\n",
            c.health.tag(),
            c.name,
            c.detail
        ));
    }
    let failing: Vec<&str> = checks
        .iter()
        .filter(|c| matches!(c.health, Health::Red | Health::Degraded))
        .map(|c| c.name)
        .collect();
    if failing.is_empty() {
        out.push_str("\nno failing checks.\n");
    } else {
        out.push_str(&format!("\nfailing checks: {}\n", failing.join(", ")));
    }
    let unknown: Vec<&str> = checks
        .iter()
        .filter(|c| c.health == Health::Unknown)
        .map(|c| c.name)
        .collect();
    if !unknown.is_empty() {
        out.push_str(&format!(
            "unknown (missing credential/tool, not a failure): {}\n",
            unknown.join(", ")
        ));
    }
    out
}

/// Report which credentials are configured and therefore which tool families
/// are fully live vs degraded. Reads presence only — never the secret value.
pub fn capabilities_report() -> String {
    let present = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
    let mark = |ok: bool| if ok { "present" } else { "MISSING" };

    let supa = present("SUPABASE_URL") && present("SUPABASE_SERVICE_ROLE_KEY");
    let gh = present("GITHUB_TOKEN") || present("GH_TOKEN");
    let cf = present("CLOUDFLARE_API_TOKEN");
    let fiducia = present("FIDUCIA_URL") && present("FIDUCIA_TOKEN");
    let root = crate::util::org_root();

    let mut out = String::from("ftnl-mcp-server self-test — credential & capability matrix\n\n");
    out.push_str(&format!(
        "server: {} v{}\norg root: {} ({})\nbackend API URL: {}\n\n",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        root.display(),
        if root.is_dir() { "exists" } else { "MISSING" },
        if present("FTNL_API_URL") {
            "configured"
        } else {
            "default localhost:8080"
        },
    ));
    out.push_str("## environment\n");
    for (k, v) in [
        ("FTNL_ROOT", present("FTNL_ROOT")),
        ("FTNL_API_URL", present("FTNL_API_URL")),
        ("SUPABASE_URL", present("SUPABASE_URL")),
        (
            "SUPABASE_SERVICE_ROLE_KEY",
            present("SUPABASE_SERVICE_ROLE_KEY"),
        ),
        ("GITHUB_TOKEN", present("GITHUB_TOKEN")),
        ("GH_TOKEN", present("GH_TOKEN")),
        ("CLOUDFLARE_API_TOKEN", cf),
        ("FIDUCIA_URL", present("FIDUCIA_URL")),
        ("FIDUCIA_TOKEN", present("FIDUCIA_TOKEN")),
    ] {
        out.push_str(&format!("  {k}: {}\n", mark(v)));
    }

    out.push_str("\n## tool families\n");
    let line = |name: &str, live: bool, note: &str| {
        format!(
            "  {name}: {}  ({note})\n",
            if live { "LIVE" } else { "DEGRADED" }
        )
    };
    out.push_str(&line(
        "org/git, search_code, cargo_build/check, org_map, architecture_docs",
        root.is_dir(),
        "local — no creds; cargo_* also need the cargo/rust toolchain on PATH",
    ));
    out.push_str(&line(
        "transfer_api_health, tunnel_api_routes",
        true,
        "needs a reachable backend (FTNL_API_URL, default :8080)",
    ));
    out.push_str(&line(
        "domains (dns_lookup, domain_info, tls_cert_check)",
        true,
        "public DoH/RDAP/TLS — always live",
    ));
    out.push_str(&line(
        "client_log_sessions, tail_client_logs, client_log_trace, client_error_summary",
        supa,
        "needs SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY",
    ));
    out.push_str(&line(
        "interface_contracts, repo_inventory, monorepo_pins, org_ci_status, site_check",
        gh,
        "site_check is always live; the GitHub API needs GITHUB_TOKEN for private repos/limits",
    ));
    out.push_str(&line(
        "cloudflare_zones, cloudflare_dns_records",
        cf,
        "needs CLOUDFLARE_API_TOKEN (Zone:Read + DNS:Read)",
    ));
    out.push_str(&line(
        "fiducia_status",
        fiducia,
        "needs FIDUCIA_URL + FIDUCIA_TOKEN",
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &'static str, h: Health) -> Check {
        Check {
            name,
            health: h,
            detail: String::new(),
        }
    }

    #[test]
    fn overall_reduces_correctly() {
        assert_eq!(
            overall(&[c("a", Health::Green), c("b", Health::Unknown)]),
            Health::Green
        );
        assert_eq!(
            overall(&[c("a", Health::Green), c("b", Health::Degraded)]),
            Health::Degraded
        );
        assert_eq!(
            overall(&[c("a", Health::Red), c("b", Health::Degraded)]),
            Health::Red
        );
        assert_eq!(overall(&[]), Health::Green);
    }

    #[test]
    fn rollup_names_failing_and_unknown_checks() {
        let checks = vec![
            Check {
                name: "apex_dns",
                health: Health::Green,
                detail: "ok".into(),
            },
            Check {
                name: "backend_health",
                health: Health::Red,
                detail: "unreachable".into(),
            },
            Check {
                name: "org_ci",
                health: Health::Unknown,
                detail: "no token".into(),
            },
        ];
        let s = format_rollup(&checks);
        assert!(s.contains("File Tunnel stack status: RED"));
        assert!(s.contains("failing checks: backend_health"));
        assert!(s.contains("unknown (missing credential/tool"));
        assert!(s.contains("org_ci"));
    }

    #[test]
    fn capabilities_report_lists_families_without_leaking() {
        let s = capabilities_report();
        assert!(s.contains("SUPABASE_SERVICE_ROLE_KEY"));
        assert!(s.contains("fiducia_status"));
        assert!(s.contains("transfer_api_health"));
        assert!(s.contains("interface_contracts"));
        // presence-only: the words present/MISSING, never a value
        assert!(s.contains("present") || s.contains("MISSING"));
    }
}
