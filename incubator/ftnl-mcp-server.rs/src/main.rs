use std::{env, sync::Arc, time::Duration};

use reqwest::{Client, Response, Url, redirect::Policy};
use rmcp::{
    ErrorData as McpError, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_API_BASE_URL: &str = "http://127.0.0.1:8080";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct FileTunnelServer {
    client: Client,
    base_url: Arc<Url>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateTunnelParams {
    /// Stable identifier for the application requesting the tunnel.
    application_id: String,
    /// Accepted media patterns, for example ["image/*", "application/pdf"].
    #[serde(default)]
    accept: Vec<String>,
    /// Optional maximum file count. The API default is used when omitted.
    max_files: Option<u16>,
    /// Optional maximum bytes per file. The API default is used when omitted.
    max_file_bytes: Option<u64>,
    /// Optional lifetime in seconds. The API enforces its own upper bound.
    expires_in_seconds: Option<u32>,
}

#[derive(Debug, Serialize)]
struct CreateTunnelBody {
    application_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accept: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_files: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TunnelParams {
    /// File Tunnel UUID returned by create_tunnel.
    tunnel_id: String,
    /// Desktop bearer capability returned once by create_tunnel.
    desktop_capability: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CancelTunnelParams {
    /// File Tunnel UUID returned by create_tunnel.
    tunnel_id: String,
    /// Desktop bearer capability returned once by create_tunnel.
    desktop_capability: String,
    /// Must be true. Cancellation is destructive and clears retained file state.
    confirm: bool,
}

impl FileTunnelServer {
    fn from_environment() -> anyhow::Result<Self> {
        let raw_base_url = env::var("FTNL_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
        let normalized = format!("{}/", raw_base_url.trim_end_matches('/'));
        let base_url = Url::parse(&normalized)?;

        anyhow::ensure!(
            matches!(base_url.scheme(), "http" | "https"),
            "FTNL_API_BASE_URL must use http or https"
        );
        anyhow::ensure!(
            base_url.username().is_empty() && base_url.password().is_none(),
            "FTNL_API_BASE_URL must not contain credentials"
        );
        anyhow::ensure!(
            base_url.query().is_none() && base_url.fragment().is_none(),
            "FTNL_API_BASE_URL must not contain a query or fragment"
        );

        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent(concat!("ftnl-mcp-server/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            base_url: Arc::new(base_url),
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, McpError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| McpError::invalid_params(error.to_string(), None))
    }

    fn validated_tunnel_id(value: &str) -> Result<&str, McpError> {
        let bytes = value.as_bytes();
        let hyphens_are_valid = [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes.get(index) == Some(&b'-'));
        let characters_are_valid = bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit()
        });

        if bytes.len() == 36 && hyphens_are_valid && characters_are_valid {
            Ok(value)
        } else {
            Err(McpError::invalid_params(
                "tunnel_id must be a canonical UUID",
                None,
            ))
        }
    }

    fn validated_capability(value: &str) -> Result<&str, McpError> {
        let trimmed = value.trim();
        if trimmed.len() < 24 || trimmed.len() > 4096 || trimmed.chars().any(char::is_whitespace) {
            Err(McpError::invalid_params(
                "desktop_capability is malformed",
                None,
            ))
        } else {
            Ok(trimmed)
        }
    }

    async fn render_response(response: Response) -> CallToolResult {
        let status = response.status();
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return tool_error(format!("File Tunnel response could not be read: {error}"));
            }
        };

        if bytes.len() > MAX_RESPONSE_BYTES {
            return tool_error(format!(
                "File Tunnel response exceeded the {MAX_RESPONSE_BYTES}-byte MCP safety limit"
            ));
        }

        let body = if bytes.is_empty() {
            json!({ "ok": status.is_success(), "status": status.as_u16() })
        } else {
            serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| {
                json!({
                    "status": status.as_u16(),
                    "body": String::from_utf8_lossy(&bytes),
                })
            })
        };
        let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());

        if status.is_success() {
            tool_success(text)
        } else {
            tool_error(format!("File Tunnel returned HTTP {}:\n{text}", status.as_u16()))
        }
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<CallToolResult, McpError> {
        match request.send().await {
            Ok(response) => Ok(Self::render_response(response).await),
            Err(error) if error.is_timeout() => Ok(tool_error(
                "File Tunnel request timed out before an authoritative response was received",
            )),
            Err(error) => Ok(tool_error(format!("File Tunnel request failed: {error}"))),
        }
    }
}

#[tool_router(server_handler)]
impl FileTunnelServer {
    #[tool(description = "Check the configured File Tunnel API health endpoint")]
    async fn health(&self) -> Result<CallToolResult, McpError> {
        let endpoint = self.endpoint("healthz")?;
        self.send(self.client.get(endpoint)).await
    }

    #[tool(description = "Create an ephemeral File Tunnel and return its pairing URI plus desktop capability. Treat the returned capability as a secret and do not place it in logs or issue text.")]
    async fn create_tunnel(
        &self,
        Parameters(params): Parameters<CreateTunnelParams>,
    ) -> Result<CallToolResult, McpError> {
        let application_id = params.application_id.trim();
        if application_id.is_empty() || application_id.len() > 128 {
            return Err(McpError::invalid_params(
                "application_id must contain 1 to 128 characters",
                None,
            ));
        }
        if params.accept.len() > 64
            || params
                .accept
                .iter()
                .any(|item| item.is_empty() || item.len() > 255)
        {
            return Err(McpError::invalid_params(
                "accept contains too many or malformed media patterns",
                None,
            ));
        }

        let endpoint = self.endpoint("v1/tunnels")?;
        let body = CreateTunnelBody {
            application_id: application_id.to_owned(),
            accept: params.accept,
            max_files: params.max_files,
            max_file_bytes: params.max_file_bytes,
            expires_in_seconds: params.expires_in_seconds,
        };
        self.send(self.client.post(endpoint).json(&body)).await
    }

    #[tool(description = "Read the authoritative snapshot for one File Tunnel using its desktop bearer capability")]
    async fn get_tunnel(
        &self,
        Parameters(params): Parameters<TunnelParams>,
    ) -> Result<CallToolResult, McpError> {
        let tunnel_id = Self::validated_tunnel_id(&params.tunnel_id)?;
        let capability = Self::validated_capability(&params.desktop_capability)?;
        let endpoint = self.endpoint(&format!("v1/tunnels/{tunnel_id}"))?;
        self.send(self.client.get(endpoint).bearer_auth(capability)).await
    }

    #[tool(description = "Cancel a File Tunnel and clear its retained file state. The confirm field must be true; uncertain outcomes are never retried automatically.")]
    async fn cancel_tunnel(
        &self,
        Parameters(params): Parameters<CancelTunnelParams>,
    ) -> Result<CallToolResult, McpError> {
        if !params.confirm {
            return Err(McpError::invalid_params(
                "confirm must be true before destructive cancellation",
                None,
            ));
        }
        let tunnel_id = Self::validated_tunnel_id(&params.tunnel_id)?;
        let capability = Self::validated_capability(&params.desktop_capability)?;
        let endpoint = self.endpoint(&format!("v1/tunnels/{tunnel_id}"))?;
        self.send(self.client.delete(endpoint).bearer_auth(capability))
            .await
    }
}

fn tool_success(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

fn tool_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = FileTunnelServer::from_environment()?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::FileTunnelServer;

    #[test]
    fn canonical_tunnel_ids_are_accepted() {
        assert!(
            FileTunnelServer::validated_tunnel_id("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
    }

    #[test]
    fn malformed_tunnel_ids_are_rejected() {
        assert!(FileTunnelServer::validated_tunnel_id("../../secrets").is_err());
        assert!(FileTunnelServer::validated_tunnel_id("550e8400e29b41d4a716446655440000").is_err());
    }

    #[test]
    fn capabilities_reject_whitespace() {
        assert!(FileTunnelServer::validated_capability("short").is_err());
        assert!(
            FileTunnelServer::validated_capability("aaaaaaaaaaaaaaaaaaaaaaaa aaaaaaaa").is_err()
        );
    }
}
