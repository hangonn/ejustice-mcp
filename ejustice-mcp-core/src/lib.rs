//! Core library for the eJustice MCP bridge: a scraping HTTP client for Namibia's
//! public eJustice court portal ([`client`]), the data model for case records and
//! documents ([`types`]), and the MCP server that exposes both as tools/resources
//! for AI agents ([`mcp`]).
//!
//! This crate is transport-agnostic — it has no opinion on *how* the MCP server is
//! served. Binary crates that depend on it choose the transport: `ejustice-mcp-local`
//! serves it over stdio, and `ejustice-mcp-http` (planned) will serve it over HTTP
//! using `axum`. Keeping the transport out of this crate means both binaries share
//! the exact same client, data model, and tool implementations.
//!
//! # Example
//!
//! ```rust,ignore
//! use ejustice_mcp_core::mcp::EjusticeMcpServer;
//! use rmcp::ServiceExt;
//!
//! let server = EjusticeMcpServer::new("https://ejustice.jud.na/ejustice");
//! let service = server.serve(rmcp::transport::stdio()).await.unwrap();
//! service.waiting().await.unwrap();
//! ```

/// The eJustice HTTP client: logs into the portal, searches for cases, fetches
/// accordion-section data, and downloads documents.
pub mod client;

/// The MCP server: wraps [`client::EjusticeClient`] in `rmcp`'s tool-router and
/// [`rmcp::ServerHandler`] traits so any MCP transport can serve it to an MCP client.
pub mod server;

/// The data model for case records, accordion sections, and documents, plus the
/// conversion from scraped portal JSON into MCP `Resource` values.
pub mod types;

mod parser;

/// URI scheme for eJustice documents exposed as MCP resources.
///
/// A full document resource URI is this scheme plus the document's `fileId`, e.g.
/// `ejustice-document://350a0adf36b646db88b56fe18b05096b`. Built in
/// [`types::CaseDocument::into_resource`] and parsed back apart in
/// [`mcp::EjusticeMcpServer::fetch_document`] and `read_resource`.
pub const DOCUMENT_URI_SCHEME: &str = "ejustice-document://";

/// Cache TTL, in milliseconds, advertised to MCP clients on `list_resources` and
/// `list_resource_templates` responses (see the `.with_ttl_ms(...)` calls in
/// [`mcp::EjusticeMcpServer`]). This is purely advisory metadata for the client —
/// it does not drive any caching on the server side.
pub const CACHE_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// Builds the full `ejustice-document://<fileId>` resource URI for a
/// document, given its `fileId` (see [`types::CaseDocument`]'s `file_id`
/// field).
pub fn document_uri(file_id: &str) -> String {
    format!("{DOCUMENT_URI_SCHEME}{file_id}")
}
