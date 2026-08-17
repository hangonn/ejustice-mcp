use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ProtocolVersion, ResourceContents,
        ServerCapabilities, ServerInfo,
    },
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::{client::EjusticeClient, types::GridSection};

/// Parameters for `search_case`.
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

/// Parameters for `get_case_section`.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(
    description = "Re-fetch just one accordion section's rows for a case, without re-fetching \
    every other section - useful for a cheap refresh (e.g. checking for a new hearing) after an \
    initial search_case call. Not required to reach documents: search_case already returns \
    every section, including documents, on the first call."
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

/// Parameters for `fetch_document`.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(
    description = "Download a specific court document (PDF, affidavit, etc.) by its file ID and \
    return it as a base64-encoded resource in the tool result - nothing is written to the \
    server's filesystem."
)]
pub struct MCPFetchDocumentRequest {
    #[schemars(
        description = "The document's file ID - the `fileId` field on a row from the \
        'documents' section returned by search_case or get_case_section. This is a long hex \
        string (e.g. '350a0adf36b646db88b56fe18b05096b'). Do NOT use the row's `id` field \
        (a short numeric value, e.g. '1611669') - that will not resolve to a file."
    )]
    pub file_id: String,
}

/// MCP server state: the eJustice HTTP client plus the generated tool router.
pub struct EjusticeMcpServer {
    client: EjusticeClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EjusticeMcpServer {
    /// Build a new server pointed at `base_url` (e.g. `https://ejustice.jud.na/ejustice`).
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
        hearings, return of services, case history, interlocutories, documents). Each documents \
        row includes a `fileId` field - pass that to fetch_document to download the file."
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

        let json = serde_json::to_string(&case).map_err(|e| {
            ErrorData::internal_error(format!("Failed to serialize case data: {e}"), None)
        })?;

        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Fetch a specific accordion section (e.g., hearings, documents) for a case. \
        Use this if you only need one specific part of the case record, or to check for updates \
        to a specific section without downloading the entire case again."
    )]
    async fn get_case_section(
        &self,
        Parameters(MCPGetCaseSectionRequest { case_no, section }): Parameters<
            MCPGetCaseSectionRequest,
        >,
    ) -> Result<CallToolResult, ErrorData> {
        match self.client.fetch_section(&case_no, section).await {
            Ok(Some(v)) => {
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
        portal using its file ID.\n\n\
        IMPORTANT: Use the `fileId` field (a long hex string, e.g. \
        '350a0adf36b646db88b56fe18b05096b'), NOT the `id` field (a short numeric row ID, e.g. \
        '1611669'). Both appear on each row in the 'documents' section returned by search_case / \
        get_case_section, but only `fileId` will resolve to an actual file. Passing the numeric \
        `id` will fail."
    )]
    async fn fetch_document(
        &self,
        Parameters(MCPFetchDocumentRequest { file_id }): Parameters<MCPFetchDocumentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        // fileId is a 32-char hex string; catch the common mistake of passing the numeric `id` instead.
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

        match self.client.fetch_document(&file_id).await {
            Ok(Some(doc)) => {
                let name = doc
                    .filename
                    .clone()
                    .unwrap_or_else(|| format!("document_{}", file_id));
                let blob = BASE64.encode(&doc.bytes);

                let summary = format!(
                    "Successfully downloaded '{}' ({} bytes, {}). The file is attached as a resource.",
                    name,
                    doc.bytes.len(),
                    doc.content_type
                );

                let uri = format!("ejustice-document://{file_id}");

                Ok(CallToolResult::success(vec![
                    ContentBlock::text(summary),
                    ContentBlock::resource(
                        ResourceContents::blob(blob, uri).with_mime_type(doc.content_type),
                    ),
                ]))
            }
            Ok(None) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Document with file_id '{file_id}' not found. It may have been sealed, deleted, or the ID is incorrect.",
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Failed to download document '{file_id}': {e}",
            ))])),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EjusticeMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();

        info.server_info = Implementation::new("ejustice-mcp-server", "1.0.0")
            .with_title("Namibia eJustice MCP Bridge")
            .with_description("Provides AI agents with access to Namibia's public court records, case details, and document downloads.");

        info.instructions = Some(
            "Tools for Namibia's public eJustice case search (ejustice.jud.na). Start \
            with search_case for a full case record - it already includes every section, \
            including documents and their fileIds. Use get_case_section to refresh one section \
            of an already-searched case. Use fetch_document with a fileId from the documents \
            section to download that file."
                .to_string(),
        );
        info
    }
}
