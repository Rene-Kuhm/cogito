# Editor & Agent Integrations

Cogito is editor- and agent-agnostic. It works with anything that can call
a CLI, expose environment variables, or speak JSON-RPC over stdio.

This document shows how to wire Cogito into the most common tools.

## Contents

- [Gentle Shell (pi)](#gentle-shell-pi) — first-class
- [Claude Code](#claude-code) — via MCP
- [Cursor](#cursor) — via MCP
- [Continue.dev (VSCode)](#continuevscode) — via MCP
- [Neovim](#neovim) — via shell call
- [Aider](#aider) — via shell command
- [Zed](#zed) — via MCP
- [Generic agent](#generic-agent) — REST API
- [Pre-commit hook](#pre-commit-hook) — git integration
- [GitHub Actions](#github-actions) — CI integration

---

## Gentle Shell (pi)

Cogito ships a first-class skill for Pi. Drop the contents of
`skills/cogito/` into your Pi skills directory:

```bash
# User-wide
cp -r skills/cogito ~/.gentle-shell/skills/

# Or per-project
mkdir -p .pi/skills
cp -r skills/cogito .pi/skills/
```

Pi auto-loads the skill and follows its workflow (`scan → query → compress → decide`)
when the trigger conditions are met. See `skills/cogito/SKILL.md` for the
full contract.

---

## Claude Code

Claude Code supports MCP servers natively. Add to your MCP config:

```json
{
  "mcpServers": {
    "cogito": {
      "command": "cogito-mcp"
    }
  }
}
```

Config location:

- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Linux: `~/.config/claude/config.json`

After restart, five tools appear in Claude Code: `scan_project`,
`compress_context`, `search_knowledge`, `add_decision`, `add_tag`.

---

## Cursor

Cursor reads MCP config from `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "cogito": {
      "command": "cogito-mcp"
    }
  }
}
```

Restart Cursor. Tools appear under the `@` menu in the composer.

---

## Continue (VSCode)

Continue.dev uses MCP via `~/.continue/config.json`:

```json
{
  "experimental": {
    "modelContextProtocolServers": [
      {
        "name": "cogito",
        "command": "cogito-mcp"
      }
    ]
  }
}
```

---

## Neovim

Cogito has no native plugin yet, but Neovim can call the CLI from any
completion or fuzzy-finder plugin. The simplest setup with
[`telescope.nvim`](https://github.com/nvim-telescope/telescope.nvim):

```lua
-- ~/.config/nvim/lua/cogito.lua
local pickers = require("telescope.pickers")
local finders = require("telescope.finders")
local conf = require("telescope.config").values
local actions = require("telescope.actions")
local action_state = require("telescope.actions.state")

local function cogito_scan()
  local output = vim.fn.systemlist("cogito scan --json")
  -- parse JSON, build picker items
  -- ...
end

vim.keymap.set("n", "<leader>cs", "<cmd>lua require'cogito'.scan()<CR>")
```

For an MCP client, see [`mcphub.nvim`](https://github.com/ravitemer/mcphub.nvim):

```lua
require("mcphub").setup({
  servers = {
    cogito = {
      command = "cogito-mcp",
    },
  },
})
```

---

## Aider

Aider can call the CLI as a pre-task hook. Add to `~/.aider.conf.yml`:

```yaml
auto-commits: false
pre-task-cmd: |
  cogito scan
```

Or invoke directly in chat:

```
/run cogito scan
/run cogito query "where is the auth code?"
```

---

## Zed

Zed reads MCP config from `~/.config/zed/settings.json`:

```json
{
  "context_servers": {
    "cogito": {
      "command": "cogito-mcp"
    }
  }
}
```

---

## Generic agent (REST API)

For agents that cannot spawn a process, run the REST API server:

```bash
cogito-api
# Listens on http://localhost:8080
```

Endpoints (all `POST`):

```bash
# Compress context
curl -X POST http://localhost:8080/api/v1/context/compress \
  -H "Content-Type: application/json" \
  -d '{"content": "fn main() { ... }", "max_tokens": 8000}'

# Add decision
curl -X POST http://localhost:8080/api/v1/decisions \
  -H "Content-Type: application/json" \
  -d '{"project_id": "uuid", "title": "...", "why": "..."}'

# Add tag
curl -X POST http://localhost:8080/api/v1/tags \
  -H "Content-Type: application/json" \
  -d '{"project_id": "uuid", "tag": "auth", "paths": ["src/auth/"]}'
```

See `SPEC.md` for the full API surface.

---

## Pre-commit hook

Use Cogito to keep the project knowledge base fresh on every commit:

```bash
# .git/hooks/pre-commit
#!/bin/sh
cogito scan
git add .cogito/
```

Or install it once globally:

```bash
# ~/.config/git/hooks/pre-commit
#!/bin/sh
cogito scan --quiet
git add .cogito/ 2>/dev/null
```

---

## GitHub Actions

Run Cogito in CI to keep the fingerprint up to date and comment on PRs with
zone changes:

```yaml
# .github/workflows/cogito.yml
name: Cogito
on:
  pull_request:
    paths: ['**/*.rs', '**/*.ts', '**/*.py']

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install cogito
        run: cargo install cogito-cli --locked
      - name: Scan project
        run: cogito scan --json > fingerprint.json
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: cogito-fingerprint
          path: fingerprint.json
```

---

## CI token (for crates.io)

Cogito's release workflow publishes to crates.io. To enable it:

1. Get a token at https://crates.io/settings/tokens
2. Add it as `CARGO_REGISTRY_TOKEN` in GitHub:
   Settings → Secrets and variables → Actions → New repository secret

The release runs only on tags matching `v*`.

---

## Editor matrix

| Editor / Agent | MCP | CLI | Skill | API |
|---|---|---|---|---|
| Gentle Shell (pi) | — | ✓ | ✓ first-class | ✓ |
| Claude Code | ✓ | ✓ | — | ✓ |
| Cursor | ✓ | ✓ | — | ✓ |
| Continue (VSCode) | ✓ | ✓ | — | ✓ |
| Neovim | via mcp | ✓ | — | ✓ |
| Aider | — | ✓ | — | ✓ |
| Zed | ✓ | ✓ | — | ✓ |
| GitHub Copilot | — | ✓ | — | ✓ |
| Generic | — | ✓ | — | ✓ |

If your tool is missing, the CLI + REST API always work. Add an entry to
this file via PR.
