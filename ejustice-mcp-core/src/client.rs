//! HTTP client for Namibia's eJustice public case-search portal.

use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::{Client, header};
use serde::Serialize;
use serde_json::Value;
use tokio::task::JoinSet;

use crate::{
    parser,
    types::{CaseSearchDetail, DocumentBytes, GridSection},
};

/// HTTP client for Namibia's eJustice public case-search portal.
#[derive(Clone)]
pub struct EjusticeClient {
    client: Client,
    /// The eJustice deployment this client talks to, e.g.
    /// `https://ejustice.jud.na/ejustice`. Trailing slashes are stripped
    /// where it matters when building request URLs.
    pub base_url: Arc<String>,
}

impl EjusticeClient {
    /// Builds a client pointed at `base_url` (e.g.
    /// `https://ejustice.jud.na/ejustice`).
    ///
    /// # Errors
    ///
    /// Returns `Err` (as a `String`) if the default headers can't be
    /// built or the underlying `reqwest::Client` fails to construct.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, String> {
        let base_url = base_url.as_ref().to_string();
        let mut headers = header::HeaderMap::new();

        let accept: header::HeaderValue =
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                .parse()
                .map_err(|e| format!("invalid Accept header value: {e}"))?;
        headers.insert(header::ACCEPT, accept);

        let accept_language: header::HeaderValue = "en-US,en;q=0.9"
            .parse()
            .map_err(|e| format!("invalid Accept-Language header value: {e}"))?;
        headers.insert(header::ACCEPT_LANGUAGE, accept_language);

