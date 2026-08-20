//! Data model for eJustice cases and documents, and the glue that turns
//! them into MCP tool/resource content.

use std::{collections::HashMap, fmt};

use rmcp::{
    ErrorData,
    model::{CallToolResult, ContentBlock, MetaObject, Resource},
    schemars::{self, JsonSchema},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document_uri;

/// A full eJustice case record, as scraped from a case-search page.
///
/// Serializes to the JSON text block returned by the `search_case` MCP
/// tool (see
/// [`into_call_tool_result`](CaseSearchDetail::into_call_tool_result))
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaseSearchDetail {
    /// The human-readable case number (e.g. `HC-MD-CIV-MOT-GEN-2025/00343`).
    #[serde(rename = "caseNumber")]
    pub case_no: String,

    #[serde(rename = "caseId")]
    pub dyna_form_id: String,

    #[serde(skip)]
    pub session_key: String,

    #[serde(skip)]
    pub current_version: String,

    #[serde(flatten)]
    pub fields: HashMap<String, String>,

    pub relief_claim: Vec<String>,

    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub grid_endpoints: HashMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub grids: Option<HashMap<String, Value>>,

    #[serde(skip)]
    pub csrf: Option<String>,
}

impl CaseSearchDetail {
    /// Builds the full MCP tool response for this case: the case data as
    /// a JSON text block, plus one `resource_link` content block per document.
    pub fn into_call_tool_result(mut self, base_url: &str) -> Result<CallToolResult, ErrorData> {
        let resources = self.take_document_resources(base_url);

        let json = serde_json::to_string(&self).map_err(|e| {
            ErrorData::internal_error(format!("Failed to serialize case data: {e}"), None)
        })?;

        let mut content = vec![ContentBlock::text(json)];
        content.extend(resources.into_iter().map(ContentBlock::resource_link));

        Ok(CallToolResult::success(content))
    }

    /// Pulls the `"Documents"` grid out of `grids` and tries to
    /// deserialize it as a [`DocumentsGridSection`].
    fn take_document_resources(&mut self, base_url: &str) -> Vec<Resource> {
        let Some(grids) = self.grids.as_mut() else {
            return Vec::new();
        };

        let key = GridSection::Documents.panel_title();

        let Some(raw) = grids.get(key) else {
            return Vec::new();
        };

        match serde_json::from_value::<DocumentsGridSection>(raw.clone()) {
            Ok(section) => {
                grids.remove(key);
                section.resources(base_url)
            }
            Err(_) => Vec::new(),
        }
    }
}

/// The eight accordion panels on a case page that load their rows via a
/// separate AJAX call. Maps 1:1 to `panel-title` text (see
/// [`panel_title`](GridSection::panel_title)), which is how
/// [`CaseSearchDetail::grid_endpoints`] keys them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "The specific dynamic section (Kendo grid) of the case to retrieve.")]
pub enum GridSection {
    /// The parties involved in the case (Applicants, Respondents, etc.).
    Parties,

    /// The legal practitioners (lawyers) representing the parties.
    LegalPractitioners,

    /// The judges assigned to the case and their approval history.
    Judges,

    /// Scheduled court hearings, dates, times, and locations.
    Hearings,

    /// Return of services (proof that documents were officially served to parties).
    ReturnOfServices,

    /// The internal workflow and task history of the case.
    CaseHistory,

    /// Interlocutory applications (sub-cases or interim motions).
    Interlocutories,

    /// Uploaded court documents (PDFs, affidavits, etc.). Comes back as
    /// resource links rather than a JSON blob — see
    /// [`DocumentsGridSection`].
    Documents,
}

impl GridSection {
    /// The exact panel-title text this section corresponds to on the eJustice page.
    pub fn panel_title(self) -> &'static str {
        match self {
            GridSection::Parties => "Case Parties",
            GridSection::LegalPractitioners => "Legal Practitioners",
            GridSection::Judges => "Case Judges",
            GridSection::Hearings => "Case Hearings",
            GridSection::ReturnOfServices => "Return of Services",
            GridSection::CaseHistory => "CASE HISTORY",
            GridSection::Interlocutories => "Interlocutories",
            GridSection::Documents => "Documents",
        }
    }
}

