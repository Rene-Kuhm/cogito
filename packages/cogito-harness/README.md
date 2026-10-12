# cogito-harness (Pi package)

Pi extension that wires [Cogito](https://github.com/Rene-Kuhm/cogito-harness) into
[Pi](https://pi.dev) and Gentle Shell.

## What it does

1. **Scans at session start.** The project's code fingerprint is generated in
   the background so the first query is fast. The scan never blocks the
   session and is capped at 30 seconds.
2. **Shows status.** The status line reports whether Cogito is ready for the
   current project.
3. **Adds `/cogito`.**

   ```
   /cogito status          what Cogito sees in this project
   /cogito scan            regenerate the code fingerprint
   /cogito query <text>    search tags and architectural decisions
   /cogito stats           stored tag/decision counts
   /cogito doctor          full diagnostics incl. agent integrations
   ```

## Install

```bash
pi install npm:cogito-harness
```

Or let Cogito do it:

```bash
cogito setup gentle-shell
```

## MCP tools

This package is the *session* adapter. The MCP tools
(`scan_project`, `compress_context`, `search_knowledge`, `add_decision`,
`add_tag`, `project_stats`) come from Pi's built-in MCP and are registered by
`cogito setup`, which writes:

```json
{
  "mcpServers": {
    "cogito": {
      "command": "cogito",
      "args": ["mcp"]
    }
  }
}
```

## Requirements

The `cogito` binary must be on `PATH`. Set `COGITO_BIN` to point at a specific
build:

```bash
export COGITO_BIN=/path/to/cogito
```
