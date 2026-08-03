#![cfg(unix)]

use std::time::Duration;

use ftnl_mcp_server::util::{run_cmd_with_limits, CommandOutputLimits};

fn small_limits(timeout: Duration) -> CommandOutputLimits {
    CommandOutputLimits {
        timeout,
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
    }
}

#[tokio::test]
async fn captures_both_pipes_below_the_limits() {
    let (success, output) = run_cmd_with_limits(
        None,
        "/bin/sh",
        &["-c", "printf 'hello'; printf 'warning' >&2"],
        small_limits(Duration::from_secs(5)),
    )
    .await
    .expect("bounded child should succeed");

    assert!(success);
    assert!(output.contains("hello"));
    assert!(output.contains("--- stderr ---"));
    assert!(output.contains("warning"));
}

#[tokio::test]
async fn rejects_stdout_before_buffering_an_unbounded_log() {
    let error = run_cmd_with_limits(
        None,
        "/bin/sh",
        &["-c", "head -c 2048 /dev/zero"],
        small_limits(Duration::from_secs(5)),
    )
    .await
    .expect_err("stdout overflow must fail closed");

    assert!(error.contains("stdout exceeded the 1024-byte limit"));
}

#[tokio::test]
async fn rejects_stderr_before_buffering_an_unbounded_log() {
    let error = run_cmd_with_limits(
        None,
        "/bin/sh",
        &["-c", "head -c 2048 /dev/zero >&2"],
        small_limits(Duration::from_secs(5)),
    )
    .await
    .expect_err("stderr overflow must fail closed");

    assert!(error.contains("stderr exceeded the 1024-byte limit"));
}

#[tokio::test]
async fn kills_and_reaps_a_timed_out_child() {
    let error = run_cmd_with_limits(
        None,
        "/bin/sh",
        &["-c", "sleep 5"],
        small_limits(Duration::from_millis(50)),
    )
    .await
    .expect_err("timeout must fail closed");

    assert!(error.contains("timed out after"));
}
