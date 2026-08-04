//! Shared helpers: subprocess running, output shaping, and input validation.
//!
//! Anything that reaches a CLI (`cargo`, `git`, `openssl`) or a URL (repo
//! names, hostnames, git refs, session ids) is validated against a
//! conservative charset first, so nothing can smuggle flags or shell syntax.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

pub const MAX_OUTPUT_CHARS: usize = 40_000;

/// Hard cap on how many bytes we will buffer from any single upstream HTTP
/// response. A hostile or misconfigured upstream (a rogue backend, DoH/RDAP,
/// Cloudflare, GitHub, Supabase, fiducia, or the live site) must not be able to
/// exhaust memory.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Wall-clock deadline for streaming a response body. The reqwest client
/// `.timeout()` bounds establishing the request, but does not by itself bound a
/// manual `chunk()` read loop, so we enforce our own deadline here.
const RESPONSE_READ_DEADLINE: Duration = Duration::from_secs(20);

/// The File Tunnel org root on disk. Defaults to `~/codes/file-tunnel`;
/// override with `FTNL_ROOT`.
pub fn org_root() -> PathBuf {
    if let Ok(root) = std::env::var("FTNL_ROOT") {
        return PathBuf::from(root);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("codes/file-tunnel")
}

/// Bound tool output so a runaway build log can't flood the MCP channel.
pub fn truncate_output(mut s: String) -> String {
    if s.len() > MAX_OUTPUT_CHARS {
        let mut cut = MAX_OUTPUT_CHARS;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        let dropped = s.len() - cut;
        s.truncate(cut);
        s.push_str(&format!("\n…[output truncated, {dropped} bytes dropped]"));
    }
    s
}

/// Default maximum stdout retained from one child process.
pub const MAX_COMMAND_STDOUT_BYTES: usize = 1024 * 1024;

/// Default maximum stderr retained from one child process.
pub const MAX_COMMAND_STDERR_BYTES: usize = 256 * 1024;

/// Resource limits for one child process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandOutputLimits {
    /// Wall-clock deadline.
    pub timeout: Duration,
    /// Maximum stdout bytes accepted before termination.
    pub max_stdout_bytes: usize,
    /// Maximum stderr bytes accepted before termination.
    pub max_stderr_bytes: usize,
}

/// Run a command with a timeout and bounded output; returns
/// `(exit_ok, combined stdout+stderr)`.
pub async fn run_cmd(
    dir: Option<&Path>,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(bool, String), String> {
    run_cmd_with_limits(
        dir,
        program,
        args,
        CommandOutputLimits {
            timeout,
            max_stdout_bytes: MAX_COMMAND_STDOUT_BYTES,
            max_stderr_bytes: MAX_COMMAND_STDERR_BYTES,
        },
    )
    .await
}

/// Run a command while concurrently draining stdout and stderr under explicit
/// byte and time limits.
///
/// The child is killed and reaped on timeout or the first over-limit byte.
/// Arguments are deliberately omitted from errors because diagnostic commands
/// can contain private paths or operator input.
pub async fn run_cmd_with_limits(
    dir: Option<&Path>,
    program: &str,
    args: &[&str],
    limits: CommandOutputLimits,
) -> Result<(bool, String), String> {
    const MAX_CONFIGURED_CAPTURE: usize = 16 * 1024 * 1024;
    if limits.timeout.is_zero()
        || !(1..=MAX_CONFIGURED_CAPTURE).contains(&limits.max_stdout_bytes)
        || !(1..=MAX_CONFIGURED_CAPTURE).contains(&limits.max_stderr_bytes)
    {
        return Err("invalid subprocess output limits".to_string());
    }

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("`{program}` not found on PATH: {error}")
        } else {
            format!("failed to spawn {program}: {error}")
        }
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "subprocess stdout pipe is unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "subprocess stderr pipe is unavailable".to_string())?;

    let operation = async {
        let wait = async {
            child
                .wait()
                .await
                .map_err(|error| format!("{program} failed: {error}"))
        };
        let stdout = read_pipe_bounded(stdout, limits.max_stdout_bytes, "stdout");
        let stderr = read_pipe_bounded(stderr, limits.max_stderr_bytes, "stderr");
        tokio::try_join!(wait, stdout, stderr)
    };

    let result = tokio::time::timeout(limits.timeout, operation).await;
    let (status, stdout, stderr) = match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(format!("`{program}` timed out after {:?}", limits.timeout));
        }
    };

    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr);
    if !stderr.trim().is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&stderr);
    }
    Ok((status.success(), text))
}

