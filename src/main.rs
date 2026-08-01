//! Entry point: serve the File Tunnel MCP tool surface over stdio.
//!
//! stdout is the MCP wire — every diagnostic here goes to stderr only.

use ftnl_mcp_server::server::FtnlMcp;
use rmcp::{transport::stdio, ServiceExt};
use tracing::Instrument;

#[tokio::main]
async fn main() -> anyhow_lite::Result<()> {
    let _telemetry = ftnl_mcp_server::telemetry::init("ftnl-mcp-server", "file-tunnel");
    let server = FtnlMcp::new();
    tracing::info!(
        org.root = %server.root.display(),
        transfer.api_configured = std::env::var_os("FTNL_API_URL").is_some(),
        transport = "stdio",
        "starting MCP server"
    );
    let server_span = tracing::info_span!("mcp.server", rpc.system = "mcp", transport = "stdio");
    let service = server
        .serve(stdio())
        .instrument(server_span.clone())
        .await
        .map_err(|e| anyhow_lite::Error(e.to_string()))?;
    service
        .waiting()
        .instrument(server_span)
        .await
        .map_err(|e| anyhow_lite::Error(e.to_string()))?;
    Ok(())
}

/// Tiny local error shim so we don't pull in the `anyhow` crate just for `main`.
mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Error>;

    #[derive(Debug)]
    pub struct Error(pub String);

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for Error {}
}
