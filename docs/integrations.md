# Editor & Agent Integrations

Cogito is agent-agnostic. One binary, `cogito`, speaks MCP over stdio and
the command line.

```bash
curl -fsSL https://raw.githubusercontent.com/Rene-Kuhm/cogito-harness/main/install.sh | sh
cogito setup          # detects your agents and offers a menu
cogito doctor         # shows what is configured
```

`cogito setup` writes each agent's configuration for you. It backs up every
file it touches, merges instead of overwriting, and is safe to run twice.

---

## What setup does per agent

| Agent | Config file(s) | Shape |
|---|---|---|
| **Gentle Shell / Pi** | `~/.gentle-shell/agent/settings.json`<br>`~/.gentle-shell/agent/mcp.json` | `packages: ["npm:cogito-harness"]`<br>`mcpServers.cogito` with `"exposure": "direct"` |
| **Claude Code** | `~/.claude.json`<br>`~/.claude/settings.json`<br>`~/.claude/hooks/cogito-session-start.sh`<br>`~/.claude/hooks/cogito-pre-tool-use.sh` | `mcpServers.cogito`<br>`hooks.SessionStart` + `hooks.PreToolUse` (sync)<br>0755 session-start script<br>0755 guardrail script |
| **Codex** | `~/.codex/config.toml` | `[mcp_servers.cogito]` |
| **Gemini CLI** | `~/.gemini/settings.json` | `mcpServers.cogito` |
| **opencode** | `~/.config/opencode/opencode.json`<br>`~/.config/opencode/plugins/cogito.js` | `mcp.cogito`<br>session plugin |
| **Crush** | `~/.config/crush/crush.json` | `mcp.cogito` |

An entry is marked `✓ configured` by `cogito doctor` only when **every**
file for that agent is present. A half-installed agent is reported as missing.

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

1. **`npm:cogito-harness`** in `settings.json` — a Pi package. On
   `session_start` it scans the project in the background and sets a status
   line, and it adds `/cogito` with `status`, `map`, `scan`, `stats` and `doctor`.

   ```
   /cogito status
   /cogito map
   ```

2. **`mcpServers.cogito`** in `mcp.json` — the MCP tools, with
   `"exposure": "direct"`. Pi's default exposure is `codemode`, and Gentle
   Shell disables codemode, so under the default the model could not reach
   the tools without `direct`.

> **`npm:cogito-harness` is not on npm yet.** Pi runs `npm install` for every
> entry in `packages`, and a 404 there aborts package resolution — so Pi does
> *not* skip an unresolvable package, it fails. `cogito setup` therefore checks
> the registry first and, when the package is absent, configures only the MCP
> half and says so. Run `cogito setup` again after the package is published and
> it will declare it.
>
> Only an `E404` from npm counts as "absent", and then setup also removes a
> declaration an earlier run left behind (`cogito doctor` flags it with `!`).
> When npm cannot be reached at all (offline, DNS, proxy), setup leaves the
> declaration exactly as it is.
>
> If you know it is available and the check cannot see it (an offline machine,
> a private registry), pass `--with-pi-package` to declare it anyway.

### Claude Code

Four files:

- `~/.claude.json` gets `mcpServers.cogito`.
- `~/.claude/hooks/cogito-session-start.sh` prints the project card (modules,
  key docs, busiest files, how to look code up) wrapped in
  `<cogito_project>`. Claude Code adds a `SessionStart` hook's output to the
  session context, so each session starts oriented without a `project_map`
  call. The card comes from the stored index in milliseconds; it rescans only
  when `HEAD` moved. The script prints nothing and exits 0 outside a project
  or when `cogito` is missing.
- `~/.claude/hooks/cogito-pre-tool-use.sh` is a guardrail for `Read` and
  `Grep`. The first time Claude reads a file of 250+ lines whole, the call is
  blocked and Claude gets the file's outline instead; the first time it greps
  for a plain name, it gets Cogito's search result. Repeating the exact same
  call runs normally, ranged reads and regex greps are never touched, and any
  error lets the call through. The decision logic is in `cogito hook
  claude-pre-tool-use`; the script only hands it Claude Code's JSON.
