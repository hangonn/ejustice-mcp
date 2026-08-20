# eJustice MCP

A **Model Context Protocol (MCP)** server that connects AI agents, including ChatGPT Desktop, Claude Desktop, Goose, and other MCP-compatible clients, to the **Namibia eJustice public portal** (`ejustice.jud.na`). Search for a case by its exact case number, retrieve structured case details and individual sections, and access filed documents through MCP resource links. It turns the portal into a focused interface for legal research without requiring manual browsing.

## Available MCP Tools

| Tool Name | Parameters | Description | AI Workflow |
| :--- | :--- | :--- | :--- |
| **`search_case`** | `case_no` | Searches for a case by its exact case number and returns the **full case record** in one call: header fields (Judge, Status, Title, Filed By...), the Relief Claim list, and every non-document accordion section's rows, including parties, legal practitioners, judges, hearings, return of services, case history, and interlocutories. Each filed document is returned as an MCP `resource_link` with its name, MIME type, metadata, and `ejustice-document://<fileId>` URI. | **Step 1:** AI searches `"HC-MD-CIV-MOT-GEN-2025/00343"` and receives the case record plus document resource links. |
| **`get_case_section`** | `case_no`, `section` | Re-fetches just *one* section's rows on demand, without re-fetching every other section. Useful for a cheap refresh (e.g. checking for a new hearing) after an initial `search_case`. Valid `section` values: `parties`, `legalPractitioners`, `judges`, `hearings`, `returnOfServices`, `caseHistory`, `interlocutories`, `documents`. The `documents` section returns resource links rather than JSON rows. | **Step 2 (optional):** AI calls this with `section: "hearings"` to check for updates, or `section: "documents"` to refresh document links. |
| **`fetch_document`** | `file_url` (the exact `ejustice-document://<fileId>` URI from a document `resource_link`) | Downloads the document from the eJustice portal and returns its contents as a base64-encoded MCP resource, preserving the document MIME type. The URI must be copied from the resource link; do not pass the row's numeric `id` or construct a URL manually. | **Step 3:** AI passes the document resource link's `uri` to download and read the filing. Clients that support MCP resources can instead call `resources/read` with the same URI. |

---

## Quick Start

You'll need an MCP-compatible client, such as ChatGPT Desktop, Claude Desktop, or Goose.

### 1. Get the server

**Option A: Download a prebuilt binary (fastest, no Rust required)**
Grab the build for your OS (Windows, macOS, or Linux) from the [Releases page](https://github.com/hangonn/ejustice-mcp/releases).

**Option B: Build from source**
Requires [Rust](https://www.rust-lang.org/tools/install), a reasonably current **stable** toolchain (`rustup update` if you're on a distro-packaged `rustc`; this crate's dependencies need 2024-edition support).

```bash
git clone https://github.com/hangonn/ejustice-mcp.git
cd ejustice-mcp
cargo build --release -p ejustice-mcp-local
```
*The compiled binary will be at `target/release/ejustice-mcp-local` (or `.exe` on Windows).*

### 2. Configure your MCP client
Add the server to your MCP client's configuration file, pointing `command` at wherever your binary ended up: the one you downloaded, or `target/release/ejustice-mcp-local` if you built from source.

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

The server uses `https://ejustice.jud.na/ejustice` by default. To point it at another compatible deployment, set the `EJUSTICE_BASE_URL` environment variable in your MCP client's server configuration.

---

## Disclaimer & Ethical Usage

This project was created while learning the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/docs/getting-started/intro) and continues to evolve. Support may vary across clients and use cases, so review and test it in your target environment before wider adoption.

This tool is designed for **legal research, academic analysis, and personal use** to interact with *publicly available* court records.

*   **Rate Limiting:** Do not use this tool to scrape the entire database or run massive automated loops. It's built for single-case lookups, not bulk harvesting.
*   **Terms of Service:** Respect the terms of service of the Namibia Judiciary.
*   **No Legal Advice:** The AI's interpretation of downloaded legal documents does not constitute legal advice.