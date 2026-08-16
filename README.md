# Namibia eJustice MCP Server

An advanced, production-ready **Model Context Protocol (MCP)** server written in Rust. It bridges AI agents (like Claude Desktop, Cursor, and Cline) with the **Namibia eJustice public portal** (`ejustice.jud.na`), allowing them to search court cases, extract dynamic grid data, and download legal documents securely and efficiently.

## Available MCP Tools

| Tool Name | Parameters | Description | AI Workflow |
| :--- | :--- | :--- | :--- |
| **`search_case`** | `case_no` | Searches for a case by its exact case number and returns the **full case record** in one call: header fields (Judge, Status, Title, Filed By...), the Relief Claim list, and every accordion section's rows — parties, legal practitioners, judges, hearings, return of services, case history, interlocutories, **and documents (with each file's `file_id`)**. | **Step 1:** AI searches `"HC-MD-CIV-MOT-GEN-2025/00343"` and gets the whole case, including document `file_id`s, in one response. |
| **`get_case_section`** | `case_no`, `section` | Re-fetches just *one* section's rows on demand, without re-fetching every other section. Useful for a cheap refresh (e.g. checking for a new hearing) after an initial `search_case` — not required to reach documents, since `search_case` already includes them. Valid `section` values: `parties`, `legal_practitioners`, `judges`, `hearings`, `return_of_services`, `case_history`, `interlocutories`, `documents`. | **Step 2 (optional):** AI calls this with `section: "hearings"` to check for updates without re-pulling the whole case. |
| **`fetch_document`** | `file_id` | Downloads a case document (PDF, DOCX, whatever was filed) by its `file_id` and returns it as a base64-encoded resource plus a short text summary (filename, size, content type). | **Step 3:** AI passes a `file_id` from `search_case`'s `documents` section to download and read the actual filing. |

---

## Quick Start

### Prerequisites
*   [Rust](https://www.rust-lang.org/tools/install), a reasonably current **stable** toolchain (`rustup update` if you're on a distro-packaged `rustc` — this crate's dependencies need 2024-edition support)
*   An MCP-compatible client (Claude Desktop, Cursor, Cline, etc.)

### 1. Build the Server
Clone the repository and build the release binary:

```bash
git clone https://github.com/yourusername/ejustice-mcp.git
cd ejustice-mcp
cargo build --release --bin ejustice-mcp
```
*The compiled binary will be at `target/release/ejustice-mcp` (or `.exe` on Windows).*

### 2. Configure your MCP Client
Add the server to your MCP client's configuration file.

**For Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ejustice": {
      "command": "/absolute/path/to/ejustice-mcp/target/release/ejustice-mcp",
      "args": []
    }
  }
}
```

**For Cursor** (`.cursor/mcp.json`):
```json
{
  "mcpServers": {
    "ejustice": {
      "command": "/absolute/path/to/ejustice-mcp/target/release/ejustice-mcp",
      "args": []
    }
  }
}
```
*(Make sure to replace `/absolute/path/to/...` with the actual path to your compiled binary!)*

### 3. (Optional) Point it at a different eJustice instance
The server defaults to `https://ejustice.jud.na/ejustice`. Override it with `EJUSTICE_BASE_URL`, e.g. in the client config:
```json
{
  "mcpServers": {
    "ejustice": {
      "command": "/absolute/path/to/ejustice-mcp/target/release/ejustice-mcp",
      "args": [],
      "env": { "EJUSTICE_BASE_URL": "https://ejustice.jud.na/ejustice" }
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