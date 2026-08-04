//! Process lifecycle for the stdio MCP server.
//!
//! The binary entrypoint delegates here so transport startup, diagnostics,
//! telemetry ownership, and orderly shutdown remain testable library concerns.

use std::ffi::OsStr;

use rmcp::{transport::stdio, ServiceExt};
use tracing::Instrument;

use crate::server::FtnlMcp;

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug)]
pub struct RuntimeError(String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

fn configured(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Initialize stderr/OTLP telemetry and serve the read-only tool surface over
/// stdio until the peer closes the transport.
pub async fn run_stdio() -> Result<()> {
    let _telemetry = crate::telemetry::init("ftnl-mcp-server", "file-tunnel");
    let server = FtnlMcp::new();
    tracing::info!(
        org.root = %server.root.display(),
        transfer.api_configured = configured(std::env::var_os("FTNL_API_URL").as_deref()),
        transport = "stdio",
        stdout = "mcp-only",
        "starting MCP server"
    );

    let server_span = tracing::info_span!("mcp.server", rpc.system = "mcp", transport = "stdio");
    let service = server
        .serve(stdio())
        .instrument(server_span.clone())
        .await
        .map_err(|error| RuntimeError(error.to_string()))?;
    service
        .waiting()
        .instrument(server_span)
        .await
        .map_err(|error| RuntimeError(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn configuration_presence_never_requires_or_exposes_a_value() {
        assert!(!configured(None));
        assert!(!configured(Some(OsStr::new(""))));
        assert!(configured(Some(OsStr::new("configured"))));

        let private = OsString::from("do-not-log-this-value");
        assert!(configured(Some(private.as_os_str())));
    }
}
