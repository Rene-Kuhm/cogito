# Editor & Agent Integrations

Cogito is agent-agnostic. One binary, `cogito`, speaks MCP over stdio, HTTP,
and the command line.

```bash
curl -fsSL https://raw.githubusercontent.com/Rene-Kuhm/cogito-harness/main/install.sh | sh
cogito setup          # detects your agents and offers a menu
cogito doctor         # shows what is configured
```

`cogito setup` writes each agent's configuration for you. It backs up every
file it touches, merges instead of overwriting, and is safe to run twice.

---

## What setup does per agent

| Agent | Config file | Shape |
|---|---|---|
| **Gentle Shell / Pi** | `~/.gentle-shell/agent/settings.json`<br>`~/.gentle-shell/agent/mcp.json` | `packages: ["npm:cogito-harness"]`<br>`mcpServers.cogito` (direct exposure) |
| **Claude Code** | `~/.claude.json`<br>`~/.claude/settings.json`<br>`~/.claude/hooks/cogito-session-start.sh` | `mcpServers.cogito`<br>`hooks.SessionStart` entry<br>hook script (0755) |
| **Codex** | `~/.codex/config.toml` | `[mcp_servers.cogito]` |
| **Gemini CLI** | `~/.gemini/settings.json` | `mcpServers.cogito` |
| **opencode** | `~/.config/opencode/opencode.json`<br>`~/.config/opencode/plugins/cogito.js` | `mcp.cogito`<br>session-start plugin |
| **Crush** | `~/.config/crush/crush.json` | `mcp.cogito` |

An entry is marked `✓ configured` by `cogito doctor` only when **every** file
for that agent is present. A half-installed agent is reported as missing.

---

## Commands

```bash
cogito setup                      # interactive menu of detected agents
cogito setup claude-code codex    # specific agents
cogito setup --all                # every detected agent
cogito setup --dry-run            # show what would change, write nothing
cogito uninstall                  # remove from every configured agent
cogito uninstall claude-code -y   # remove from one, no prompt
```

Agent aliases: `pi` = `gentle-shell`, `claude` = `claude-code`,
`gemini-cli` = `gemini`.

---

## Agent-by-agent notes

### Gentle Shell / Pi

Two halves, both written by setup:

1. **`npm:cogito-harness`** in `settings.json` — a Pi package. On `session_start` it
   scans the project in the background and sets a status line, and it adds
   `/cogito` with `status`, `scan`, `query`, `stats` and `doctor`.

   ```
   /cogito status
   /cogito query where is authentication handled?
   ```

2. **`mcpServers.cogito`** in `mcp.json` — the MCP tools, with
   `"exposure": "direct"`. Pi's default exposure is `codemode`, and Gentle
   Shell disables codemode, so under the default the model could not reach
   the tools.

> **`npm:cogito-harness` is not on npm yet.** Pi runs `npm install` for every
> entry in `packages`, and a 404 there aborts package resolution — so Pi does
> *not* skip an unresolvable package, it fails. `cogito setup` therefore checks
> the registry first and, when the package is absent, configures only the MCP
> half and says so. Run `cogito setup` again after the package is published and
> it will declare it.
>
> If you know it is available and the check cannot see it (an offline machine,
> a private registry), pass `--with-pi-package` to declare it anyway.

### Claude Code

Three files:

- `~/.claude.json` gets `mcpServers.cogito`.
- `~/.claude/hooks/cogito-session-start.sh` warms the fingerprint. It exits 0
  when `cogito` is missing and backgrounds the scan with `nohup`, so it can
  never fail or slow a session.
- `~/.claude/settings.json` gets a `SessionStart` entry running that script
  with `async: true`.

Claude Code keeps MCP servers and settings in separate files, which is why it
is the only agent that touches two config files.

### Codex

`~/.codex/config.toml` gains:

```toml
[mcp_servers.cogito]
command = "/path/to/cogito"
args = ["mcp"]
enabled = true
```

The file is edited with `toml_edit`, so comments and formatting elsewhere are
preserved.

### Gemini CLI

`~/.gemini/settings.json` gains `mcpServers.cogito`.

### opencode

- `~/.config/opencode/opencode.json` gains `mcp.cogito`.
- `~/.config/opencode/plugins/cogito.js` fires `cogito scan --quiet` on
  `session.created`. opencode loads everything in that directory, so no config
  change is needed for the plugin itself.

### Crush

`~/.config/crush/crush.json` gains `mcp.cogito` with `type: "stdio"`.

---

## Integrating an agent we do not support yet

Any MCP-capable client works if you add the server yourself:

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

Cogito exposes six tools, each taking an explicit `path` to the project root:

| Tool | Purpose |
|---|---|
| `scan_project` | Generate the code fingerprint |
| `compress_context` | Reduce context; AST-aware with a `language` hint |
| `search_knowledge` | Search tags and decisions (FTS5, porter stemming) |
| `add_decision` | Record an architectural decision |
| `add_tag` | Tag code paths |
| `project_stats` | Stored tag/decision counts |

State lives in `<project>/.cogito/state.db`. One server process can serve many
projects; each tool call opens the database for the `path` it was given.

---

## Any agent without MCP

Use the CLI or the REST API.

```bash
cogito scan
cogito query "where is billing?"
cogito compress < big.rs --language rust
cogito-api            # REST on :8080
```

---

## Editor-only integrations

### Neovim

Telescope over `cogito scan --json`, or an MCP client such as
[`mcphub.nvim`](https://github.com/ravitemer/mcphub.nvim).

### Aider

```
/run cogito scan
/run cogito query "where is the auth code?"
```

### CI

```yaml
- run: cargo install cogito-harness --locked || true
- run: cogito scan --json > fingerprint.json
```

---

## Editor matrix

| Editor / Agent | MCP | Adapter | Skill | CLI | API |
|---|---|---|---|---|---|
| Gentle Shell (pi) | ✓ direct | ✓ cogito-harness | ✓ | ✓ | ✓ |
| Claude Code | ✓ | ✓ SessionStart hook | — | ✓ | ✓ |
| opencode | ✓ | ✓ session plugin | — | ✓ | ✓ |
| Codex | ✓ | — | — | ✓ | ✓ |
| Gemini CLI | ✓ | — | — | ✓ | ✓ |
| Crush | ✓ | — | — | ✓ | ✓ |
| Cursor | ✓ | — | — | ✓ | ✓ |
| Zed | ✓ | — | — | ✓ | ✓ |
| Aider / Neovim | via mcp | — | — | ✓ | ✓ |

Missing an agent? Add a row to `Agent` in
`crates/cogito-harness/src/agents.rs` — it needs a config path and a shape.

---

## Uninstalling

```bash
cogito uninstall          # every agent where Cogito is configured
cogito uninstall --yes    # non-interactive
```

Backups of every file Cogito ever changed are left next to the original as
`<name>.cogito-backup-<timestamp>`.