async fn read_pipe_bounded<R>(
    mut reader: R,
    limit: usize,
    stream_name: &str,
) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("reading subprocess {stream_name} failed: {error}"))?;
        if read == 0 {
            return Ok(output);
        }
        let next = output
            .len()
            .checked_add(read)
            .ok_or_else(|| format!("subprocess {stream_name} length overflow"))?;
        if next > limit {
            return Err(format!(
                "subprocess {stream_name} exceeded the {limit}-byte limit"
            ));
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

pub async fn git(dir: &Path, args: &[&str]) -> Result<(bool, String), String> {
    run_cmd(Some(dir), "git", args, Duration::from_secs(30)).await
}

/// Read an HTTP response body with both a size cap and a read deadline, so a
/// malformed/oversized/stalled upstream yields a typed error instead of an OOM
/// or a hang. Short-circuits on an oversized declared `Content-Length` and
/// enforces the cap again while streaming (a lying/absent length can't slip
/// past). Returns the raw bytes; use [`read_text_capped`]/[`read_json_capped`]
/// for the common shapes.
pub async fn read_capped(resp: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if let Some(len) = resp.content_length() {
        if len > max as u64 {
            return Err(format!(
                "upstream response too large: declared Content-Length {len} exceeds {max}-byte cap"
            ));
        }
    }
    let mut resp = resp;
    let read = async move {
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("reading response body failed: {e}"))?
        {
            if buf.len() + chunk.len() > max {
                return Err(format!("upstream response exceeded the {max}-byte cap"));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok::<Vec<u8>, String>(buf)
    };
    tokio::time::timeout(RESPONSE_READ_DEADLINE, read)
        .await
        .map_err(|_| format!("reading response body timed out after {RESPONSE_READ_DEADLINE:?}"))?
}

/// Size- and time-bounded UTF-8 (lossy) read of a response body.
pub async fn read_text_capped(resp: reqwest::Response) -> Result<String, String> {
    let bytes = read_capped(resp, MAX_RESPONSE_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Size- and time-bounded JSON read of a response body.
pub async fn read_json_capped(resp: reqwest::Response) -> Result<serde_json::Value, String> {
    let bytes = read_capped(resp, MAX_RESPONSE_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("response was not valid JSON: {e}"))
}

/// Validate a DNS/Cloudflare record type (A, AAAA, CNAME, TXT, HTTPS, …) before
/// it reaches a request URL. Record types are short and ASCII-alphanumeric;
/// anything else is rejected (URL-encoding already neutralizes it, but the value
/// should never reach a URL unvalidated).
pub fn safe_record_type(rtype: &str) -> Result<(), String> {
    let ok = (1..=16).contains(&rtype.len()) && rtype.chars().all(|c| c.is_ascii_alphanumeric());
    if !ok {
        return Err(format!("invalid DNS record type: {rtype:?}"));
    }
    Ok(())
}

/// Validate an opaque identifier (e.g. a client `session_id`) that will be
/// interpolated into a request. Stricter than [`safe_token`]: it forbids `/`,
/// `\`, `:`, `@`, `..`, and a leading `.`/`-`, so it can never introduce a path
/// segment, a scheme/host, or a traversal — even in URL positions where those
/// would matter. Use [`safe_token`] only for values (git refs) that legitimately
/// contain `/` or `..`.
pub fn safe_opaque_id(id: &str, what: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && !id.starts_with('.')
        && !id.starts_with('-')
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        return Err(format!("invalid {what}: {id:?}"));
    }
    Ok(())
}

/// Validate a name used as a single path segment (repo name, log level, …).
pub fn safe_segment(name: &str, what: &str) -> Result<(), String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
    {
        return Err(format!("invalid {what} name: {name:?}"));
    }
    Ok(())
}

/// Validate a token passed through to a CLI (git ref, filter, …): conservative
/// charset and never flag-shaped.
pub fn safe_token(name: &str, what: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':' | '/' | '@'));
    if !ok {
        return Err(format!("invalid {what}: {name:?}"));
    }
    Ok(())
}

