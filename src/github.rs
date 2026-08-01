//! GitHub REST access for the File Tunnel org plus pure summarizers: latest CI
//! run per repo, the monorepo's submodule pins, and a github.io site liveness
//! check. Unauthenticated by default; sends a bearer token when `GITHUB_TOKEN`
//! or `GH_TOKEN` is set (raises the rate limit, reaches private repos). Network
//! access is confined to `GitHubClient`; everything that interprets JSON is a
//! pure function over `serde_json::Value`.

use serde_json::Value;

use crate::domains::http_client;

pub const ORG: &str = "file-tunnel";

/// Repos summarized by `org_ci_status` and `repo_inventory` (the org's non-Pages
/// repositories).
pub const CI_REPOS: &[&str] = &[
    "ftnl-backend-api.rs",
    "ftnl-web-server.rs",
    "ftnl-interfaces",
    "ftnl-sync",
    "ftnl-clients",
    "ftnl-ui-components",
    "ftnl-infra",
    "ftnl-e2e",
    "ftnl-mcp-server.rs",
    "ftnl-monorepo",
];

/// The superproject that pins each app repo as a git submodule under `apps/`.
pub const MONOREPO: &str = "ftnl-monorepo";

/// The org's GitHub Pages marketing site.
pub const SITE_URL: &str = "https://file-tunnel.github.io";

const API_ROOT: &str = "https://api.github.com";

pub struct GitHubClient {
    http: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub fn new() -> Result<Self, String> {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok()
            .filter(|t| !t.trim().is_empty());
        Ok(Self {
            http: http_client()?,
            token,
        })
    }

    async fn get(&self, path: &str, accept: &str) -> Result<(reqwest::StatusCode, String), String> {
        let url = format!("{API_ROOT}{path}");
        let mut req = self
            .http
            .get(&url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("GET {url} failed: {e}"))?;
        let status = resp.status();
        let body = crate::util::read_text_capped(resp)
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        Ok((status, body))
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        let (status, body) = self.get(path, "application/vnd.github+json").await?;
        if !status.is_success() {
            let snippet: String = body.chars().take(300).collect();
            return Err(format!("GET {path} returned {status}: {snippet}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("GET {path}: invalid JSON: {e}"))
    }

    /// GET returning `(status, parsed_json)` — lets callers treat a 404 as data
    /// (e.g. "no releases yet") instead of an error.
    pub async fn get_json_status(&self, path: &str) -> Result<(u16, Value), String> {
        let (status, body) = self.get(path, "application/vnd.github+json").await?;
        let value = serde_json::from_str(&body).unwrap_or(Value::Null);
        Ok((status.as_u16(), value))
    }

    /// Raw file contents through the contents API (used for `.gitmodules`).
    pub async fn get_raw(&self, path: &str) -> Result<String, String> {
        let (status, body) = self.get(path, "application/vnd.github.raw+json").await?;
        if !status.is_success() {
            let snippet: String = body.chars().take(300).collect();
            return Err(format!("GET {path} returned {status}: {snippet}"));
        }
        Ok(body)
    }
}

/// Summarize the newest workflow run from `GET .../actions/runs?per_page=1`.
pub fn summarize_latest_run(repo: &str, body: &Value) -> String {
    let run = body
        .get("workflow_runs")
        .and_then(Value::as_array)
        .and_then(|r| r.first());
    match run {
        Some(run) => {
            let f = |k: &str| run.get(k).and_then(Value::as_str).unwrap_or("?");
            let conclusion = run
                .get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or("in-progress");
            format!(
                "  {repo}: {} on {} → {}/{}  ({})",
                f("name"),
                f("head_branch"),
                f("status"),
                conclusion,
                f("updated_at")
            )
        }
        None => format!("  {repo}: (no workflow runs)"),
    }
}

/// One `[submodule "..."]` section from a `.gitmodules` file.
#[derive(Debug, PartialEq, Eq)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
}

/// Parse the sections out of a `.gitmodules` file.
pub fn parse_gitmodules(text: &str) -> Vec<Submodule> {
    let mut out: Vec<Submodule> = Vec::new();
    let mut cur: Option<Submodule> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("[submodule ") {
            if let Some(prev) = cur.take() {
                out.push(prev);
            }
            let name = rest.trim_end_matches(']').trim_matches('"').to_string();
            cur = Some(Submodule {
                name,
                path: String::new(),
                url: String::new(),
                branch: None,
            });
        } else if let Some(sm) = cur.as_mut() {
            if let Some(v) = line
                .strip_prefix("path =")
                .or_else(|| line.strip_prefix("path="))
            {
                sm.path = v.trim().to_string();
            } else if let Some(v) = line
                .strip_prefix("url =")
                .or_else(|| line.strip_prefix("url="))
            {
                sm.url = v.trim().to_string();
            } else if let Some(v) = line
                .strip_prefix("branch =")
                .or_else(|| line.strip_prefix("branch="))
            {
                sm.branch = Some(v.trim().to_string());
            }
        }
    }
    if let Some(prev) = cur.take() {
        out.push(prev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_latest_run_reads_first_run() {
        let body = json!({"workflow_runs": [
            {"name": "ci", "head_branch": "main", "status": "completed",
             "conclusion": "success", "updated_at": "2026-07-31T10:00:00Z"}
        ]});
        let s = summarize_latest_run("ftnl-backend-api.rs", &body);
        assert!(s.contains("ftnl-backend-api.rs"));
        assert!(s.contains("main"));
        assert!(s.contains("completed/success"));
        assert!(
            summarize_latest_run("x", &json!({"workflow_runs": []})).contains("no workflow runs")
        );
    }

    #[test]
    fn parse_gitmodules_extracts_pins() {
        let text = "\
[submodule \"apps/backend-api\"]
\tpath = apps/backend-api
\turl = https://github.com/file-tunnel/ftnl-backend-api.rs.git
";
        let mods = parse_gitmodules(text);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "apps/backend-api");
        assert_eq!(mods[0].path, "apps/backend-api");
        assert_eq!(
            mods[0].url,
            "https://github.com/file-tunnel/ftnl-backend-api.rs.git"
        );
        assert_eq!(mods[0].branch, None);
    }
}
