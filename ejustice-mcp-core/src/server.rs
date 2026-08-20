//! The MCP server itself: tool handlers (`search_case`,
//! `get_case_section`, `fetch_document`) and resource handlers for
//! reading a downloaded document by its `ejustice-document://` URI.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CacheScope, CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult,
        ListResourcesResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, ResourceContents, ResourceTemplate,
        ServerCapabilities, ServerInfo,
    },
    schemars::{self, JsonSchema},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::{
    CACHE_TTL_MS, DOCUMENT_URI_SCHEME,
    client::EjusticeClient,
    types::{DocumentsGridSection, GridSection},
};

/// Request parameters for the `search_case` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(
    description = "Search for a public court case by case number and return the full case \
    record: header fields (title, type, status, judge, filed by, urgent...), the relief claim \
    list, and every accordion section's rows (parties, legal practitioners, judges, hearings, \
    return of services, case history, interlocutories, documents)."
)]
pub struct MCPSearchCaseRequest {
    #[schemars(
        description = "The exact eJustice case number as it appears on the portal, including \
        slashes and dashes (e.g. 'HC-MD-CIV-MOT-GEN-2025/00343')."
    )]
    pub case_no: String,
}

/// Request parameters for the `get_case_section` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(
    description = "Re-fetch just one accordion section's rows for a case, without re-fetching \
    every other section — useful for a cheap refresh (e.g. checking for a new hearing) after an \
    initial search_case call."
)]
pub struct MCPGetCaseSectionRequest {
    #[schemars(
        description = "The exact eJustice case number as it appears on the portal, including \
        slashes and dashes (e.g. 'HC-MD-CIV-MOT-GEN-2025/00343')."
    )]
    pub case_no: String,

    #[schemars(
        description = "The accordion section/grid to fetch (e.g. 'documents' for file metadata \
        and fileIds, or 'hearings' for the hearing schedule)."
    )]
    pub section: GridSection,
}

/// Request parameters for the `fetch_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(
    description = "Download a specific court document (PDF, affidavit, etc.) by its resource URI."
)]
pub struct MCPFetchDocumentRequest {
    #[schemars(description = "The document's MCP resource URI, in the form \
        'ejustice-document://<fileId>' — copy it from the `uri` of a resource_link \
        returned by search_case or get_case_section, don't construct it by hand.")]
    pub file_url: String,
}

