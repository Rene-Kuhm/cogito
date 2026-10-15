# Cogito

**Intelligent context harness for AI coding agents.**

Cogito gives AI coding agents the project context they need — structure, symbols, references — without sending the entire codebase. It indexes files and declarations with tree-sitter, keeps an incremental SQLite/FTS5 store in sync with the working tree, and exposes six MCP tools (`context_pack`, `project_map`, `search_code`, `file_outline`, `related_files`, `impact`) that agents can call on demand. Token savings on benchmark: 89.8% on ripgrep and bat — from 743,440 tokens to 75,961 over eight search terms.

**Storage**: SQLite with FTS5 full-text index over files and symbols. Incremental sync: a new uncommitted file is visible without a commit or a manual refresh.

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/Rene-Kuhm/cogito-harness/main/install.sh | sh

# Wire it into the agents you use (backups every file it touches)
cogito setup

# Check what is configured
cogito doctor

# Use it
cogito init                     # create .cogito/ in the current project
cogito scan                     # build fingerprint and full index
cogito search "useAuth"         # look up a symbol, file or doc section
cogito outline src/main.rs      # see a file's declarations with line ranges
cogito map                       # print the full project map
cogito map --card               # print the short project card
cogito related src/auth.rs      # show what a file imports and what imports it
cogito bench                    # measure token savings vs baseline explorer
```

[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/cogito-harness-core.svg)](https://crates.io/crates/cogito-harness-core)
[![docs](https://img.shields.io/badge/docs-docs.rs-orange.svg)](https://docs.rs/cogito-harness-core)

---

## Works with everything

`cogito setup` detects the agents installed on your machine and offers a menu.
It writes each agent's config, backing up every file first:

| Agent | Integration |
|---|---|
| **Gentle Shell (pi)** | `cogito-harness` npm package (session scan, status line, `/cogito` commands) + direct MCP exposure |
| **Claude Code** | MCP + `SessionStart` hook (project card on session start) + `PreToolUse` guardrail |
| **Codex** | MCP + `AGENTS.md` block (project card reminder + MCP tool names + read/grep guardrail) |
| **opencode** | MCP + session plugin (card injection, guardrails) + `AGENTS.md` block |
| **Gemini CLI** | MCP |
| **Crush** | MCP |
| **Cursor / Zed / any MCP client** | one config block |
| **Any agent without MCP** | CLI |

```bash
cogito setup              # interactive menu of detected agents
cogito setup --all        # configure everything detected
cogito setup --dry-run    # show what would change, write nothing
cogito uninstall          # remove, keeping backups
```

Full per-agent detail: [`docs/integrations.md`](docs/integrations.md).

---

## What is Cogito?

AI coding agents suffer from **context inflation**. Large codebases exceed context windows, agents fetch irrelevant code, and every conversation starts from scratch. Cogito solves this by giving the agent the right structural information at the right time.

| Feature | What it does | Why it matters |
|---------|-------------|----------------|
| **Project card** | ~320-token summary: modules, key docs, busiest files | Agent orients without a call |
| **Code fingerprint** | Hot/cold/frozen zones from git history, modules from manifests, entry points and data flows | Agent knows where the action is |
| **Symbol & file index** | Tree-sitter declarations for 10 languages, keyed by language | Agents find symbols, not just files |
| **Outline & reference lookup** | Declarations with exact line ranges; imports resolved for TS/JS, Rust, Python, Go, Java | Only the needed lines are read |
| **Incremental sync** | SQLite index synced from the working tree on every call | Uncommitted files are visible without a commit |
| **Token benchmark** | Reproducible `cogito bench` over pinned open-source repos | Measure actual savings |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         COGITO CORE                         │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │   Code       │  │   Symbol    │  │  Incremental       │ │
│  │  Fingerprint │  │   Index     │  │  SQLite / FTS5     │ │
│  └──────────────┘  └──────────────┘  └────────────────────┘ │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │   Import     │  │   Tree-      │  │   MCP Protocol    │ │
│  │  Resolution │  │   sitter     │  │   Adapter         │ │
│  └──────────────┘  └──────────────┘  └────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
          │                     │
    ┌─────┴─────┐         ┌─────┴─────┐
    │    CLI    │         │  MCP      │
    └───────────┘         │  Server   │
                          └───────────┘
                           │
              ┌────────────┴──────────────┐
              │         ANY AI AGENT       │
              │  Claude Code · opencode    │
              │  Codex · Gemini · Crush   │
              │  pi · any MCP client       │
              └────────────────────────────┘
```

---

## Installation

### From source

```bash
git clone https://github.com/Rene-Kuhm/cogito-harness.git
cd cogito
cargo install --path crates/cogito-harness
```

### From crates.io (when published)

```bash
cargo install cogito-harness
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/Rene-Kuhm/cogito-harness/releases).

---

## Quick Start

```bash
# 1. Initialise Cogito in your project
cogito init

# 2. Build fingerprint and full index
cogito scan

# 3. Print the project card (what an agent gets on session start)
cogito map --card

# 4. Search for a symbol, file or doc section
cogito search "useAuth"