/// Validate an operator-supplied base URL (e.g. a backend or fiducia endpoint):
/// must be http(s):// and free of whitespace/quotes so nothing can smuggle a
/// second URL or a header. Returns the trailing-slash-trimmed form.
pub fn safe_base_url(url: &str) -> Result<String, String> {
    let raw = url.trim();
    if raw.len() > 2_048
        || raw
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '`' | '\'' | '"'))
    {
        return Err("base URL contains invalid characters".to_string());
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| "base URL is invalid".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err("base URL must use http(s) and include a host".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("base URL must not contain credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("base URL must not contain a query or fragment".to_string());
    }
    if is_metadata_host(&parsed) {
        return Err("cloud metadata endpoints are not allowed".to_string());
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn is_metadata_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(ip) => ip.is_link_local() || ip.is_unspecified(),
            std::net::IpAddr::V6(ip) => ip.is_unicast_link_local() || ip.is_unspecified(),
        };
    }
    matches!(
        host.trim_end_matches('.').to_ascii_lowercase().as_str(),
        "metadata.google.internal" | "metadata.azure.internal"
    )
}

/// Validate a DNS hostname / domain name.
pub fn safe_hostname(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 253
        && !name.starts_with('-')
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'));
    if !ok {
        return Err(format!("invalid hostname: {name:?}"));
    }
    Ok(())
}

