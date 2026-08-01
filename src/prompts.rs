//! MCP prompts: canned, parameterized workflows for the File Tunnel org. Each
//! prompt expands to a single user message that tells the agent which of this
//! server's tools to run and how to read the result. They keep the platform's
//! load-bearing facts (the :8080 backend, capability model, one-time pairing
//! secrets/event tickets, client→Supabase telemetry) front and center.

use std::collections::BTreeMap;

pub struct PromptArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

pub struct PromptDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub args: &'static [PromptArg],
}

pub fn all() -> Vec<PromptDef> {
    vec![
        PromptDef {
            name: "deploy_readiness",
            title: "Deploy readiness rollup",
            description: "Decide whether the File Tunnel stack is safe to deploy: aggregate \
                          the cargo toolchain, backend API health, apex DNS, marketing site, \
                          and org CI into a GREEN/DEGRADED/RED verdict.",
            args: &[PromptArg {
                name: "environment",
                description: "Target environment to reason about (e.g. production). Optional.",
                required: false,
            }],
        },
        PromptDef {
            name: "triage_client_errors",
            title: "Triage client telemetry",
            description: "Investigate a spike of client-side errors reported through the \
                          Supabase telemetry tables and trace the worst session's timeline.",
            args: &[
                PromptArg {
                    name: "environment",
                    description:
                        "Filter to one client environment: production | staging | dev. Optional.",
                    required: false,
                },
                PromptArg {
                    name: "group_by",
                    description: "How to aggregate: message | url | category (default message).",
                    required: false,
                },
            ],
        },
        PromptDef {
            name: "domain_audit",
            title: "Domain & DNS audit",
            description: "Audit registration, DNS, TLS and Cloudflare records for a File Tunnel \
                          domain (Squarespace-registered, Cloudflare-served, GitHub Pages).",
            args: &[PromptArg {
                name: "domain",
                description: "Registrable domain to audit (default file-tunnel.github.io).",
                required: false,
            }],
        },
        PromptDef {
            name: "transfer_api_review",
            title: "Transfer/tunnel API review",
            description: "Probe the live backend API and review its health, unauthenticated \
                          route surface, and the capability-scoped tunnel lifecycle contract.",
            args: &[PromptArg {
                name: "base_url",
                description: "Backend base URL to probe (default FTNL_API_URL or :8080).",
                required: false,
            }],
        },
    ]
}

pub fn get(name: &str) -> Option<PromptDef> {
    all().into_iter().find(|p| p.name == name)
}

fn arg<'a>(args: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    args.get(key).map(String::as_str).filter(|s| !s.is_empty())
}

