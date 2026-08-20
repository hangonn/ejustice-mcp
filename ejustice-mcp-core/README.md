# ejustice-mcp-core

Library crate: the eJustice scraping client, the case/document data
model, and the MCP tool/resource handlers. Not a binary — there's nothing
to run here directly. See `ejustice-mcp-local` (stdio) or
`ejustice-mcp-http` (HTTP) for something you can actually
launch.

## Tools this crate exposes over MCP

| Tool | Parameters | Notes |
|---|---|---|
| `search_case` | `case_no` | Returns header fields, relief claims, and every accordion section as JSON, plus one `resource_link` per document. |
| `get_case_section` | `case_no`, `section` | Cheap refresh of a single section. Not required to reach documents — `search_case` already includes them. |
| `fetch_document` | `file_url` (a full `ejustice-document://<fileId>` resource URI) | Downloads the document's bytes. |

## Resources

Documents are also directly readable as MCP resources:
`ejustice-document://<fileId>` (see the crate-root `document_uri`
function for how these are built). `fileId` values come from a
document's row in the `documents` accordion section — **not** the row's
numeric `id`, which is a different, shorter value; both tools reject an
`id` passed by mistake with a pointer to the right field.
