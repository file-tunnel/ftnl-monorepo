//! The MCP tool surface. Thin wrappers over the util/builds/transfer/github/
//! interfaces/domains/cloudflare/supabase/docs modules so the logic stays
//! unit-testable outside the MCP plumbing. Every tool is read-only or build-only.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    GetPromptRequestParams, GetPromptResult, Implementation, ListPromptsResult,
    ListResourcesResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
    ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents, Role,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::status::{self, Check, Health};
use crate::util::{self, git, run_cmd, safe_segment, truncate_output};
use crate::{
    builds, cloudflare, docs, domains, fiducia, github, interfaces, prompts, resources, supabase,
    transfer,
};

#[derive(Deserialize, JsonSchema)]
pub struct RepoReq {
    /// Repo directory name under the org root, e.g. "ftnl-backend-api.rs".
    repo: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RecentCommitsReq {
    /// Repo directory name under the org root.
    repo: String,
    /// How many commits to show (default 15).
    count: Option<u32>,
    /// Branch or ref to log (default: current HEAD).
    branch: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchCodeReq {
    /// Extended regex passed to `git grep -E` (tracked files only).
    pattern: String,
    /// Restrict to one repo (directory name). Default: all git repos in the org.
    repo: Option<String>,
    /// Cap on total matching lines returned (default 200).
    max_matches: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CargoReq {
    /// Repo directory name under the org root, e.g. "ftnl-backend-api.rs".
    repo: String,
    /// Pass `--locked` (fail if Cargo.lock would change). Default false.
    locked: Option<bool>,
    /// Timeout in seconds (default 600, max 1800).
    timeout_secs: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ApiProbeReq {
    /// Base URL of the backend API. Default: FTNL_API_URL or
    /// http://127.0.0.1:8080 (the reference server's local bind).
    base_url: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DnsLookupReq {
    /// Domain name to resolve, e.g. "file-tunnel.github.io".
    domain: String,
    /// Record type (A, AAAA, CNAME, MX, TXT, NS, …). Default: a sweep.
    record_type: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DomainInfoReq {
    /// Registrable domain, e.g. "file-tunnel.app" (not a subdomain).
    domain: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct TlsCertReq {
    /// Hostname to check, e.g. "file-tunnel.github.io".
    host: String,
    /// Port (default 443).
    port: Option<u16>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CfRecordsReq {
    /// Zone name, e.g. "file-tunnel.app".
    zone: String,
    /// Filter by record type (A, CNAME, TXT, …).
    record_type: Option<String>,
    /// Filter by exact record name, e.g. "portal.file-tunnel.app".
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LogSessionsReq {
    /// Filter by client environment (e.g. "production", "staging", "dev").
    environment: Option<String>,
    /// Max snapshots to return (default 30, max 200).
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TailLogsReq {
    /// The session_id whose entries to tail (from client_log_sessions).
    session_id: String,
    /// Filter to one level (error, warn, info, debug, …).
    level: Option<String>,
    /// Max entries to return (default 100, max 500).
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ErrorSummaryReq {
    /// Filter by client environment.
    environment: Option<String>,
    /// Aggregate by "message" (default), "url"/"route", or "category".
    group_by: Option<String>,
    /// How many recent error/warn entries to scan (default 500, max 1000).
    scan_limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TraceReq {
    /// The session_id whose full ordered timeline to pull (from client_log_sessions).
    session_id: String,
    /// Max entries to return, oldest first (default 300, max 1000).
    limit: Option<u32>,
}

#[derive(Clone)]
pub struct FtnlMcp {
    pub root: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl FtnlMcp {
    pub fn new() -> Self {
        Self {
            root: util::org_root(),
            tool_router: crate::telemetry::instrument_tool_router(Self::tool_router()),
        }
    }

    fn repo_path(&self, name: &str) -> Result<PathBuf, String> {
        safe_segment(name, "repo")?;
        let p = self.root.join(name);
        if !p.is_dir() {
            return Err(format!(
                "no such repo dir {:?} under {} — use org_overview to list local checkouts, \
                 or org_map / repo_inventory for the full org inventory",
                name,
                self.root.display()
            ));
        }
        Ok(p)
    }

    fn git_repos(&self) -> Vec<String> {
        let mut repos: Vec<String> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join(".git").exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        repos.sort();
        repos
    }
}

impl Default for FtnlMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl FtnlMcp {
    // ------------------------------------------------------------- org / git

    #[tool(
        description = "Overview of every git repo checked out under the file-tunnel org root: branch, dirty file count, ahead/behind upstream, last commit. Local checkouts may be a subset of the org — use org_map or repo_inventory for the full inventory. Start here to orient."
    )]
    async fn org_overview(&self) -> Result<String, String> {
        let repos = self.git_repos();
        let mut out = format!("org root: {}\n", self.root.display());
        if repos.is_empty() {
            out.push_str(
                "\n(no git checkouts here yet — see org_map / repo_inventory for the full file-tunnel repo list)\n",
            );
            return Ok(out);
        }
        out.push('\n');
        for name in &repos {
            let dir = self.root.join(name);
            let branch = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])
                .await
                .map(|(_, s)| s.trim().to_string())
                .unwrap_or_else(|e| format!("<{e}>"));
            let dirty = git(&dir, &["status", "--porcelain"])
                .await
                .map(|(_, s)| s.lines().count())
                .unwrap_or(0);
            let last = git(&dir, &["log", "-1", "--format=%h %ad %s", "--date=short"])
                .await
                .map(|(_, s)| s.trim().to_string())
                .unwrap_or_default();
            let upstream = match git(
                &dir,
                &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
            )
            .await
            {
                Ok((true, s)) => {
                    let mut it = s.split_whitespace();
                    let behind = it.next().unwrap_or("0");
                    let ahead = it.next().unwrap_or("0");
                    format!("ahead {ahead}, behind {behind}")
                }
                _ => "no upstream".to_string(),
            };
            out.push_str(&format!(
                "## {name}\n  branch: {branch} ({upstream})\n  dirty files: {dirty}\n  last commit: {last}\n\n"
            ));
        }
        Ok(truncate_output(out))
    }

    #[tool(
        description = "Detailed git status for one repo under the org root: branch, upstream delta, short status, recent commits, worktrees, and stashes."
    )]
    async fn repo_status(&self, Parameters(req): Parameters<RepoReq>) -> Result<String, String> {
        let dir = self.repo_path(&req.repo)?;
        let mut out = String::new();
        for (title, args) in [
            ("branch", vec!["rev-parse", "--abbrev-ref", "HEAD"]),
            ("status", vec!["status", "--short", "--branch"]),
            (
                "recent commits",
                vec!["log", "-8", "--format=%h %ad %an %s", "--date=short"],
            ),
            ("worktrees", vec!["worktree", "list"]),
            ("stashes", vec!["stash", "list"]),
        ] {
            let (_, text) = git(&dir, &args).await?;
            let text = text.trim();
            if !text.is_empty() {
                out.push_str(&format!("## {title}\n{text}\n\n"));
            }
        }
        Ok(truncate_output(out))
    }

    #[tool(description = "Show recent commits for a repo under the org root (git log).")]
    async fn recent_commits(
        &self,
        Parameters(req): Parameters<RecentCommitsReq>,
    ) -> Result<String, String> {
        let dir = self.repo_path(&req.repo)?;
        let n = req.count.unwrap_or(15).min(200).to_string();
        let mut args = vec![
            "log",
            "--format=%h %ad %an %s",
            "--date=short",
            "-n",
            n.as_str(),
        ];
        if let Some(branch) = req.branch.as_deref() {
            util::safe_token(branch, "branch")?;
            args.push(branch);
        }
        let (ok, text) = git(&dir, &args).await?;
        if !ok {
            return Err(text);
        }
        Ok(truncate_output(text))
    }

    #[tool(
        description = "Search tracked source across the org (or one repo) with `git grep -E`. Use this instead of raw grep — it skips build/ and target/ dirs."
    )]
    async fn search_code(
        &self,
        Parameters(req): Parameters<SearchCodeReq>,
    ) -> Result<String, String> {
        let repos = match req.repo.as_deref() {
            Some(r) => {
                self.repo_path(r)?;
                vec![r.to_string()]
            }
            None => self.git_repos(),
        };
        if repos.is_empty() {
            return Ok("no git checkouts under the org root to search".to_string());
        }
        let max = req.max_matches.unwrap_or(200) as usize;
        let mut lines_out: Vec<String> = Vec::new();
        let mut hit_repos = 0usize;
        for name in &repos {
            if lines_out.len() >= max {
                break;
            }
            let dir = self.root.join(name);
            let (ok, text) = run_cmd(
                Some(&dir),
                "git",
                &["grep", "-nEI", "-e", &req.pattern, "--", ":!*.lock"],
                Duration::from_secs(60),
            )
            .await?;
            if !ok {
                continue; // exit 1 = no matches in this repo
            }
            hit_repos += 1;
            for l in text.lines() {
                if lines_out.len() >= max {
                    lines_out.push(format!("…[capped at {max} matches]"));
                    break;
                }
                lines_out.push(format!("{name}/{l}"));
            }
        }
        if lines_out.is_empty() {
            return Ok(format!(
                "no matches for {:?} in {} repo(s)",
                req.pattern,
                repos.len()
            ));
        }
        Ok(truncate_output(format!(
            "matches in {hit_repos} repo(s):\n{}",
            lines_out.join("\n")
        )))
    }

    // ------------------------------------------------------ cargo (build-only)

    #[tool(
        description = "Build a Rust repo under the org root with `cargo build` (build-only; touches only that repo's target/ cache). Set FTNL_CARGO_OFFLINE=1 to pass --offline. Errors clearly if the `cargo` CLI is not installed."
    )]
    async fn cargo_build(&self, Parameters(req): Parameters<CargoReq>) -> Result<String, String> {
        self.run_cargo(&req, "build").await
    }

    #[tool(
        description = "Type-check a Rust repo under the org root with `cargo check` (build-only, faster than build). Set FTNL_CARGO_OFFLINE=1 to pass --offline. Errors clearly if the `cargo` CLI is not installed."
    )]
    async fn cargo_check(&self, Parameters(req): Parameters<CargoReq>) -> Result<String, String> {
        self.run_cargo(&req, "check").await
    }

    // ------------------------------------------- tunnel / transfer backend API

    #[tool(
        description = "Probe the File Tunnel backend API's unauthenticated GET /healthz. Base URL defaults to FTNL_API_URL or http://127.0.0.1:8080 (ftnl-backend-api.rs's local bind; port-forward the in-cluster service or run it locally first). Read-only."
    )]
    async fn transfer_api_health(
        &self,
        Parameters(req): Parameters<ApiProbeReq>,
    ) -> Result<String, String> {
        let base = req.base_url.unwrap_or_else(transfer::api_base_url);
        transfer::health(&base).await
    }

    #[tool(
        description = "Map the tunnel/transfer API surface: live-probe the unauthenticated /healthz route and list the capability-scoped tunnel lifecycle routes (create/claim/declare/upload/download/event-tickets/events/cancel), which fail closed without a desktop/phone capability or a one-time event ticket. Read-only — never creates a tunnel or moves bytes."
    )]
    async fn tunnel_api_routes(
        &self,
        Parameters(req): Parameters<ApiProbeReq>,
    ) -> Result<String, String> {
        let base = req.base_url.unwrap_or_else(transfer::api_base_url);
        transfer::routes(&base).await
    }

    #[tool(
        description = "Explain the File Tunnel architecture: the two-capability model (desktop observes/downloads, phone declares/uploads), the HTTP control/data plane routes, ticket-authenticated realtime events, and the local-first sync state machine. Embedded doc — no network."
    )]
    async fn architecture_docs(&self) -> Result<String, String> {
        Ok(docs::ARCHITECTURE.to_string())
    }

    // ---------------------------------- interfaces / inventory / monorepo / CI

    #[tool(
        description = "List the canonical File Tunnel contracts published by ftnl-interfaces (OpenAPI, AsyncAPI, JSON Schema, cross-language fixtures, and generated Rust/TS/Dart/Gleam snapshots) via the GitHub contents API. Set GITHUB_TOKEN/GH_TOKEN for higher rate limits."
    )]
    async fn interface_contracts(&self) -> Result<String, String> {
        let gh = github::GitHubClient::new()?;
        interfaces::contracts(&gh).await
    }

    #[tool(
        description = "Release/repository inventory for the whole file-tunnel org: per repo, its description, visibility, default branch, last push, and latest release tag. Uses the GitHub API; set GITHUB_TOKEN/GH_TOKEN for private repos + higher limits."
    )]
    async fn repo_inventory(&self) -> Result<String, String> {
        let gh = github::GitHubClient::new()?;
        interfaces::inventory(&gh).await
    }

    #[tool(
        description = "Read the ftnl-monorepo superproject's submodule pins from its .gitmodules (each deployable repo is tracked under apps/). Uses the GitHub contents API; set GITHUB_TOKEN/GH_TOKEN to reach the private monorepo."
    )]
    async fn monorepo_pins(&self) -> Result<String, String> {
        let gh = github::GitHubClient::new()?;
        let raw = gh
            .get_raw(&format!(
                "/repos/{}/{}/contents/.gitmodules",
                github::ORG,
                github::MONOREPO
            ))
            .await?;
        let mods = github::parse_gitmodules(&raw);
        if mods.is_empty() {
            return Ok(format!("{} has no submodules declared", github::MONOREPO));
        }
        let mut out = format!("{}/{} submodules:\n", github::ORG, github::MONOREPO);
        for m in &mods {
            out.push_str(&format!(
                "  {}\n    path: {}\n    url:  {}\n    branch: {}\n",
                m.name,
                m.path,
                m.url,
                m.branch.as_deref().unwrap_or("(default)")
            ));
        }
        out.push_str("\n(pins follow each repo's default branch)\n");
        Ok(truncate_output(out))
    }

    #[tool(
        description = "Latest GitHub Actions run per non-Pages file-tunnel repo: workflow, branch, status/conclusion, and update time. Set GITHUB_TOKEN/GH_TOKEN for private repos and a higher rate limit."
    )]
    async fn org_ci_status(&self) -> Result<String, String> {
        let gh = github::GitHubClient::new()?;
        let mut out = String::from("file-tunnel CI (latest run per repo):\n");
        for repo in github::CI_REPOS {
            let path = format!("/repos/{}/{repo}/actions/runs?per_page=1", github::ORG);
            match gh.get_json(&path).await {
                Ok(body) => out.push_str(&github::summarize_latest_run(repo, &body)),
                Err(e) => out.push_str(&format!("  {repo}: <{e}>",)),
            }
            out.push('\n');
        }
        Ok(truncate_output(out))
    }

    #[tool(
        description = "Check the org's GitHub Pages marketing site (https://file-tunnel.github.io): HTTP status, whether it looks like the File Tunnel landing page, and the served title. Read-only GET."
    )]
    async fn site_check(&self) -> Result<String, String> {
        let client = domains::http_client()?;
        let resp = client
            .get(github::SITE_URL)
            .send()
            .await
            .map_err(|e| format!("GET {} failed: {e}", github::SITE_URL))?;
        let status = resp.status();
        let body = util::read_text_capped(resp).await?;
        let title = body
            .split_once("<title>")
            .and_then(|(_, r)| r.split_once("</title>"))
            .map(|(t, _)| t.trim().to_string())
            .unwrap_or_else(|| "(no <title>)".to_string());
        let looks_ftnl =
            body.to_lowercase().contains("file tunnel") || body.to_lowercase().contains("tunnel");
        Ok(format!(
            "GET {} → HTTP {status}\n  title: {title}\n  looks like the File Tunnel site: {}",
            github::SITE_URL,
            if looks_ftnl { "yes" } else { "no" }
        ))
    }

    // ---------------------------------------------------------------- domains

    #[tool(
        description = "Resolve DNS records via DNS-over-HTTPS (Cloudflare 1.1.1.1). Registrar-agnostic. Omit record_type for an A/AAAA/CNAME/NS/MX/TXT sweep."
    )]
    async fn dns_lookup(
        &self,
        Parameters(req): Parameters<DnsLookupReq>,
    ) -> Result<String, String> {
        let types: Vec<String> = match req.record_type {
            Some(t) => vec![t.to_uppercase()],
            None => ["A", "AAAA", "CNAME", "NS", "MX", "TXT"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let mut out = format!("DNS for {} (via 1.1.1.1 DoH):\n", req.domain);
        for t in &types {
            let v = domains::doh_query(&req.domain, t).await?;
            out.push_str(&format!("## {t}\n{}\n", domains::format_doh(t, &v)));
        }
        Ok(truncate_output(out))
    }

    #[tool(
        description = "Registration + hosting info via RDAP: registrar (e.g. Squarespace, Cloudflare), expiry with days-remaining, status flags, and nameservers classified by DNS host. The canonical way to inspect Squarespace-registered org domains (no public DNS API)."
    )]
    async fn domain_info(
        &self,
        Parameters(req): Parameters<DomainInfoReq>,
    ) -> Result<String, String> {
        let v = domains::rdap_lookup(&req.domain).await?;
        let mut out = domains::summarize_rdap(&v);
        if let Ok(ns) = domains::doh_query(&req.domain, "NS").await {
            out.push_str(&format!(
                "\nlive NS (per 1.1.1.1):\n{}\n",
                domains::format_doh("NS", &ns)
            ));
        }
        Ok(truncate_output(out))
    }

    #[tool(
        description = "Check the TLS certificate served at host:port (default 443): subject, issuer, notAfter, and days until expiry."
    )]
    async fn tls_cert_check(
        &self,
        Parameters(req): Parameters<TlsCertReq>,
    ) -> Result<String, String> {
        domains::tls_cert_check(&req.host, req.port.unwrap_or(443)).await
    }

    #[tool(
        description = "List Cloudflare zones visible to CLOUDFLARE_API_TOKEN (read-only; scope the token Zone:Read + DNS:Read). Cloudflare proxies the File Tunnel sites."
    )]
    async fn cloudflare_zones(&self) -> Result<String, String> {
        let body = cloudflare::api_get("/zones", &[("per_page", "50")]).await?;
        Ok(truncate_output(format!(
            "Cloudflare zones:\n{}",
            cloudflare::format_zones(&body)
        )))
    }

    #[tool(
        description = "List DNS records in a Cloudflare zone (by zone name), optionally filtered by type and/or exact record name. Requires CLOUDFLARE_API_TOKEN."
    )]
    async fn cloudflare_dns_records(
        &self,
        Parameters(req): Parameters<CfRecordsReq>,
    ) -> Result<String, String> {
        // Validate every user-supplied input before any network I/O.
        util::safe_hostname(&req.zone)?;
        let rtype = req.record_type.as_deref().map(str::to_uppercase);
        if let Some(t) = rtype.as_deref() {
            util::safe_record_type(t)?;
        }
        if let Some(n) = req.name.as_deref() {
            util::safe_hostname(n)?;
        }
        let zone_id = cloudflare::zone_id_by_name(&req.zone).await?;
        let mut query: Vec<(&str, &str)> = vec![("per_page", "100")];
        if let Some(t) = rtype.as_deref() {
            query.push(("type", t));
        }
        if let Some(n) = req.name.as_deref() {
            query.push(("name", n));
        }
        let body = cloudflare::api_get(&format!("/zones/{zone_id}/dns_records"), &query).await?;
        Ok(truncate_output(format!(
            "DNS records in zone {}:\n{}",
            req.zone,
            cloudflare::format_records(&body)
        )))
    }

    // --------------------------------------------- supabase client telemetry

    #[tool(
        description = "List recent client-telemetry sessions from ftnl_client_log_snapshots (the upload portal / web UI / Flutter clients stream their redacted log ring buffer straight into Supabase). Filter by environment. Needs SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY."
    )]
    async fn client_log_sessions(
        &self,
        Parameters(req): Parameters<LogSessionsReq>,
    ) -> Result<String, String> {
        supabase::sessions(req.environment.as_deref(), req.limit.unwrap_or(30)).await
    }

    #[tool(
        description = "Tail individual client log entries for one session_id from ftnl_client_log_entries (newest first), optionally filtered to a level. Needs SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY."
    )]
    async fn tail_client_logs(
        &self,
        Parameters(req): Parameters<TailLogsReq>,
    ) -> Result<String, String> {
        supabase::tail(
            &req.session_id,
            req.level.as_deref(),
            req.limit.unwrap_or(100),
        )
        .await
    }

    #[tool(
        description = "Summarize recent client error/warn entries grouped (by message | url/route | category), most frequent first — a fast triage of what's breaking in the portal / web / Flutter clients. Needs SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY."
    )]
    async fn client_error_summary(
        &self,
        Parameters(req): Parameters<ErrorSummaryReq>,
    ) -> Result<String, String> {
        let group = match req.group_by.as_deref() {
            Some(g) => supabase::GroupBy::parse(g)?,
            None => supabase::GroupBy::Message,
        };
        supabase::error_summary(
            req.environment.as_deref(),
            group,
            req.scan_limit.unwrap_or(500),
        )
        .await
    }

    #[tool(
        description = "Pull one session's FULL ordered entry timeline (oldest → newest) from ftnl_client_log_entries — the client_log_trace for reconstructing what a client did before it broke. Bounded. Needs SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY."
    )]
    async fn client_log_trace(
        &self,
        Parameters(req): Parameters<TraceReq>,
    ) -> Result<String, String> {
        supabase::trace(&req.session_id, req.limit.unwrap_or(300)).await
    }

    #[tool(
        description = "Explain the org's telemetry pattern: CLIENT logs (upload portal / web via supabase-js, Dart/Flutter via supabase_flutter) stream DIRECTLY to Supabase RPCs/tables (redacted — never secrets/capabilities/tickets/bytes), while the BACKEND emits structured stdout (Promtail/Loki) + OTLP. Embedded doc — no network."
    )]
    async fn telemetry_docs(&self) -> Result<String, String> {
        Ok(docs::TELEMETRY_DOCS.to_string())
    }

    // ------------------------------------------------------------------ docs

    #[tool(
        description = "Embedded org/architecture map for file-tunnel: repos, the capability model, the tunnel lifecycle, the backend (:8080) + portal (:3000), the sync boundary, and shared conventions (Nix, formal methods, dpm migrations, Cloudflare DNS). No network."
    )]
    async fn org_map(&self) -> Result<String, String> {
        Ok(docs::ORG_MAP.to_string())
    }

    // ------------------------------------------------------ ops / aggregate

    #[tool(
        description = "FLAGSHIP readiness rollup for the whole file-tunnel stack: cargo toolchain (local), backend API GET /healthz (network), DNS for the github.io Pages site, the marketing site itself, and latest ftnl-backend-api.rs CI (GITHUB_TOKEN). Returns a GREEN/DEGRADED/RED verdict naming every failing check. Checks needing an absent credential/tool are UNKNOWN, not RED. Bounded, read-only, every network step is timed out."
    )]
    async fn stack_status(&self) -> Result<String, String> {
        let mut checks: Vec<Check> = Vec::new();

        // 1) cargo toolchain (local, fast) — represents the build family.
        let (h, detail) =
            match run_cmd(None, "cargo", &["--version"], Duration::from_secs(10)).await {
                Ok((true, v)) => (Health::Green, v.trim().to_string()),
                Ok((false, _)) => (
                    Health::Degraded,
                    "`cargo --version` returned non-zero".to_string(),
                ),
                Err(_) => (
                    Health::Unknown,
                    "cargo CLI not on PATH — cargo_build/cargo_check are degraded".to_string(),
                ),
            };
        checks.push(Check {
            name: "cargo_toolchain",
            health: h,
            detail,
        });

        // 2) backend API /healthz (network)
        let base = transfer::api_base_url();
        let (h, detail) = match transfer::health(&base).await {
            Ok(s) if s.contains("HTTP 200") => (Health::Green, "backend /healthz 200".to_string()),
            Ok(s) => (
                Health::Degraded,
                s.lines().next().unwrap_or("non-200").to_string(),
            ),
            Err(e) => (
                Health::Red,
                format!("backend unreachable: {}", e.lines().next().unwrap_or("?")),
            ),
        };
        checks.push(Check {
            name: "backend_health",
            health: h,
            detail,
        });

        // 3) DNS for the github.io Pages site
        let (h, detail) = match domains::doh_query("file-tunnel.github.io", "A").await {
            Ok(v) => {
                let n = v
                    .get("Answer")
                    .and_then(|a| a.as_array())
                    .map_or(0, Vec::len);
                if n > 0 {
                    (
                        Health::Green,
                        format!("file-tunnel.github.io has {n} A record(s)"),
                    )
                } else {
                    (
                        Health::Degraded,
                        "file-tunnel.github.io has no A records".to_string(),
                    )
                }
            }
            Err(e) => (Health::Red, format!("DoH query failed: {e}")),
        };
        checks.push(Check {
            name: "pages_dns",
            health: h,
            detail,
        });

        // 4) marketing site (network)
        let (h, detail) = match domains::http_client()?.get(github::SITE_URL).send().await {
            Ok(r) if r.status().is_success() => (
                Health::Green,
                format!("{} HTTP {}", github::SITE_URL, r.status()),
            ),
            Ok(r) => (
                Health::Degraded,
                format!("{} HTTP {}", github::SITE_URL, r.status()),
            ),
            Err(e) => (
                Health::Red,
                format!("{} unreachable: {e}", github::SITE_URL),
            ),
        };
        checks.push(Check {
            name: "site_health",
            health: h,
            detail,
        });

        // 5) latest ftnl-backend-api.rs CI (needs GITHUB_TOKEN for private repos)
        let (h, detail) =
            if std::env::var("GITHUB_TOKEN").is_err() && std::env::var("GH_TOKEN").is_err() {
                (
                    Health::Unknown,
                    "GITHUB_TOKEN unset — cannot read private-repo CI".to_string(),
                )
            } else {
                match github::GitHubClient::new()?
                    .get_json(&format!(
                        "/repos/{}/ftnl-backend-api.rs/actions/runs?per_page=1",
                        github::ORG
                    ))
                    .await
                {
                    Ok(body) => match body
                        .pointer("/workflow_runs/0/conclusion")
                        .and_then(|c| c.as_str())
                    {
                        Some("success") => (
                            Health::Green,
                            "ftnl-backend-api.rs latest run: success".to_string(),
                        ),
                        Some(other) => (
                            Health::Red,
                            format!("ftnl-backend-api.rs latest run: {other}"),
                        ),
                        None => (
                            Health::Unknown,
                            "ftnl-backend-api.rs: no completed run".to_string(),
                        ),
                    },
                    Err(e) => (Health::Unknown, format!("CI query failed: {e}")),
                }
            };
        checks.push(Check {
            name: "backend_ci",
            health: h,
            detail,
        });

        Ok(truncate_output(status::format_rollup(&checks)))
    }

    #[tool(
        description = "Self-test: report which env vars/creds are present and therefore which tool families are fully LIVE vs DEGRADED (org root, backend URL, Supabase, GitHub, Cloudflare, fiducia). Presence only — never prints a secret value. Local, no network."
    )]
    async fn self_test(&self) -> Result<String, String> {
        Ok(truncate_output(status::capabilities_report()))
    }

    #[tool(
        description = "Check the org's fiducia.cloud secrets + locks/leases plane (read-only): whether this org's required secrets are present and whether leases look healthy. Needs FIDUCIA_URL + FIDUCIA_TOKEN (optional FIDUCIA_REQUIRED_SECRETS). Never fetches secret values; graceful typed error when unconfigured."
    )]
    async fn fiducia_status(&self) -> Result<String, String> {
        let env = fiducia::env()?;
        fiducia::status_report(&env).await
    }
}

