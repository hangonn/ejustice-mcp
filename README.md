# Namibia eJustice MCP Server

A **Model Context Protocol (MCP)** server, written in Rust, that gives AI agents — Claude Desktop, Cursor, Cline, or any other MCP-compatible client — direct access to the **Namibia eJustice public portal** (`ejustice.jud.na`). It searches court cases, pulls the full structured case record, and downloads filed documents, through tool calls instead of manual browsing.

## Available MCP Tools

| Tool Name | Parameters | Description | AI Workflow |
| :--- | :--- | :--- | :--- |
| **`search_case`** | `case_no` | Searches for a case by its exact case number and returns the **full case record** in one call: header fields (Judge, Status, Title, Filed By...), the Relief Claim list, and every accordion section's rows — parties, legal practitioners, judges, hearings, return of services, case history, interlocutories, **and documents (with each file's `file_id`)**. | **Step 1:** AI searches `"HC-MD-CIV-MOT-GEN-2025/00343"` and gets the whole case, including document `file_id`s, in one response. |
| **`get_case_section`** | `case_no`, `section` | Re-fetches just *one* section's rows on demand, without re-fetching every other section. Useful for a cheap refresh (e.g. checking for a new hearing) after an initial `search_case` — not required to reach documents, since `search_case` already includes them. Valid `section` values: `parties`, `legal_practitioners`, `judges`, `hearings`, `return_of_services`, `case_history`, `interlocutories`, `documents`. | **Step 2 (optional):** AI calls this with `section: "hearings"` to check for updates without re-pulling the whole case. |
| **`fetch_document`** | `file_id` | Downloads a case document (PDF, DOCX, whatever was filed) by its `file_id` and returns it as a base64-encoded resource plus a short text summary (filename, size, content type). | **Step 3:** AI passes a `file_id` from `search_case`'s `documents` section to download and read the actual filing. |

---

## Quick Start

You'll need an MCP-compatible client — Claude Desktop, Cursor, Cline, or similar.

### 1. Get the server

**Option A — Download a prebuilt binary (fastest, no Rust required)**
Grab the build for your OS — Windows, macOS, or Linux — from the [Releases page](https://github.com/hangonn/ejustice-mcp/releases).

**Option B — Build from source**
Requires [Rust](https://www.rust-lang.org/tools/install), a reasonably current **stable** toolchain (`rustup update` if you're on a distro-packaged `rustc` — this crate's dependencies need 2024-edition support).

```bash
git clone https://github.com/hangonn/ejustice-mcp.git
cd ejustice-mcp
cargo build --release --bin ejustice-mcp
```
*The compiled binary will be at `target/release/ejustice-mcp` (or `.exe` on Windows).*

### 2. Configure your MCP client
Add the server to your MCP client's configuration file, pointing `command` at wherever your binary ended up — the one you downloaded, or `target/release/ejustice-mcp` if you built from source.

**For Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ejustice": {
      "command": "/absolute/path/to/ejustice-mcp",
      "args": []
    }
  }
}
```

---

## Disclaimer & Ethical Usage

This tool is designed for **legal research, academic analysis, and personal use** to interact with *publicly available* court records.

*   **Rate Limiting:** Do not use this tool to scrape the entire database or run massive automated loops. It's built for single-case lookups, not bulk harvesting.
*   **Terms of Service:** Respect the terms of service of the Namibia Judiciary.
*   **No Legal Advice:** The AI's interpretation of downloaded legal documents does not constitute legal advice.