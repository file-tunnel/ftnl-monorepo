//! End-to-end test: spawn the real binary, speak MCP JSON-RPC over stdio
//! against a hermetic temp "org" (a scratch git repo), and check tool
//! behavior including error/validation paths. No network access required.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct McpProc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpProc {
    fn spawn(org_root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ftnl-mcp"))
            .env("FTNL_ROOT", org_root)
            // Ensure Supabase/Cloudflare/fiducia tools take their unset-env paths.
            .env_remove("SUPABASE_URL")
            .env_remove("SUPABASE_SERVICE_ROLE_KEY")
            .env_remove("CLOUDFLARE_API_TOKEN")
            .env_remove("FIDUCIA_URL")
            .env_remove("FIDUCIA_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ftnl-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut proc = McpProc {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = proc.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "integration-test", "version": "0"}
            }),
        );
        assert_eq!(
            init.pointer("/result/serverInfo/name")
                .and_then(Value::as_str),
            Some("ftnl-mcp-server")
        );
        assert!(
            init.pointer("/result/instructions")
                .and_then(Value::as_str)
                .unwrap_or("")
                .contains("org_map"),
            "instructions should mention org_map"
        );
        proc.notify("notifications/initialized");
        proc
    }

    fn send(&mut self, msg: &Value) {
        let line = serde_json::to_string(msg).unwrap();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read response");
            assert!(n > 0, "server closed stdout before responding to {method}");
            let v: Value = serde_json::from_str(&line).expect("response is JSON");
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                return v;
            }
        }
    }

    fn call_tool(&mut self, name: &str, args: Value) -> (bool, String) {
        let resp = self.request("tools/call", json!({"name": name, "arguments": args}));
        let result = resp
            .get("result")
            .unwrap_or_else(|| panic!("tools/call {name} returned no result: {resp}"));
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = result
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        (is_error, text)
    }
}

impl Drop for McpProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build a temp org root containing one tiny committed git repo.
fn temp_org(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ftnl-mcp-it-{tag}-{}", std::process::id()));
    let repo = root.join("demo-repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "// SENTINEL_PATTERN_XYZ lives here\npub fn f() {}\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.email=it@test",
                "-c",
                "user.name=Integration Test",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(st.success(), "git {args:?} failed");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "initial"]);
    root
}

