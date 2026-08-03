#!/usr/bin/env python3
"""One-shot structural migration from wait_with_output to bounded pipe drains."""

from pathlib import Path

path = Path("src/util.rs")
text = path.read_text(encoding="utf-8")

imports = "use std::path::{Path, PathBuf};\nuse std::time::Duration;\n"
replacement_imports = (
    imports + "\nuse tokio::io::{AsyncRead, AsyncReadExt};\n"
)
if text.count(imports) != 1:
    raise SystemExit("util import anchor is missing or ambiguous")
if "use tokio::io::{AsyncRead, AsyncReadExt};" not in text:
    text = text.replace(imports, replacement_imports, 1)

start_marker = "/// Run a command with a timeout; returns (exit_ok, combined stdout+stderr).\n"
end_marker = "\npub async fn git(dir: &Path, args: &[&str])"
if text.count(start_marker) != 1 or text.count(end_marker) != 1:
    raise SystemExit("run_cmd structural anchors are missing or ambiguous")
start = text.index(start_marker)
end = text.index(end_marker, start)

new = '''/// Default maximum stdout retained from one child process.
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
        text.push_str("\\n--- stderr ---\\n");
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
'''

text = text[:start] + new + text[end:]
path.write_text(text, encoding="utf-8")
