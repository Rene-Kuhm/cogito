# cogito-harness (Pi package)

Pi extension that wires [Cogito](https://github.com/Rene-Kuhm/cogito-harness) into
[Pi](https://pi.dev) and Gentle Shell.

## What it does

1. **Puts a project card in the system prompt.** At session start it loads a
   ~250-token summary (modules, busiest files, how to look code up),
   rescanning only if `HEAD` moved, and adds it to every turn's system prompt
   as `<cogito_project>`. It is identical between turns, so the provider
   serves it from its prompt cache, and the model starts each answer knowing
   the layout without spending a tool call on orientation.
2. **Guardrails.** The first time the agent reads a large file (250+ lines)
   whole, it gets Cogito's outline instead; the first time it greps or finds a
   plain name, it gets Cogito's search result. Repeating the exact same call
   goes through, so the agent is never stuck; it is only shown the cheap
   answer first. On a 708-line README, reading it whole ~11,100 tokens →
   outline ~530; `grep README **/*.md` ~4,600 → search ~400.
3. **Shows status.** The status line reports whether Cogito is ready for the
   current project.
4. **Adds `/cogito`.**

   ```
   /cogito status          what Cogito sees in this project
   /cogito map             the project map the agent reads
   /cogito scan            regenerate the code fingerprint
   /cogito search <text>   find symbols, files and modules by words
   /cogito stats           what the index holds
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
(`project_map`, `search_code`, `file_outline`, `related_files`)
come from Pi's
built-in MCP and are registered by `cogito setup`, which writes:

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
