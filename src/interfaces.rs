//! Domain-specific inventory tools over the GitHub API:
//!
//! * `interface_contracts` — lists the canonical File Tunnel contracts published
//!   by `ftnl-interfaces` (OpenAPI, AsyncAPI, JSON Schema, fixtures, generated
//!   bindings). These are the shared source of truth for every service, portal,
//!   UI component, and SDK.
//! * `repo_inventory` — a release/repository roll-up of the whole `file-tunnel`
//!   org: description, visibility, default branch, last push, and latest release
//!   tag per repo.
//!
//! Network access is confined to the passed `GitHubClient`; the summarizers are
//! pure functions over `serde_json::Value`.

use serde_json::Value;

use crate::github::{self, GitHubClient};
use crate::util;

/// The interfaces repo directories that hold the canonical contracts.
pub const CONTRACT_DIRS: &[(&str, &str)] = &[
    ("openapi", "HTTP control + data plane (OpenAPI)"),
    ("asyncapi", "realtime event stream (AsyncAPI)"),
    ("schema", "runtime-validatable JSON Schema payloads"),
    ("fixtures", "cross-language contract vectors"),
    (
        "generated",
        "reviewable Rust/TypeScript/Dart/Gleam snapshots",
    ),
];

/// List the contract files under one `ftnl-interfaces` directory.
pub async fn contracts(gh: &GitHubClient) -> Result<String, String> {
    let mut out = format!(
        "canonical File Tunnel contracts ({}/ftnl-interfaces):\n",
        github::ORG
    );
    for (dir, note) in CONTRACT_DIRS {
        let path = format!("/repos/{}/ftnl-interfaces/contents/{dir}", github::ORG);
        match gh.get_json_status(&path).await {
            Ok((200, body)) => {
                out.push_str(&format!("\n## {dir}/ — {note}\n"));
                out.push_str(&summarize_contents(&body));
            }
            Ok((404, _)) => {
                out.push_str(&format!("\n## {dir}/ — {note}\n  (not present)\n"));
            }
            Ok((status, _)) => {
                out.push_str(&format!("\n## {dir}/ — {note}\n  (HTTP {status})\n"));
            }
            Err(e) => {
                out.push_str(&format!("\n## {dir}/ — {note}\n  <{e}>\n"));
            }
        }
    }
    out.push_str(
        "\n(v1 is additive; clients must ignore unknown fields/event kinds. Removing or \
         changing a field's meaning requires a new API version.)\n",
    );
    Ok(util::truncate_output(out))
}

/// Pure: render a GitHub `contents` directory listing as `name (type, size)`.
pub fn summarize_contents(body: &Value) -> String {
    match body.as_array() {
        Some(list) if !list.is_empty() => {
            let mut rows: Vec<String> = list
                .iter()
                .map(|e| {
                    let name = e.get("name").and_then(Value::as_str).unwrap_or("?");
                    let kind = e.get("type").and_then(Value::as_str).unwrap_or("?");
                    match kind {
                        "dir" => format!("  {name}/  (dir)"),
                        _ => {
                            let size = e.get("size").and_then(Value::as_u64).unwrap_or(0);
                            format!("  {name}  ({size} bytes)")
                        }
                    }
                })
                .collect();
            rows.sort();
            rows.join("\n")
        }
        _ => "  (empty)".to_string(),
    }
}

