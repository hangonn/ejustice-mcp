use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};

use ego_tree::NodeRef;
use regex::Regex;
use reqwest::{Client, header};
use scraper::{ElementRef, Html, Node, Selector};
use serde::Serialize;
use serde_json::Value;
use tokio::task::JoinSet;

use crate::types::{CaseSearchDetail, DocumentBytes, GridSection};

#[derive(Clone)]
pub struct EjusticeClient {
    client: Client,
    base_url: Arc<String>,
}

impl EjusticeClient {
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

    pub async fn search_case(&self, case_no: &str) -> Result<Option<CaseSearchDetail>, String> {
        let html = self.search_html(case_no).await?;
        let mut case = match parse_case_detail(&html, case_no).map_err(|err| {
            format!("Failed to parse case search detail for case number {case_no}: {err}",)
        })? {
            Some(c) => c,
            None => return Ok(None),
        };

        let grids = self.fetch_grids(&case).await?;
        case.grids = Some(grids);

        Ok(Some(case))
    }

    async fn fetch_one_grid(&self, path: &str, csrf: Option<&str>) -> Value {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "page": 1,
            "pageSize": 100,
            "skip": 0,
            "take": 100,
            "sort": [],
            "filter": {},
            "group": Value::Null,
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

    pub async fn fetch_section(
        &self,
        case_no: &str,
        section: GridSection,
    ) -> Result<Option<Value>, String> {
        let html = self.search_html(case_no).await?;

        let info = match parse_case_detail(&html, case_no) {
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
            .map_err(|err| format!("[Stream] Failed to read document body from {url}: {err}"))?;

        if bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(DocumentBytes {
            bytes,
            content_type,
            filename,
        }))
    }
}

// ~~~~ Case info public search form ~~~~

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

