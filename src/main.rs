//! Thin stdio bootstrap; the library runtime owns lifecycle and telemetry.

#[tokio::main]
async fn main() -> ftnl_mcp_server::runtime::Result<()> {
    ftnl_mcp_server::runtime::run_stdio().await
}