/// Release/repository roll-up for the whole org.
pub async fn inventory(gh: &GitHubClient) -> Result<String, String> {
    let mut out = format!("{} org repository + release inventory:\n\n", github::ORG);
    for repo in github::CI_REPOS {
        let meta = gh
            .get_json_status(&format!("/repos/{}/{repo}", github::ORG))
            .await;
        let (line, ok) = match meta {
            Ok((200, v)) => (summarize_repo(repo, &v), true),
            Ok((status, _)) => (format!("## {repo}\n  (HTTP {status})"), false),
            Err(e) => (format!("## {repo}\n  <{e}>"), false),
        };
        out.push_str(&line);
        out.push('\n');
        if ok {
            let rel = gh
                .get_json_status(&format!("/repos/{}/{repo}/releases/latest", github::ORG))
                .await;
            let rel_line = match rel {
                Ok((200, v)) => summarize_release(&v),
                Ok((404, _)) => "  latest release: (none)".to_string(),
                Ok((status, _)) => format!("  latest release: (HTTP {status})"),
                Err(e) => format!("  latest release: <{e}>"),
            };
            out.push_str(&rel_line);
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(util::truncate_output(out))
}

/// Pure: one repo's metadata line.
pub fn summarize_repo(repo: &str, v: &Value) -> String {
    let visibility = if v.get("private").and_then(Value::as_bool) == Some(true) {
        "private"
    } else {
        "public"
    };
    let archived = if v.get("archived").and_then(Value::as_bool) == Some(true) {
        "  [archived]"
    } else {
        ""
    };
    let desc = v
        .get("description")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .unwrap_or("(no description)");
    let branch = v
        .get("default_branch")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let pushed = v.get("pushed_at").and_then(Value::as_str).unwrap_or("?");
    format!(
        "## {repo}  ({visibility}, default {branch}){archived}\n  {desc}\n  last push: {pushed}"
    )
}

/// Pure: latest-release line.
pub fn summarize_release(v: &Value) -> String {
    let tag = v.get("tag_name").and_then(Value::as_str).unwrap_or("?");
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty());
    let published = v.get("published_at").and_then(Value::as_str).unwrap_or("?");
    let draft = v.get("draft").and_then(Value::as_bool) == Some(true);
    let prerelease = v.get("prerelease").and_then(Value::as_bool) == Some(true);
    let mut flags = String::new();
    if draft {
        flags.push_str(" [draft]");
    }
    if prerelease {
        flags.push_str(" [prerelease]");
    }
    match name {
        Some(n) => format!("  latest release: {tag} — {n}  ({published}){flags}"),
        None => format!("  latest release: {tag}  ({published}){flags}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_contents_lists_files_and_dirs_sorted() {
        let body = json!([
            {"name": "ftnl.openapi.yaml", "type": "file", "size": 12345},
            {"name": "components", "type": "dir"}
        ]);
        let s = summarize_contents(&body);
        assert!(s.contains("ftnl.openapi.yaml  (12345 bytes)"));
        assert!(s.contains("components/  (dir)"));
        // sorted: "components/" precedes "ftnl.openapi.yaml"
        assert!(s.find("components").unwrap() < s.find("ftnl.openapi").unwrap());
        assert_eq!(summarize_contents(&json!([])), "  (empty)");
    }

    #[test]
    fn summarize_repo_renders_visibility_and_desc() {
        let v = json!({
            "private": true,
            "description": "Rust control and data-plane API",
            "default_branch": "main",
            "pushed_at": "2026-07-31T06:08:58Z"
        });
        let s = summarize_repo("ftnl-backend-api.rs", &v);
        assert!(s.contains("ftnl-backend-api.rs"));
        assert!(s.contains("private"));
        assert!(s.contains("default main"));
        assert!(s.contains("Rust control and data-plane API"));
        assert!(s.contains("2026-07-31"));

        let pub_archived = json!({"private": false, "archived": true, "default_branch": "main"});
        let s2 = summarize_repo("old", &pub_archived);
        assert!(s2.contains("public"));
        assert!(s2.contains("[archived]"));
        assert!(s2.contains("(no description)"));
    }

    #[test]
    fn summarize_release_handles_tags_and_flags() {
        let v = json!({"tag_name": "v0.2.0", "name": "Beta", "published_at": "2026-07-30T00:00:00Z", "prerelease": true});
        let s = summarize_release(&v);
        assert!(s.contains("v0.2.0 — Beta"));
        assert!(s.contains("[prerelease]"));
        let bare = json!({"tag_name": "v1.0.0", "published_at": "2026-07-31T00:00:00Z"});
        assert!(summarize_release(&bare).contains("latest release: v1.0.0"));
    }
}
