use std::{collections::HashMap, fmt};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaseSearchDetail {
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

/// The eight accordion panels on a case page that load their rows via a separate AJAX
/// call. Maps 1:1 to `panel-title` text, which is how `grid_endpoints` keys them.
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

    /// Uploaded court documents (PDFs, affidavits, etc.). Each row's `fileId` field can be
    /// passed to `fetch_document` to download that file.
    Documents,
}

impl GridSection {
    /// Exact panel-title text this section corresponds to on the page.
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

#[derive(Debug, Clone)]
pub struct DocumentBytes {
    pub bytes: bytes::Bytes,
    pub content_type: String,
    pub filename: Option<String>,
}
