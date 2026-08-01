//! ftnl-mcp-server — an MCP server (stdio) for the **File Tunnel** org
//! (github.com/file-tunnel): secure, ephemeral, QR-connected cross-device file
//! transfers (a desktop upload field paired to files that live on a phone).
//!
//! Tool families:
//! - org/git navigation over the org root (`FTNL_ROOT`),
//! - cargo build/check runners for the Rust services (build-only),
//! - tunnel/transfer API health & route probes + an architecture doc,
//! - interface-contract listing + repo/release inventory + monorepo pins,
//! - org CI status and the github.io marketing-site check,
//! - registrar-agnostic domain tooling (DoH / RDAP / TLS / Cloudflare),
//! - read-only Supabase client-telemetry debugging tools.
//!
//! All tools are read-only or build-only, per the org AGENTS.md safety rules
//! (no deletes, no moves, no git history rewrites, no transfer mutation). stdout
//! is the MCP wire; all logging goes to stderr.

pub mod builds;
pub mod cloudflare;
pub mod docs;
pub mod domains;
pub mod fiducia;
pub mod github;
pub mod interfaces;
pub mod prompts;
pub mod resources;
pub mod server;
pub mod status;
pub mod supabase;
pub mod telemetry;
pub mod transfer;
pub mod util;