impl FtnlMcp {
    /// Shared cargo runner used by cargo_build / cargo_check.
    async fn run_cargo(&self, req: &CargoReq, subcommand: &str) -> Result<String, String> {
        let dir = self.repo_path(&req.repo)?;
        let owned = builds::cargo_args(
            subcommand,
            req.locked.unwrap_or(false),
            builds::offline_from_env(),
        )?;
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(600).min(1800));
        let (ok, text) = run_cmd(Some(&dir), "cargo", &args, timeout)
            .await
            .map_err(|e| {
                if e.contains("not found on PATH") {
                    format!(
                        "{e}\ninstall the Rust toolchain (https://rustup.rs) — this tool only \
                         runs the cargo CLI, it does not vendor a toolchain"
                    )
                } else {
                    e
                }
            })?;
        let mut out = format!(
            "cargo {} in {}: {}\n\n",
            args.join(" "),
            req.repo,
            if ok { "OK" } else { "FAILED" }
        );
        out.push_str(&builds::filter_diagnostics(&text));
        Ok(truncate_output(out))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FtnlMcp {
    async fn list_resources(
        &self,
        _req: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let items = resources::all()
            .into_iter()
            .map(|r| {
                Resource::new(r.uri, r.name)
                    .with_title(r.title)
                    .with_description(r.description)
                    .with_mime_type(r.mime)
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(items))
    }

    async fn read_resource(
        &self,
        req: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let r = resources::get(&req.uri).ok_or_else(|| {
            McpError::resource_not_found(
                format!("unknown resource {:?} — see resources/list", req.uri),
                None,
            )
        })?;
        let contents = ResourceContents::text(r.body, r.uri).with_mime_type(r.mime);
        Ok(ReadResourceResult::new(vec![contents]))
    }

    async fn list_prompts(
        &self,
        _req: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let items = prompts::all()
            .into_iter()
            .map(|p| {
                let args: Vec<PromptArgument> = p
                    .args
                    .iter()
                    .map(|a| {
                        PromptArgument::new(a.name)
                            .with_description(a.description)
                            .with_required(a.required)
                    })
                    .collect();
                Prompt::new(p.name, Some(p.description), Some(args)).with_title(p.title)
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(items))
    }

    async fn get_prompt(
        &self,
        req: GetPromptRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let args: BTreeMap<String, String> = req
            .arguments
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, s)
            })
            .collect();
        let (description, text) =
            prompts::render(&req.name, &args).map_err(|e| McpError::invalid_params(e, None))?;
        Ok(
            GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
                .with_description(description),
        )
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Tools for the File Tunnel org — secure, ephemeral, QR-connected cross-device file \
             transfers (a desktop upload field paired to files on a phone). Start with org_map \
             (embedded inventory) or org_overview (local checkouts). cargo_build/cargo_check run \
             the cargo CLI build-only in a repo under FTNL_ROOT (~/codes/file-tunnel). \
             transfer_api_health/tunnel_api_routes probe the backend API (default \
             http://127.0.0.1:8080 = ftnl-backend-api.rs); architecture_docs documents the \
             capability model + lifecycle. interface_contracts lists the ftnl-interfaces \
             OpenAPI/AsyncAPI; repo_inventory rolls up repos+releases; monorepo_pins/org_ci_status/\
             site_check use the GitHub API (GITHUB_TOKEN for private repos). Domain tools use \
             DoH/RDAP/TLS + Cloudflare (CLOUDFLARE_API_TOKEN). client_log_sessions/\
             tail_client_logs/client_log_trace/client_error_summary read client telemetry from \
             Supabase (SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY); telemetry_docs explains the \
             pattern (secrets/capabilities/tickets/bytes are never logged). stack_status gives a \
             GREEN/DEGRADED/RED readiness rollup; self_test shows which creds are configured; \
             fiducia_status checks the shared secrets/leases plane (FIDUCIA_URL + FIDUCIA_TOKEN). \
             Embedded knowledge is also exposed as MCP resources (orgmap://file-tunnel, \
             docs://architecture, docs://telemetry, schema://client-telemetry) and prompts \
             (deploy_readiness, triage_client_errors, domain_audit, transfer_api_review). All \
             tools are read-only or build-only.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_exposes_all_tools() {
        let router = FtnlMcp::tool_router();
        for tool in [
            "org_overview",
            "repo_status",
            "recent_commits",
            "search_code",
            "cargo_build",
            "cargo_check",
            "transfer_api_health",
            "tunnel_api_routes",
            "architecture_docs",
            "interface_contracts",
            "repo_inventory",
            "monorepo_pins",
            "org_ci_status",
            "site_check",
            "dns_lookup",
            "domain_info",
            "tls_cert_check",
            "cloudflare_zones",
            "cloudflare_dns_records",
            "client_log_sessions",
            "tail_client_logs",
            "client_error_summary",
            "client_log_trace",
            "telemetry_docs",
            "org_map",
            "stack_status",
            "self_test",
            "fiducia_status",
        ] {
            assert!(router.has_route(tool), "missing tool {tool}");
        }
        assert_eq!(router.list_all().len(), 28);
    }
}