- `~/.claude/settings.json` gets a `SessionStart` entry for the card and a
  `PreToolUse` entry (matcher `^(Read|Grep)$`) for the guardrail, both
  synchronous (an `async` hook's output never reaches the session; `cogito
  setup` repairs entries written that way by older versions).

Cogito writes a `.gitignore` of `*` inside each `.cogito/` it creates, so its
index and per-session state never show up in `git status`.

`setup` leaves a file untouched when its content would not change, so
repairing the hook does not rewrite `~/.claude.json` while Claude Code is
running.

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

`setup` also installs a Cogito block into `AGENTS.md` at the project root.
Codex reads every `AGENTS.md` from the project root toward the working
directory and concatenates them (closer files take precedence), with a 32 KiB
budget. The block is short and carries the project card reminder, the MCP tool
names, and the guardrail rule.

You can add a SessionStart hook to inject the project card into Codex sessions:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "^(startup|resume|clear|compact|fork)$",
      "hooks": [{
        "type": "command",
        "command": "sh '/path/to/cogito-session-start.sh'",
        "timeout": 30
      }]
    }]
  }
}
```

And the hook script (`~/.codex/hooks/cogito-session-start.sh`, 0755):

```sh
#!/bin/sh
command -v cogito >/dev/null 2>&1 || exit 0
cd "${CODEX_DIR:-.}" 2>/dev/null || exit 0
[ -e .git ] || [ -d .cogito ] || exit 0
card=$(cogito map --card 2>/dev/null)
[ -n "$card" ] || exit 0
printf '<cogito_project>\n%s\n</cogito_project>\n' "$card"
```

> **Codex hooks are not yet verified.** Codex has a hooks system, but its
> contract was not verified against a live Codex CLI session. The `AGENTS.md`
> block is the only integration implemented.

### Gemini CLI

`~/.gemini/settings.json` gains `mcpServers.cogito`.

### opencode

- `~/.config/opencode/opencode.json` gains `mcp.cogito`.
- `~/.config/opencode/plugins/cogito.js` is installed by `setup`. It:
  - Fires `cogito scan --quiet` on `session.created` and `session.start`.
  - Injects the project card via `experimental.chat.system.transform` (once per
    session).
  - Intercepts the first whole-file `Read` and plain-name `Grep` calls via
    `tool.execute.before` and returns Cogito's outline or search result instead.
    The same call on retry passes through.

`setup` also installs the Cogito block into `AGENTS.md` at the project root.
opencode reads every `AGENTS.md` from the workspace root toward home.

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
| `context_pack` | The context for a task in one call: ranked fragments with the reason each was chosen, inside a token budget. The first thing to call when you know what you are about to do |
| `project_map` | Modules with entry point, size, most changed files and which modules they use. Read it before exploring the repository |
| `related_files` | What one file imports, what imports it, and its external packages |
| `search_code` | Find code by words: symbols (functions, types, classes) with `file:line`, file paths, module names and doc sections |
| `file_outline` | A file's declarations (code) or sections (Markdown) with line ranges, so only the needed range is read |
| `impact` | What a diff reaches: the declarations its hunks touch, the files that import the changed files and the files that declare those symbol names, with a risk level from the importer count. An overapproximation by design |

The map is stored after each scan and reused until `HEAD` moves, so these calls
read no source files. Documentation is indexed too: `search_code` finds
Markdown sections by title, and `file_outline` on a `.md` file lists its
sections with line ranges.

`context_pack` is also the tool an agent should reach for first when it already
knows what it is about to do, because it replaces the sequence of `search_code`,
`file_outline` and ranged reads that would otherwise be needed to get to the same
place.

State lives in `<project>/.cogito/state.db`. One server process can serve many
projects; each tool call opens the database for the `path` it was given.

---

## Any agent without MCP

Use the CLI directly:

```bash
cogito scan
cogito search "useAuth"
cogito outline src/auth.rs
cogito map --card
```

---

## Editor-only integrations

### Neovim

Telescope over `cogito map --json`, or an MCP client such as
[`mcphub.nvim`](https://github.com/ravitemer/mcphub.nvim).

### Aider

```
/run cogito scan
/run cogito search "where is the auth code?"
```

### CI

```yaml
- run: cargo install cogito-harness --locked || true
- run: cogito scan --json > fingerprint.json
```

---

## Editor matrix

| Editor / Agent | MCP | Session-start card | Read/grep guardrail | `AGENTS.md` block |
|---|---|---|---|---|
| Gentle Shell (pi) | ✅ direct | ✅ | — | — |
| Claude Code | ✅ | ✅ | ✅ | — |
| opencode | ✅ | ✅ | ✅ | ✅ |
| Codex | ✅ | — | — | ✅ |
| Gemini CLI | ✅ | — | — | — |
| Crush | ✅ | — | — | — |
| Cursor | ✅ | — | — | — |
| Zed | ✅ | — | — | — |
| Aider / Neovim | via MCP | — | — | — |

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
