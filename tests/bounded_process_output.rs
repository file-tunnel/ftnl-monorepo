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
    assert_eq!(output, "hello\n--- stderr ---\nwarning");
}

#[tokio::test]
async fn preserves_nonzero_exit_as_a_product_status() {
    let (success, output) = run_cmd_with_limits(
        None,
        "/bin/sh",
        &["-c", "printf 'partial'; exit 7"],
        small_limits(Duration::from_secs(5)),
    )
    .await
    .expect("a nonzero child exit is not a capture failure");

    assert!(!success);
    assert_eq!(output, "partial");
}

#[tokio::test]
async fn rejects_invalid_limits_before_spawning() {
    let limits = CommandOutputLimits {
        timeout: Duration::from_secs(5),
        max_stdout_bytes: 0,
        max_stderr_bytes: 1024,
    };
    let error = run_cmd_with_limits(None, "/bin/sh", &["-c", "exit 0"], limits)
        .await
        .expect_err("zero-byte stdout limits must fail closed");

    assert_eq!(error, "invalid subprocess output limits");
}

#[tokio::test]
async fn preserves_product_local_missing_program_errors() {
    let program = "/definitely-not-a-real-ftnl-mcp-program";
    let error = run_cmd_with_limits(
        None,
        program,
        &[],
        small_limits(Duration::from_secs(5)),
    )
    .await
    .expect_err("a missing executable must be reported");

    assert!(error.starts_with(&format!("`{program}` not found on PATH:")));
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

#[test]
fn process_adapter_delegates_to_immutable_shared_crate() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/util.rs");
    let source = std::fs::read_to_string(path).expect("read process adapter source");
    assert!(source.contains("ore_mcp_process::{run_bounded"));
    assert!(!source.contains("tokio::process::Command"));
    assert!(!source.contains("async fn read_pipe_bounded"));
}