# 5. See a file's declarations with exact line ranges
cogito outline src/main.rs

# 6. See what a file imports and what imports it
cogito related src/auth.rs

# 7. Measure token savings
cogito bench
```

---

## Features

### Code Fingerprint Generator

```bash
cogito scan
cogito fingerprint show
```

Generates a minimal structural snapshot identifying:
- Entry points (main, routes, handlers)
- Data flows between modules
- Hot/cold/frozen zones from git history analysis
- Module structure from manifests (`Cargo.toml`, `package.json`, `go.mod`, …)

### Symbol & File Index

```bash
cogito search "useAuth"       # symbols, files, doc sections
cogito outline src/main.rs    # declarations with line ranges
```

Tree-sitter declaration extraction for Rust, TypeScript, JavaScript, Python,
Go, Java, C#, Ruby, PHP, Kotlin and Swift. Markdown sections are indexed
separately. FTS5 with bm25 ranking over symbol names and doc section titles.
The index syncs incrementally from the working tree on every call.

### Import Resolution

```bash
cogito related src/auth.rs
```

Resolves imports for TypeScript/JavaScript, Rust, Python, Go and Java.
`related_files` shows: what the file imports, what imports it, and external
packages.

### Incremental Sync

The SQLite index (`indexed_files`, `indexed_symbols` with FTS5, `indexed_imports`)
is kept in sync with the working tree by content hash + size + mtime.
An uncommitted file is visible in `search_code` without a commit or `--refresh`.

### Token Benchmark

```bash
cogito bench
```

Reproducible benchmark against pinned open-source repositories (ripgrep, bat),
counting tokens with `tiktoken-rs` / `cl100k_base`. Baseline vs Cogito:
listing files, grepping, whole-file reads vs card + `search_code` + `file_outline`.
On ripgrep and bat combined: 743,440 → 75,961 tokens (89.8% savings).

### MCP Server

```bash
cogito mcp
```

Six tools: `context_pack`, `project_map`, `search_code`, `file_outline`,
`related_files`, `impact`. Each tool takes an explicit `path` to the project root.

`impact` answers before a commit. It reads the diff — the uncommitted changes by
default, or `base..HEAD` when given a revision — maps its hunks onto the
outline's line ranges, and names the declarations they touch, the files that
import the changed files and the files that use those names. A use counts only
when the index already has an import edge to the declaring file, and only for
TypeScript and Rust: every other language is named under an explicit "not
supported" instead of being guessed at. Its risk level comes only from the
importer count, and it says so: the answer is an overapproximation, because the
import graph and the identifier names come from the index, which holds no calls
or type resolution.

`context_pack` is the one to start with when you know what you are about to do:
give it the task in your own words and it returns the fragments that matter,
ranked, with the reason each was chosen and inside a token budget (2000 by
default, 8000 maximum). Measured recall over ten labelled tasks on ripgrep and
bat: 66.7% overall, and the split matters more than the average — 83.3% on the
six English tasks against 41.7% on the four Spanish ones. The tables and the
limits are in [`docs/benchmarks.md`](docs/benchmarks.md), including what was
tried and reverted. The main limit: a question in Spanish about code written in
English gets much less out of it than the same question in English.

---

## Comparison

| Feature | Cogito | LangChain | LlamaIndex |
|---------|--------|-----------|------------|
| Tree-sitter symbols | ✅ | ❌ | ❌ |
| Hot/cold/frozen zones | ✅ | ❌ | ❌ |
| Incremental index | ✅ | ❌ | ❌ |
| Import resolution | ✅ (5 stacks) | ❌ | ❌ |
| MCP native | ✅ | ⚠️ | ❌ |
| Reproducible benchmark | ✅ | ❌ | ❌ |
| Token savings | 89.8% (bench) | Unknown | Unknown |
| Open source | ✅ | ✅ | ✅ |

---

## Supported Models / Agents

Cogito is agent-agnostic. It works with any agent that speaks MCP:
Gentle Shell (pi), Claude Code, opencode, Codex, Gemini CLI, Crush, Cursor, Zed.

Agents without MCP use the CLI directly.

---

## Supported Languages

| Language | Fingerprint | Symbol extraction | Import resolution |
|----------|-------------|-------------------|-------------------|
| Rust | ✅ | ✅ | ✅ |
| TypeScript / JavaScript | ✅ | ✅ | ✅ |
| Python | ✅ | ✅ | ✅ |
| Go | ✅ | ✅ | ✅ |
| Java | ✅ | ✅ | ✅ |
| C# | ✅ | ✅ | — |
| Ruby | ✅ | ✅ | — |
| PHP | ✅ | ✅ | — |
| Kotlin | ✅ | ✅ | — |
| Swift | ✅ | ✅ | — |

---

## Documentation

- [SPEC.md](SPEC.md) — Technical specification
- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute
- [docs/integrations.md](docs/integrations.md) — Per-agent setup and behaviour
- [docs/benchmarks.md](docs/benchmarks.md) — Token savings benchmark details

---

## License

Apache License 2.0 — see [LICENSE](LICENSE)

---

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for development
setup, code style, commit conventions and the pull request process.