        let builder = Client::builder()
            .default_headers(headers)
            .cookie_store(true)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(60))
            .tls_danger_accept_invalid_certs(true);

        let client = builder
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        Ok(Self {
            client,
            base_url: Arc::new(base_url),
        })
    }

    /// Looks up a case by its exact case number and returns the full case
    /// record: header fields, relief claim list, and every accordion
    /// section's rows (all fetched concurrently via
    /// [`fetch_grids`](EjusticeClient::fetch_grids)).
    ///
    /// Returns `Ok(None)` if `case_no` doesn't match any case on the
    /// portal — a not-found case number is an expected, non-error
    /// outcome here, not something that should short-circuit the caller
    /// with an `Err`.
    pub async fn search_case(&self, case_no: &str) -> Result<Option<CaseSearchDetail>, String> {
        let html = self.search_html(case_no).await?;
        let mut case = match parser::parse_case_detail(&html, case_no).map_err(|err| {
            format!("Failed to parse case search detail for case number {case_no}: {err}",)
        })? {
            Some(c) => c,
            None => return Ok(None),
        };

        let grids = self.fetch_grids(&case).await?;
        case.grids = Some(grids);

        Ok(Some(case))
    }

    /// Re-searches `case_no` and fetches just one accordion section's
    /// rows, without fetching every other section.
    ///
    /// Returns `Ok(None)` if the case isn't found, or if the case's page
    /// doesn't have an endpoint for `section` at all (e.g. a case with no
    /// hearings yet may not render a hearings grid).
    pub async fn fetch_section(
        &self,
        case_no: &str,
        section: GridSection,
    ) -> Result<Option<Value>, String> {
        let html = self.search_html(case_no).await?;

        let info = match parser::parse_case_detail(&html, case_no) {
            Ok(Some(c)) => c,
            Ok(None) => return Ok(None),
            Err(err) => {
                return Err(format!(
                    "Failed to parse case search detail for {case_no}: {err}"
                ));
            }
        };

        let Some(path) = info.grid_endpoints.get(section.panel_title()) else {
            return Ok(None);
        };

        Ok(Some(self.fetch_one_grid(path, info.csrf.as_deref()).await))
    }

    /// Downloads a document's raw bytes by its `fileId`.
    ///
    /// Returns `Ok(None)` if the server responds `404`, or if it responds
    /// successfully but with an empty body — treated the same as "not
    /// found" rather than returned as a zero-byte file.
    pub async fn fetch_document(&self, file_id: &str) -> Result<Option<DocumentBytes>, String> {
        let url = format!("{}/document/download/{}", self.base_url, file_id);

        let resp =
            self.client.get(&url).send().await.map_err(|err| {
                format!("[Network] Failed to initiate download from {url} : {err}")
            })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let resp = resp.error_for_status().map_err(|err| {
            let status_code = err
                .status()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            format!("[HTTP] Server rejected download for {url} with status {status_code}: {err}")
        })?;

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let filename = resp
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(|cd| {
                cd.split(';').find_map(|part| {
                    let part = part.trim();
                    part.strip_prefix("filename=")
                        .map(|f| f.trim_matches('"').to_string())
                })
            });

        let bytes = resp
            .bytes()
            .await
            .map_err(|err| format!("[Stream] Failed to read document body from {url}: {err:?}"))?;

        if bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(DocumentBytes {
            bytes,
            content_type,
            filename,
        }))
    }

    /// POSTs `case_no` to the public case-search endpoint and returns the
    /// raw HTML of the resulting page.
    ///
    /// This is the first step of every public entry point on this client
    /// ([`search_case`](EjusticeClient::search_case) and
    /// [`fetch_section`](EjusticeClient::fetch_section) both call it)
    async fn search_html(&self, case_no: &str) -> Result<String, String> {
        let search_url = format!(
            "{}/f/caseinfo/publicsearch",
            self.base_url.trim_end_matches('/')
        );

        let resp = self
            .client
            .post(&search_url)
            .form(&CaseInfoSearchForm::new(case_no))
            .send()
            .await
            .map_err(|err| {
                let reason = if err.is_connect() {
                    "connection refused/failed"
                } else if err.is_timeout() {
                    "request timed out"
                } else if err.is_redirect() {
                    "redirect loop/failure"
                } else {
                    "network error"
                };
                format!("[Network] POST to {search_url} failed ({reason}): {err}")
            })?;

        let resp = resp
        .error_for_status()
        .map_err(|err| {
            let status_code = err.status()
                .map(|s| s.as_u16().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let hint = if status_code == "403" || status_code == "400" {
                " (Hint: Likely invalid CSRF token, missing session cookie, or blocked by WAF)"
            } else {
                ""
            };

            format!("[HTTP] Server rejected search POST to {search_url} with status {status_code}{hint}: {err}")
        })?;

        resp.text().await.map_err(|err| {
            format!("[Stream] Failed to read HTML response body from {search_url}: {err}")
        })
    }

    /// Fetches one Kendo grid's rows as raw JSON.
    ///
    /// Errors are represented as JSON, not as `Err`: network failures,
    /// non-2xx responses, and JSON-parse failures all come back as
    /// `Ok(Value)` shaped like `{"error": "...", "message": "..."}` (HTTP
    /// errors also include `status_code` and a truncated
    /// `raw_response_snippet`).
    async fn fetch_one_grid(&self, path: &str, csrf: Option<&str>) -> Value {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            // "page": 1,
            // "pageSize": 100,
            // "skip": 0,
            // "take": 100,
            // "sort": [],
            // "filter": {},
            // "group": Value::Null,
        });

        let mut req = self
            .client
            .post(&url)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Content-Type", "application/json")
            .json(&body);

        if let Some(token) = csrf {
            req = req.header("X-CSRF-TOKEN", token);
        }

        let resp = match req.send().await {
            Ok(resp) => resp,
            Err(err) => {
                let reason = if err.is_connect() {
                    "connection refused/failed"
                } else if err.is_timeout() {
                    "request timed out"
                } else {
                    "network error"
                };
                return serde_json::json!({
                    "error": "network_failure",
                    "message": format!("[Network] POST to {url} failed ({reason}): {err}")
                });
            }
        };

        let status = resp.status();
        if !status.is_success() {
            // Read the raw text to help debug WAF blocks or HTML error pages
            let raw_text = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("Failed to read error body: {e}"));

            return serde_json::json!({
                "error": "http_error",
                "status_code": status.as_u16(),
                "message": format!("[HTTP] Server rejected request to {url} with status {status}"),
                // Truncate the snippet to prevent blowing up the AI's context window
                "raw_response_snippet": raw_text.chars().take(500).collect::<String>()
            });
        }

        match resp.json::<Value>().await {
            Ok(v) => v,
            Err(err) => {
                serde_json::json!({
                    "error": "parse_error",
                    "message": format!("[Parse] Failed to parse JSON response from {url}: {err}")
                })
            }
        }
    }

    /// Fetches every grid in `record.grid_endpoints` concurrently — one
    /// task per grid, via a `JoinSet` — and returns panel title -> raw
    /// JSON rows.
    ///
    /// # Errors
    ///
    /// Only errors if a fetch *task itself* panics. A failed HTTP
    /// request for one grid does not fail the whole batch — see
    /// [`fetch_one_grid`](EjusticeClient::fetch_one_grid).
    async fn fetch_grids(
        &self,
        record: &CaseSearchDetail,
    ) -> Result<HashMap<String, Value>, String> {
        let mut set = JoinSet::new();

        for (panel, path) in &record.grid_endpoints {
            let this = self.clone();
            let panel = panel.clone();
            let path = path.clone();
            let csrf = record.csrf.clone();
            set.spawn(async move {
                let result = this.fetch_one_grid(&path, csrf.as_deref()).await;
                (panel, result)
            });
        }

        let mut out = HashMap::new();
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok((panel, value)) => {
                    out.insert(panel, value);
                }
                Err(e) => {
                    return Err(format!("A grid fetch task panicked: {e}"));
                }
            }
        }

        Ok(out)
    }
}

/// Form body for the public case-search POST
/// (`{base_url}/f/caseinfo/publicsearch`).
#[derive(Serialize, Debug)]
struct CaseInfoSearchForm<'a> {
    page_refresh: &'a str,
    page_draft: &'a str,

    #[serde(rename = "dynaForm")]
    dyna_form: &'a str,

    #[serde(rename = "currentVersion")]
    current_version: i32,

    #[serde(rename = "sessionKey")]
    session_key: &'a str,

    #[serde(rename = "attributes[caseNo_search]")]
    case_no_search: &'a str,

    #[serde(rename = "attributes[caseNo_search_history]")]
    case_no_search_history: &'a str,

    case_parties: &'a str,
    case_lawyers: &'a str,

    #[serde(rename = "_csrf")]
    csrf: &'a str,
}

impl<'a> CaseInfoSearchForm<'a> {
    /// Builds the form body for searching `case_no`.
    pub fn new(case_no: &'a str) -> Self {
        Self {
            page_refresh: "true",
            page_draft: "",
            dyna_form: "",
            current_version: 0,
            session_key: "",
            case_no_search: case_no,
            case_no_search_history: "",
            case_parties: "grid-readonly-15",
            case_lawyers: "grid-readonly-31",
            csrf: "",
        }
    }
}