    // The rename attribute handles the brackets perfectly
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

// ~~~ Parse case detail ~~~~

pub fn parse_case_detail(html: &str, case_no: &str) -> Result<Option<CaseSearchDetail>, String> {
    let doc = Html::parse_document(html);

    let case_sel = Selector::parse(r#"p[data-static-name="caseno"]"#)
        .map_err(|e| format!("Selector parse error: {}", e))?;
    if doc.select(&case_sel).next().is_none() {
        tracing::debug!("Case number {case_no} not found in HTML");
        return Ok(None);
    }

    // Visible static fields
    let mut fields = HashMap::new();
    let static_sel = Selector::parse(r#"p.form-control-static[data-static-name]"#)
        .map_err(|e| format!("Selector parse error: {}", e))?;
    for el in doc.select(&static_sel) {
        if let Some(key) = el.value().attr("data-static-name") {
            fields.insert(key.to_string(), normalize_text(el));
        }
    }

    // Form metadata
    let dyna_form_id =
        extract_input(&doc, r#"input[name="dynaForm"]"#).ok_or("missing hidden input: dynaForm")?;
    let session_key = extract_input(&doc, r#"input[name="sessionKey"]"#)
        .ok_or("missing hidden input: sessionKey")?;
    let current_version =
        extract_input(&doc, r#"input[name="currentVersion"]"#).unwrap_or_default();
    let csrf = extract_input(&doc, r#"input[name="_csrf"]"#);

    // Relief claim list items
    let relief = parse_relief_claim(&doc)?;

    // Discover grid endpoints
    let grid_endpoints = discover_grid_endpoints(html, &doc)?;

    Ok(Some(CaseSearchDetail {
        case_no: case_no.to_string(),
        dyna_form_id,
        session_key,
        current_version,
        fields,
        relief_claim: relief,
        grid_endpoints,
        grids: None,
        csrf,
    }))
}

static ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r##"\$\("#(grid-[\w-]+|document-group-[\w-]+)"\)\.kendoGrid"##)
        .expect("ID_RE: hardcoded regex literal is malformed")
});

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"read:\s*\{\s*url:\s*(?:eJustice\.contextPath\s*\+\s*)?"([^"]+)""#)
        .expect("URL_RE: hardcoded regex literal is malformed")
});

fn normalize_text(el: ElementRef) -> String {
    el.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_input(doc: &Html, sel_str: &str) -> Option<String> {
    let sel = Selector::parse(sel_str).ok()?;
    doc.select(&sel)
        .next()?
        .value()
        .attr("value")
        .map(|s| s.trim().to_string())
}

fn parse_relief_claim(doc: &Html) -> Result<Vec<String>, String> {
    // 1. Locate the <p> anchor (the <ol> is a sibling of this <p> in the parsed DOM).
    let p_sel = Selector::parse(r#"p[data-static-name="casedetails"]"#)
        .map_err(|e| format!("Selector parse error: {e}"))?;
    let Some(p_el) = doc.select(&p_sel).next() else {
        return Ok(vec![]);
    };

    // 2. Go up to the parent <div> that holds both the <p> and the <ol>.
    let Some(container) = p_el.parent().and_then(ElementRef::wrap) else {
        return Ok(vec![]);
    };

    // 3. Find the first <ol> inside that container.
    let ol_sel = Selector::parse("ol").map_err(|e| format!("Selector parse error: {e}"))?;
    let Some(top_ol) = container.select(&ol_sel).next() else {
        return Ok(vec![]);
    };

    // 4. Collect text from every <li>, but stop recursing when we hit a nested <ol>.
    let li_sel = Selector::parse("li").map_err(|e| format!("Selector parse error: {e}"))?;
    Ok(top_ol
        .select(&li_sel)
        .map(li_text_excluding_nested_ol)
        .filter(|s| !s.is_empty())
        .collect())
}

/// Gather text inside an <li> but ignore any nested <ol> (and its children) completely.
fn li_text_excluding_nested_ol(li: ElementRef) -> String {
    let mut parts = Vec::new();
    for child in li.children() {
        parts.push(text_without_ol(child));
    }
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Recursive helper: collect text nodes, but return empty string for any <ol> subtree.
fn text_without_ol(node: NodeRef<'_, Node>) -> String {
    let mut parts = Vec::new();

    match node.value() {
        Node::Text(text) => {
            let t = text.trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
        }
        Node::Element(elem) => {
            if elem.name() == "ol" {
                return String::new(); // prune entire nested list
            }
            for child in node.children() {
                parts.push(text_without_ol(child));
            }
        }
        _ => {}
    }

    parts.join(" ")
}

/// Correlate panel titles with grid element IDs, then regex-match inline scripts
/// to find each grid's Kendo DataSource `read.url`.
fn discover_grid_endpoints(html: &str, doc: &Html) -> Result<HashMap<String, String>, String> {
    // Map grid ID -> panel title
    let panel_sel = Selector::parse(".panel").map_err(|e| format!("Selector parse error: {e}"))?;
    let title_sel =
        Selector::parse(".panel-title a").map_err(|e| format!("Selector parse error: {e}"))?;
    let grid_sel = Selector::parse(r#"[id^="grid-"], [id^="document-group-"]"#)
        .map_err(|e| format!("Selector parse error: {e}"))?;

    let mut id_to_title = HashMap::new();
    for panel in doc.select(&panel_sel) {
        let title = panel
            .select(&title_sel)
            .next()
            .map(normalize_text)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        for g in panel.select(&grid_sel) {
            if let Some(id) = g.value().attr("id") {
                id_to_title.insert(id.to_string(), title.clone());
            }
        }
    }

    let mut endpoints = HashMap::new();
    for block in html.split("<script>").skip(1) {
        let Some((code, _)) = block.split_once("</script>") else {
            continue;
        };
        let id = ID_RE.captures(code).map(|c| c[1].to_string());
        let url = URL_RE.captures(code).map(|c| c[1].to_string());
        if let (Some(id), Some(url)) = (id, url)
            && let Some(title) = id_to_title.get(&id)
        {
            endpoints.insert(title.clone(), url);
        }
    }
    Ok(endpoints)
}