/// Render a prompt to `(description, message_text)`. Unknown names error.
pub fn render(name: &str, args: &BTreeMap<String, String>) -> Result<(String, String), String> {
    let def = get(name).ok_or_else(|| format!("unknown prompt {name:?}"))?;
    let text = match name {
        "deploy_readiness" => {
            let env = arg(args, "environment").unwrap_or("the current target");
            format!(
                "Assess File Tunnel deploy readiness for {env}. Run `stack_status` for the \
                 GREEN/DEGRADED/RED rollup (cargo toolchain, backend /healthz, apex DNS, \
                 marketing site, org CI), then corroborate the failing checks with \
                 `org_ci_status` (latest Actions run per repo), `monorepo_pins` (stale \
                 submodule pins under apps/), `repo_inventory` (per-repo latest release/tag), \
                 and `tunnel_api_routes` (live backend surface). Name every failing check, \
                 decide GO / NO-GO, and call out anything only marked UNKNOWN because a \
                 credential (GITHUB_TOKEN, a reachable backend) is missing rather than \
                 actually failing."
            )
        }
        "triage_client_errors" => {
            let env = arg(args, "environment");
            let group = arg(args, "group_by").unwrap_or("message");
            let env_clause = match env {
                Some(e) => format!(" Filter to the {e} environment."),
                None => String::new(),
            };
            format!(
                "Triage File Tunnel client-side errors.{env_clause} Start with \
                 `client_error_summary` (group_by={group}) for the aggregated error/warn \
                 counts. Pick the noisiest session from `client_log_sessions`, then run \
                 `client_log_trace` on its session_id for the full ordered timeline (or \
                 `tail_client_logs` for a level filter). Remember these are the WASM/TS \
                 upload-portal/web (supabase-js) and Dart/Flutter (supabase_flutter) clients \
                 streaming straight to Supabase — not the backend, whose telemetry is \
                 stdout/OTel. Pairing secrets, capabilities, tickets, and file bytes are never \
                 in the telemetry, so reason from redacted metadata only. Summarize the likely \
                 root cause and which repo owns it. All reads need SUPABASE_URL + \
                 SUPABASE_SERVICE_ROLE_KEY."
            )
        }
        "domain_audit" => {
            let domain = arg(args, "domain").unwrap_or("file-tunnel.github.io");
            format!(
                "Audit the domain {domain}. Run `domain_info` (RDAP: registrar/expiry/NS — \
                 Squarespace typically holds registration, Cloudflare serves DNS), \
                 `dns_lookup` (live records via DoH), `tls_cert_check` (certificate expiry), \
                 and, if CLOUDFLARE_API_TOKEN is set, `cloudflare_dns_records` for {domain}. \
                 Flag near expiry (<30d), NS host mismatches, and missing/duplicate records."
            )
        }
        "transfer_api_review" => {
            let base = arg(args, "base_url");
            let base_clause = match base {
                Some(b) => format!(" against {b}"),
                None => String::new(),
            };
            format!(
                "Review the File Tunnel backend API's live surface{base_clause}. First confirm \
                 it is up with `transfer_api_health` (GET /healthz), then run \
                 `tunnel_api_routes` to enumerate the unauthenticated probe(s) and the \
                 capability-scoped tunnel lifecycle (create → claim → declare → upload → \
                 download → events → cancel). Cross-check the live surface against \
                 `interface_contracts` (the canonical OpenAPI/AsyncAPI in ftnl-interfaces). \
                 Remember the security invariants: a tunnel UUID is a routing identifier, not \
                 a credential; pairing secrets live in the URL fragment and are one-time; \
                 desktop and phone capabilities are separate; WebSockets use one-time event \
                 tickets. Flag any route reachable without a capability, and treat an \
                 unreachable backend (FTNL_API_URL unset / not port-forwarded) as UNKNOWN, \
                 not a failure."
            )
        }
        _ => return Err(format!("prompt {name:?} has no renderer")),
    };
    Ok((def.description.to_string(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn all_prompts_render_and_reference_real_tools() {
        for p in all() {
            let (_desc, text) = render(p.name, &BTreeMap::new()).expect(p.name);
            assert!(!text.is_empty());
        }
        assert!(render("deploy_readiness", &BTreeMap::new())
            .unwrap()
            .1
            .contains("stack_status"));
        assert!(render("triage_client_errors", &BTreeMap::new())
            .unwrap()
            .1
            .contains("client_log_trace"));
        assert!(render("domain_audit", &BTreeMap::new())
            .unwrap()
            .1
            .contains("domain_info"));
        assert!(render("transfer_api_review", &BTreeMap::new())
            .unwrap()
            .1
            .contains("tunnel_api_routes"));
    }

    #[test]
    fn parameters_are_interpolated_and_unknown_errors() {
        let (_d, t) = render(
            "domain_audit",
            &args(&[("domain", "file-tunnel.github.io")]),
        )
        .unwrap();
        assert!(t.contains("file-tunnel.github.io"));
        let (_d, t) = render(
            "triage_client_errors",
            &args(&[("environment", "production"), ("group_by", "url")]),
        )
        .unwrap();
        assert!(t.contains("production environment"));
        assert!(t.contains("group_by=url"));
        let (_d, t) = render(
            "transfer_api_review",
            &args(&[("base_url", "http://10.0.0.5:8080")]),
        )
        .unwrap();
        assert!(t.contains("http://10.0.0.5:8080"));
        assert!(render("bogus", &BTreeMap::new()).is_err());
    }
}
