# ejustice-mcp-local

The stdio-transport binary for the eJustice MCP server. Point a local MCP client (Claude
Desktop, Claude Code, or any other client that spawns MCP servers as subprocesses) at this.


## Running

```sh
cargo run -p ejustice-mcp-local
```

## Configuring an MCP client

Example client config (adjust for your client):

```jsonc
{
  "mcpServers": {
    "ejustice": {
      "command": "path/to/ejustice-mcp-local"
    }
  }
}
```