impl fmt::Display for GridSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.panel_title())
    }
}

/// The `"Documents"` accordion section's grid response, deserialized just
/// enough to build MCP resources from it.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentsGridSection {
    /// Total number of documents reported by the portal for this grid page.
    pub total: u32,
    /// The document rows themselves.
    pub data: Vec<CaseDocument>,
}

impl DocumentsGridSection {
    /// Converts every document row into an MCP [`Resource`] pointing at
    /// its `ejustice-document://` URI.
    pub fn resources(&self, base_url: &str) -> Vec<Resource> {
        self.data
            .iter()
            .map(|d| d.into_resource(base_url))
            .collect()
    }
}

/// One row of a case's `"Documents"` grid — an uploaded court document
/// (PDF, affidavit, etc.) and its metadata.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseDocument {
    pub id: i64,
    pub created_by: i64,
    pub created_by_user_name: String,
    pub created_date: String,
    pub updated_by: i64,
    pub updated_by_user_name: String,
    pub updated_date: String,
    pub version: i32,
    #[serde(rename = "type")]
    pub doc_type: DocType,
    pub composite_name: String,
    /// The long hex identifier used to download this document and to
    /// build its `ejustice-document://` resource URI (see
    /// [`crate::document_uri`]).
    pub file_id: String,
    #[serde(default)]
    pub description: Option<String>,
    pub file_name: String,
    pub system_file_name: String,
    pub file_extension: String,
    pub sub_folder: String,
    pub file_size: i64,
    pub content_type: String,
    pub uploaded_date: String,
    pub storage_type: String,
    pub display_name: String,
    pub document_group_key: String,
    pub document_key: String,
}

impl CaseDocument {
    /// Builds the MCP [`Resource`] an MCP client sees for this document row.
    pub fn into_resource(&self, base_url: &str) -> Resource {
        Resource::new(document_uri(&self.file_id), self.file_name.clone())
            .with_title(self.display_name.clone())
            .with_description(self.resource_description())
            .with_mime_type(self.content_type.clone())
            .with_size(self.file_size as u64)
            .with_meta(self.resource_meta(base_url))
    }

    /// Human-readable summary shown as the resource's description
    fn resource_description(&self) -> String {
        let mut parts = vec![format!("Document type: {}", self.doc_type.name)];

        if let Some(desc) = self.description.as_deref().filter(|d| !d.is_empty())
            && desc != self.file_name
        {
            parts.push(desc.to_string());
        }

        parts.join(". ")
    }

    /// Structured metadata mirroring
    /// [`resource_description`](CaseDocument::resource_description), for
    /// clients that want individual fields instead of parsing the
    /// description text.
    fn resource_meta(&self, base_url: &str) -> MetaObject {
        let mut meta = MetaObject::new();

        meta.insert(
            "directHttpsUrl".into(),
            serde_json::json!(format!("{}/download/document/{}", base_url, self.file_id)),
        );

        meta
    }
}

/// A document's type/category on the eJustice portal (e.g. "Affidavit",
/// "Notice of Motion") as assigned by the court.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocType {
    pub id: i64,
    pub name: String,
}

/// A downloaded document's raw bytes plus the metadata that came back
/// with the download response — as opposed to [`CaseDocument`], which is
/// the grid *listing* metadata, known before any download happens.
#[derive(Debug, Clone)]
pub struct DocumentBytes {
    pub bytes: bytes::Bytes,
    /// From the response's `Content-Type` header, or
    /// `"application/octet-stream"` if absent.
    pub content_type: String,
    /// From the response's `Content-Disposition` header's `filename=`
    /// parameter, if present.
    pub filename: Option<String>,
}