/// MCP server state: the eJustice HTTP client plus the generated tool router.
pub struct EjusticeMcpServer {
    /// HTTP client used to search cases and download documents from the
    /// eJustice portal.
    client: EjusticeClient,
    /// Routes incoming `tools/call` requests to the matching `#[tool]` method.
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EjusticeMcpServer {
    /// Builds a new server pointed at `base_url` (e.g.
    /// `https://ejustice.jud.na/ejustice`).
    ///
    /// # Panics
    ///
    /// Panics if the underlying HTTP client fails to build (see [`EjusticeClient::new`]).
    pub fn new(base_url: impl AsRef<str>) -> Self {
        let client = EjusticeClient::new(base_url).expect("Failed to build http client");
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search a Namibia eJustice case by case number and return the full case \
        record: header fields (title, type, status, judge, filed by, urgent...), the relief \
        claim list, and every accordion section's rows (parties, legal practitioners, judges, \
        hearings, return of services, case history, interlocutories) as JSON — plus one \
        resource_link per document in the case."
    )]
    async fn search_case(
        &self,
        Parameters(MCPSearchCaseRequest { case_no }): Parameters<MCPSearchCaseRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let case = match self.client.search_case(&case_no).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Case number '{}' not found in the eJustice portal. Please verify the exact formatting (e.g., 'HC-MD-CIV-MOT-GEN-2025/00343') and try again.",
                    case_no
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Failed to search for case '{}': {}",
                    case_no, e
                ))]));
            }
        };

        case.into_call_tool_result(&self.client.base_url)
    }

    #[tool(
        description = "Fetch a specific accordion section (e.g., hearings, documents) for a case. \
        Use this if you only need one specific part of the case record, or to check for updates \
        to a specific section without downloading the entire case again. For the `documents` \
        section, the response is a list of resource_links rather than JSON."
    )]
    async fn get_case_section(
        &self,
        Parameters(MCPGetCaseSectionRequest { case_no, section }): Parameters<
            MCPGetCaseSectionRequest,
        >,
    ) -> Result<CallToolResult, ErrorData> {
        match self.client.fetch_section(&case_no, section).await {
            Ok(Some(v)) => {
                if section == GridSection::Documents {
                    let docs: DocumentsGridSection = serde_json::from_value(v).map_err(|e| {
                        ErrorData::internal_error(
                            format!("Failed to parse documents section: {e}"),
                            None,
                        )
                    })?;

                    let content = docs
                        .resources(&self.client.base_url)
                        .into_iter()
                        .map(ContentBlock::resource_link)
                        .collect();

                    return Ok(CallToolResult::success(content));
                }

                let json = serde_json::to_string(&v)
                    .map_err(|e| ErrorData::internal_error(format!("serialize: {e}"), None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
            }
            Ok(None) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Case number '{}' not found in the eJustice portal.",
                case_no
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Failed to fetch '{}' section for case '{}': {}",
                section, case_no, e
            ))])),
        }
    }

    #[tool(
        description = "Download a specific court document (PDF, affidavit, etc.) from the eJustice \
        portal using its resource URI."
    )]
    async fn fetch_document(
        &self,
        Parameters(MCPFetchDocumentRequest { file_url }): Parameters<MCPFetchDocumentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(file_url) = file_url.strip_prefix(DOCUMENT_URI_SCHEME) else {
            return Err(ErrorData::invalid_params(
                format!(
                    "Unsupported resource URI '{}' - expected an '{DOCUMENT_URI_SCHEME}' URI.",
                    file_url
                ),
                None,
            ));
        };

        // Catch the common mistake of passing the numeric `id` instead of `fileId`.
        if file_url.chars().all(|c| c.is_ascii_digit()) && file_url.len() <= 10 {
            return Err(ErrorData::invalid_params(
                format!(
                    "'{file_url}' is a short numeric value, which suggests you passed a document \
                    row `id` instead of a `fileId`. `fileId` is a longer hex string (e.g. \
                    '350a0adf36b646db88b56fe18b05096b'). Copy it from the `fileId` field of a row \
                    in the documents section."
                ),
                None,
            ));
        }

        match self.client.fetch_document(file_url).await {
            Ok(Some(doc)) => {
                let blob = BASE64.encode(&doc.bytes);

                Ok(CallToolResult::success(vec![ContentBlock::resource(
                    ResourceContents::blob(blob, file_url).with_mime_type(doc.content_type),
                )]))
            }
            Ok(None) => Err(ErrorData::resource_not_found(
                format!(
                    "Document with file_url '{file_url}' not found. It may have been sealed, deleted, or the ID is incorrect.",
                ),
                None,
            )),
            Err(e) => Err(ErrorData::internal_error(
                format!("Failed to download document '{file_url}': {e}"),
                None,
            )),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EjusticeMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();

        info.server_info = Implementation::new("ejustice-mcp-server", "1.0.0")
            .with_title("Namibia eJustice MCP Bridge")
            .with_description("Provides AI agents with access to Namibia's public court records, case details, and document downloads.");

        info.instructions = Some(
            "Tools for Namibia's public eJustice case search (ejustice.jud.na). Start \
            with search_case for a full case record — it already includes every section, \
            including documents and their resource URIs. Use get_case_section to refresh one \
            section of an already-searched case. Use fetch_document with a document's resource \
            URI to download that file — or, if your client supports the MCP resources protocol, \
            call resources/read with the same URI instead; both return identical results."
                .to_string(),
        );

        info
    }

    /// Documents aren't enumerable ahead of time — they only exist in the
    /// context of a specific case, discovered via `search_case` /
    /// `get_case_section("documents")`. So this intentionally returns an empty
    /// list rather than trying to enumerate every document across every case in
    /// the portal.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![],
            ..Default::default()
        }
        .with_ttl_ms(CACHE_TTL_MS)
        .with_cache_scope(CacheScope::Public))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let Some(file_id) = request.uri.strip_prefix(DOCUMENT_URI_SCHEME) else {
            return Err(ErrorData::invalid_params(
                format!(
                    "Unsupported resource URI '{}' - expected an '{DOCUMENT_URI_SCHEME}' URI.",
                    request.uri
                ),
                None,
            ));
        };

        // Catch the common mistake of passing the numeric `id` instead of `fileId`.
        if file_id.chars().all(|c| c.is_ascii_digit()) && file_id.len() <= 10 {
            return Err(ErrorData::invalid_params(
                format!(
                    "'{file_id}' is a short numeric value, which suggests you passed a document \
                    row `id` instead of a `fileId`. `fileId` is a longer hex string (e.g. \
                    '350a0adf36b646db88b56fe18b05096b'). Copy it from the `fileId` field of a row \
                    in the documents section."
                ),
                None,
            ));
        }

        match self.client.fetch_document(file_id).await {
            Ok(Some(doc)) => {
                let blob = BASE64.encode(&doc.bytes);

                Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
                    vec![
                        ResourceContents::blob(blob, request.uri).with_mime_type(doc.content_type),
                    ],
                )))
            }
            Ok(None) => Err(ErrorData::resource_not_found(
                format!(
                    "Document with file_id '{file_id}' not found. It may have been sealed, deleted, or the ID is incorrect.",
                ),
                None,
            )),
            Err(e) => Err(ErrorData::internal_error(
                format!("Failed to download document '{file_id}': {e}"),
                None,
            )),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![ResourceTemplate::new(
                format!("{DOCUMENT_URI_SCHEME}{{fileId}}"),
                "court-document",
            )],
            ..Default::default()
        }
        .with_ttl_ms(CACHE_TTL_MS)
        .with_cache_scope(CacheScope::Public))
    }
}
