use std::{collections::HashMap, sync::LazyLock};

use ego_tree::NodeRef;
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};

use crate::types::CaseSearchDetail;

/// Parses a case-search HTML response into a [`CaseSearchDetail`].
///
/// Returns `Ok(None)` (not an error) if the page doesn't contain a
/// `p[data-static-name="caseno"]` element — the signal this crate uses to
/// tell "no such case" apart from a real case page.
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

/// Matches a Kendo grid element's `id` out of an inline `<script>` block,
/// e.g. `$("#grid-readonly-15").kendoGrid(...)`.
static ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r##"\$\("#(grid-[\w-]+|document-group-[\w-]+)"\)\.kendoGrid"##)
        .expect("ID_RE: hardcoded regex literal is malformed")
});

/// Matches a Kendo grid's `DataSource.transport.read.url` out of an
/// inline `<script>` block, e.g.
/// `read: { url: eJustice.contextPath + "/f/..." }` (the
/// `eJustice.contextPath +` prefix is optional/ignored by the pattern).
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"read:\s*\{\s*url:\s*(?:eJustice\.contextPath\s*\+\s*)?"([^"]+)""#)
        .expect("URL_RE: hardcoded regex literal is malformed")
});

/// Collapses an element's text content to a single line with normalized
/// (single-space) whitespace.
fn normalize_text(el: ElementRef) -> String {
    el.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads the `value` attribute of the first element matching `sel_str` —
/// used for the hidden `<input>` fields that carry session state
/// (`dynaForm`, `sessionKey`, `_csrf`, ...).
fn extract_input(doc: &Html, sel_str: &str) -> Option<String> {
    let sel = Selector::parse(sel_str).ok()?;
    doc.select(&sel)
        .next()?
        .value()
        .attr("value")
        .map(|s| s.trim().to_string())
}

/// Extracts the numbered relief-claim list (`<ol><li>...`) from the case
/// details panel, if present. Returns an empty `Vec` (not an error) if
/// the case has no relief claim section.
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
///
/// This is how every accordion section's AJAX endpoint is discovered —
/// there's no static list of them anywhere, since which panels exist (and
/// their exact grid element ids) can vary per case.
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
