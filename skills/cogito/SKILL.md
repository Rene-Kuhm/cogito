---
name: cogito
description: "Use Cogito to optimize context before reading or grepping code: generate code fingerprints (entry points, hot/cold zones, data flows), compress context, and search semantic tags and architectural decisions. Trigger: large codebase, context budget pressure, repeated searches, \"where is X\", \"how does Y work\", planning a change across modules, or before any read-heavy operation that may inflate tokens."
---

# Cogito — Context Optimization for AI Agents

Use this skill **before** any read-heavy operation in a non-trivial codebase.
Cogito is a local-first context harness: it answers structural questions about
a project without forcing the agent to read source files.

## When to Use

Trigger this skill when **any** of these hold:

- The user asks a structural question about a codebase: "where is auth?",
  "how does billing work?", "what modules touch X?".
- Context budget is at risk: many files to read, large files, repeated searches.
- The task spans multiple modules and you need a map before editing.
- You would otherwise `cat` or `Read` several files just to orient yourself.
- You need to know *why* a past decision was made (architectural decisions log).
- You need to compress existing context (large diff, large log, large file).

Do **not** use this skill for:

- Single-file edits where the file is already in context.
- Pure shell tasks unrelated to code.
- Read-once operations on tiny files (<200 lines).

## Core Capability: the Code Fingerprint

Cogito pre-computes a minimal structural snapshot of a project so the agent
can answer most orientation questions without reading code:

| What you want to know | What Cogito returns |
|---|---|
| Where do I start? | `entry_points` (main, lib, handlers) |
| What touches module X? | `data_flows` (imports/requires graph) |
| What changes a lot? | `hot_zones` (most-edited files via git) |
| What is stable / safe? | `cold_zones` and `frozen_zones` |
| Why was Y chosen? | `decisions` log (architectural) |
| Where is the auth code? | `tags` (semantic) |

## Standard Workflow

The standard path is `scan → query → compress → decide`. The first three
steps are read-only and cheap; only the last is a write.

1. **Scan** the project once. This populates the fingerprint and persists it
   to `.cogito/` in the repo. After this, every later invocation is fast.

   ```bash
   cogito scan
   ```

2. **Query** the knowledge base for the question at hand. This searches
   semantic tags and the decision log.

   ```bash
   cogito query "where is authentication?"
   ```

3. **Read only what you must.** If the fingerprint and queries answered the
   question, you never read a file. If not, read the smallest set guided by
   `hot_zones` and `entry_points`.

4. **Compress** any large context you must pass downstream (a diff, a log,
   a file, a multi-file paste) before forwarding it to another model call.

   ```bash
   cogito compress < huge_context.txt > compressed.txt
   ```

5. **Record decisions** when you make or observe a non-obvious architectural
   choice. This survives across sessions.

   ```bash
   cogito decision add --title "JWT for auth" --why "stateless, scales" \
       --alternative "cookie sessions" --tags auth --paths src/auth/
   ```

## The Two Cardinal Sins

1. **Reading before scanning.** Opening files without a fingerprint first
   almost always means you read the wrong files. Scan first.
2. **Sending raw context to a model.** If you would paste > 2k tokens of
   context into another prompt, run `cogito compress` first.

## MCP Integration

When running inside an MCP-compatible host (Claude Code, Cursor, etc.) and
the `cogito mcp` server is configured, prefer the MCP tools over the CLI:

| Tool | Purpose |
|---|---|
| `project_map` | Modules with entry point, size, most changed files and which modules they use. Read it before exploring the repository |
| `related_files` | What one file imports, what imports it, and its external packages |
| `search_code` | Find code by words: symbols (functions, types, classes) with `file:line`, file paths, module names and tags |
| `file_outline` | A file's declarations with line ranges, so only the needed range is read |
| `add_tag` | Label code locations so `search_code` finds them by that word |

The MCP server is `cogito mcp` (same binary, `mcp` subcommand). It uses the
same `.cogito/state.db` as the CLI, so both stay in sync. `cogito setup` writes
the MCP configuration for every agent it supports.

## Integration with Pi (gentle-shell)

When running inside Pi:

- The skill auto-loads; you do not need to import it.
- Prefer `cogito scan` over multiple `Read` calls when context is large.
- Use `mem_search` for cross-session knowledge; use `cogito query` for
  project-local knowledge.
- When Pi suggests this skill via the trigger conditions above, follow its
  workflow instead of improvising reads.

## Failure Modes

- **No git repo in the project root**: zones detection degrades to filesystem-only
  (no hot/cold/frozen). Other features still work.
- **No `.cogito/` directory**: first `cogito scan` creates it. Until then,
  `cogito query` returns empty.
- **Massive monorepos (>10k files)**: scan may take a few seconds. Cache the
  fingerprint per session, do not re-run.

## Related Skills

- `read-efficiency`: optimize single-read operations
- `gentle-ai`: harness discipline, ODD workflow