/// Parse an openssl `notAfter=` date like "Jun 15 12:00:00 2026 GMT".
pub fn parse_openssl_enddate(s: &str) -> Option<chrono::NaiveDateTime> {
    let s = s.trim().strip_prefix("notAfter=").unwrap_or(s.trim());
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%b %e %H:%M:%S %Y GMT").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segment_accepts_normal_names() {
        assert!(safe_segment("ftnl-backend-api.rs", "repo").is_ok());
        assert!(safe_segment("ftnl-monorepo", "repo").is_ok());
    }

    #[test]
    fn safe_segment_rejects_traversal_and_hidden() {
        for bad in ["", "..", "a/b", "a\\b", ".git", "../etc", "foo/.."] {
            assert!(safe_segment(bad, "repo").is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn safe_token_rejects_flag_injection() {
        assert!(safe_token("main", "branch").is_ok());
        assert!(safe_token("v0.1.0", "ref").is_ok());
        for bad in ["", "--all", "-n", "a b", "x;y", "$(id)", "a`b`"] {
            assert!(safe_token(bad, "arg").is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn safe_base_url_accepts_http_and_rejects_smuggling() {
        assert_eq!(
            safe_base_url("http://127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            safe_base_url("https://fiducia.cloud").unwrap(),
            "https://fiducia.cloud"
        );
        for bad in [
            "ftp://x",
            "127.0.0.1",
            "https://a b",
            "https://a'b",
            "http://a\"b",
            "https://user:secret@x.com",
            "http://169.254.169.254/latest/meta-data",
            "http://metadata.google.internal/computeMetadata/v1",
        ] {
            assert!(safe_base_url(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn safe_hostname_validation() {
        assert!(safe_hostname("file-tunnel.github.io").is_ok());
        assert!(safe_hostname("a-b.c-d.io").is_ok());
        for bad in ["", "-x.com", ".com", "exa mple.com", "a;b.com", "x_y.com"] {
            assert!(safe_hostname(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn truncate_output_marks_dropped_bytes_and_respects_boundaries() {
        let s = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let t = truncate_output(s);
        assert!(t.contains("output truncated"));
        assert!(t.len() < MAX_OUTPUT_CHARS + 100);
        assert_eq!(truncate_output("short".into()), "short");
        // multi-byte chars straddling the cut must not panic
        let multi = "é".repeat(MAX_OUTPUT_CHARS);
        assert!(truncate_output(multi).contains("output truncated"));
    }

    #[test]
    fn parse_openssl_enddate_formats() {
        let d = parse_openssl_enddate("notAfter=Jun 15 12:00:00 2026 GMT").unwrap();
        assert_eq!(d.format("%Y-%m-%d").to_string(), "2026-06-15");
        let d = parse_openssl_enddate("notAfter=Jul  3 01:02:03 2027 GMT").unwrap();
        assert_eq!(d.format("%Y-%m-%d").to_string(), "2027-07-03");
        assert!(parse_openssl_enddate("garbage").is_none());
    }

    #[test]
    fn safe_record_type_accepts_types_and_rejects_smuggling() {
        for good in [
            "A", "AAAA", "CNAME", "TXT", "NS", "MX", "SRV", "CAA", "HTTPS", "SVCB",
        ] {
            assert!(safe_record_type(good).is_ok(), "should accept {good:?}");
        }
        for bad in [
            "",
            "A;DROP",
            "A B",
            "TXT=1",
            "a/b",
            "../x",
            "A,AAAA",
            "TOOOOOOOOOOOOOOOOLONG",
        ] {
            assert!(safe_record_type(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn safe_opaque_id_rejects_traversal_scheme_and_path() {
        for good in ["abc-123", "sess_ABC.def", "a1b2c3", "01F8MECHZX3TBDSZ7XR8H"] {
            assert!(
                safe_opaque_id(good, "session_id").is_ok(),
                "accept {good:?}"
            );
        }
        // exactly the loose pattern flagged cross-fleet: `/` and `..` must not pass.
        for bad in [
            "",
            "..",
            "../../etc",
            "a/b",
            "a\\b",
            "x@y",
            "a:b",
            ".hidden",
            "-x",
            "a b",
            "x;y",
            "a..b",
            "s/../s",
        ] {
            let e = safe_opaque_id(bad, "session_id").unwrap_err();
            assert!(e.contains("invalid session_id"), "reject {bad:?}: {e}");
        }
    }

    /// Serve one hand-crafted HTTP/1.1 response on a loopback socket, then close.
    fn serve_once(raw_response: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf); // drain the request line/headers
                let _ = sock.write_all(&raw_response);
                let _ = sock.flush();
            }
        });
        format!("http://{addr}/")
    }

    async fn get(url: &str) -> reqwest::Response {
        reqwest::Client::new().get(url).send().await.unwrap()
    }

    #[tokio::test]
    async fn read_capped_short_circuits_oversized_content_length() {
        let url = serve_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n".to_vec(),
        );
        let err = read_capped(get(&url).await, 100).await.unwrap_err();
        assert!(err.contains("Content-Length"), "got: {err}");
    }

    #[tokio::test]
    async fn read_capped_enforces_cap_while_streaming() {
        // No Content-Length; body is delivered then the socket closes. The cap
        // must still fire from the streaming accumulator.
        let mut resp = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        resp.extend(vec![b'x'; 500]);
        let url = serve_once(resp);
        let err = read_capped(get(&url).await, 100).await.unwrap_err();
        assert!(err.contains("cap"), "got: {err}");
    }

    #[tokio::test]
    async fn read_capped_returns_small_bodies() {
        let url = serve_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
        );
        let bytes = read_capped(get(&url).await, 100).await.unwrap();
        assert_eq!(bytes, b"hello");
    }
}