#[test]
fn tools_list_contains_all_tool_families() {
    let org = temp_org("list");
    let mut mcp = McpProc::spawn(&org);
    let resp = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = resp
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .expect("tools array")
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .collect();
    for expected in [
        // org/git
        "org_overview",
        "repo_status",
        "recent_commits",
        "search_code",
        // cargo build-only
        "cargo_build",
        "cargo_check",
        // transfer / tunnel API
        "transfer_api_health",
        "tunnel_api_routes",
        "architecture_docs",
        // interfaces / inventory / monorepo / ci / site
        "interface_contracts",
        "repo_inventory",
        "monorepo_pins",
        "org_ci_status",
        "site_check",
        // domains
        "dns_lookup",
        "domain_info",
        "tls_cert_check",
        "cloudflare_zones",
        "cloudflare_dns_records",
        // supabase telemetry
        "client_log_sessions",
        "tail_client_logs",
        "client_error_summary",
        "client_log_trace",
        "telemetry_docs",
        // docs + ops/aggregate
        "org_map",
        "stack_status",
        "self_test",
        "fiducia_status",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }
    // every tool must carry a valid inputSchema object
    for t in resp
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .unwrap()
    {
        assert!(
            t.get("inputSchema").map(Value::is_object).unwrap_or(false),
            "tool {:?} missing inputSchema",
            t.get("name")
        );
    }
    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn resources_and_prompts_are_listed_and_readable() {
    let org = temp_org("respr");
    let mut mcp = McpProc::spawn(&org);

    let rl = mcp.request("resources/list", json!({}));
    let uris: Vec<&str> = rl
        .pointer("/result/resources")
        .and_then(Value::as_array)
        .expect("resources array")
        .iter()
        .filter_map(|r| r.get("uri").and_then(Value::as_str))
        .collect();
    for u in [
        "orgmap://file-tunnel",
        "docs://architecture",
        "docs://telemetry",
        "schema://client-telemetry",
    ] {
        assert!(uris.contains(&u), "missing resource {u}: {uris:?}");
    }

    // read the schema resource and confirm the DDL is present
    let rr = mcp.request(
        "resources/read",
        json!({"uri": "schema://client-telemetry"}),
    );
    let text = rr
        .pointer("/result/contents/0/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        text.contains("ftnl_client_log_snapshots"),
        "got: {text:.120}"
    );

    // unknown resource → error, not crash
    let bad = mcp.request("resources/read", json!({"uri": "nope://x"}));
    assert!(bad.get("error").is_some(), "unknown resource should error");

    // prompts
    let pl = mcp.request("prompts/list", json!({}));
    let names: Vec<&str> = pl
        .pointer("/result/prompts")
        .and_then(Value::as_array)
        .expect("prompts array")
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .collect();
    for p in [
        "deploy_readiness",
        "triage_client_errors",
        "domain_audit",
        "transfer_api_review",
    ] {
        assert!(names.contains(&p), "missing prompt {p}: {names:?}");
    }

    // get a parameterized prompt
    let gp = mcp.request(
        "prompts/get",
        json!({"name": "domain_audit", "arguments": {"domain": "file-tunnel.github.io"}}),
    );
    let msg = gp
        .pointer("/result/messages/0/content/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(msg.contains("file-tunnel.github.io"), "got: {msg:.160}");

    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn ops_tools_behave() {
    let org = temp_org("ops");
    let mut mcp = McpProc::spawn(&org);

    // self_test is local, always succeeds, presence-only (no secret values)
    let (err, text) = mcp.call_tool("self_test", json!({}));
    assert!(!err, "self_test errored: {text}");
    assert!(text.contains("SUPABASE_SERVICE_ROLE_KEY"));
    assert!(text.contains("MISSING") || text.contains("present"));

    // fiducia unconfigured → clean typed error naming the env vars
    let (err, text) = mcp.call_tool("fiducia_status", json!({}));
    assert!(err);
    assert!(text.contains("FIDUCIA_URL"), "got: {text}");

    // client_error_summary rejects a junk group_by before any network I/O
    let (err, text) = mcp.call_tool("client_error_summary", json!({"group_by": "user; drop"}));
    assert!(err);
    assert!(text.contains("invalid group_by"), "got: {text}");

    // client_log_trace rejects an injected session id
    let (err, text) = mcp.call_tool("client_log_trace", json!({"session_id": "--x; rm"}));
    assert!(err);
    assert!(text.contains("invalid session_id"), "got: {text}");

    // stack_status runs the whole rollup and returns a verdict (network checks
    // may degrade in a sandbox, but it must not crash and must name a verdict)
    let (err, text) = mcp.call_tool("stack_status", json!({}));
    assert!(!err, "stack_status errored: {text}");
    assert!(
        text.contains("File Tunnel stack status:"),
        "got: {text:.200}"
    );

    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn org_and_git_tools_work_against_temp_org() {
    let org = temp_org("git");
    let mut mcp = McpProc::spawn(&org);

    let (err, text) = mcp.call_tool("org_overview", json!({}));
    assert!(!err, "org_overview errored: {text}");
    assert!(text.contains("demo-repo"));
    assert!(text.contains("branch: main"));

    let (err, text) = mcp.call_tool(
        "search_code",
        json!({"pattern": "SENTINEL_PATTERN_[A-Z]+", "max_matches": 10}),
    );
    assert!(!err, "search_code errored: {text}");
    assert!(text.contains("demo-repo/src/lib.rs:1:"), "got: {text}");

    let (err, text) = mcp.call_tool("recent_commits", json!({"repo": "demo-repo", "count": 5}));
    assert!(!err, "recent_commits errored: {text}");
    assert!(text.contains("initial"));

    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn embedded_docs_tools_describe_the_org() {
    let org = temp_org("docs");
    let mut mcp = McpProc::spawn(&org);

    let (err, text) = mcp.call_tool("org_map", json!({}));
    assert!(!err);
    assert!(text.contains("ftnl-backend-api.rs"));
    assert!(text.contains("8080"));

    let (err, text) = mcp.call_tool("architecture_docs", json!({}));
    assert!(!err);
    assert!(text.contains("capability"));
    assert!(text.contains("/v1/tunnels/{id}/claim"));

    let (err, text) = mcp.call_tool("telemetry_docs", json!({}));
    assert!(!err);
    assert!(text.contains("ingest_ftnl_client_log_snapshot"));
    assert!(text.contains("supabase_flutter"));

    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn invalid_inputs_surface_as_tool_errors_not_crashes() {
    let org = temp_org("err");
    let mut mcp = McpProc::spawn(&org);

    // path traversal in repo name
    let (err, text) = mcp.call_tool("repo_status", json!({"repo": "../../etc"}));
    assert!(err, "traversal should be a tool error");
    assert!(text.contains("invalid repo name"));

    // unknown repo for a cargo build
    let (err, text) = mcp.call_tool("cargo_build", json!({"repo": "nope-repo"}));
    assert!(err);
    assert!(text.contains("no such repo"));

    // flag injection into a session id
    let (err, text) = mcp.call_tool(
        "tail_client_logs",
        json!({"session_id": "--sneaky; rm -rf /"}),
    );
    assert!(err);
    assert!(text.contains("invalid session_id"));

    // bad hostname for TLS check (rejected before any network I/O)
    let (err, text) = mcp.call_tool("tls_cert_check", json!({"host": "bad host;id"}));
    assert!(err);
    assert!(text.contains("invalid hostname"));

    // bad domain for DoH lookup
    let (err, text) = mcp.call_tool("dns_lookup", json!({"domain": "exa mple.com"}));
    assert!(err);
    assert!(text.contains("invalid hostname"));

    // record_type smuggling is rejected before any network I/O
    let (err, text) = mcp.call_tool(
        "dns_lookup",
        json!({"domain": "file-tunnel.github.io", "record_type": "A;DROP"}),
    );
    assert!(err);
    assert!(text.contains("invalid DNS record type"), "got: {text}");

    // cloudflare_dns_records validates record_type before it needs a token
    let (err, text) = mcp.call_tool(
        "cloudflare_dns_records",
        json!({"zone": "file-tunnel.app", "record_type": "A;x"}),
    );
    assert!(err);
    assert!(text.contains("invalid DNS record type"), "got: {text}");

    // path-traversal-shaped session_id is rejected
    for sid in ["../../secret", "a/b", "x@y"] {
        let (err, text) = mcp.call_tool("client_log_trace", json!({"session_id": sid}));
        assert!(err, "session_id {sid:?} should be rejected");
        assert!(text.contains("invalid session_id"), "got: {text}");
    }

    // server must still be alive after all those error paths
    let (err, _) = mcp.call_tool("org_map", json!({}));
    assert!(!err, "server wedged after error paths");

    let _ = std::fs::remove_dir_all(&org);
}

#[test]
fn env_gated_tools_error_actionably_without_credentials() {
    let org = temp_org("env");
    let mut mcp = McpProc::spawn(&org);

    let (err, text) = mcp.call_tool("cloudflare_zones", json!({}));
    assert!(err);
    assert!(text.contains("CLOUDFLARE_API_TOKEN"), "got: {text}");

    let (err, text) = mcp.call_tool("client_log_sessions", json!({}));
    assert!(err);
    assert!(text.contains("SUPABASE_URL"), "got: {text}");

    let _ = std::fs::remove_dir_all(&org);
}
