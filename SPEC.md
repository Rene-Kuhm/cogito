# Cogito — Project Specification

> *"Cogito, ergo sum"* — The intelligent context harness for AI coding agents.

## 1. Concept & Vision

**Cogito** is an open-source context optimization harness that sits between AI coding agents and their codebases. It reduces token consumption, improves context relevance, and accelerates agent performance — without modifying the underlying models or agents.

The core philosophy: *Give AI agents exactly what they need, when they need it, no more.*

### The Problem

AI coding agents suffer from context inflation:

- **Context overflow**: Large codebases exceed context windows
- **Irrelevant retrieval**: Agents fetch unrelated code or read whole files
- **No working-tree visibility**: Uncommitted files are invisible without a commit
- **Verbose output**: Agents read whole files when only a declaration is needed

Existing solutions (LangChain, LlamaIndex) are integration frameworks — they connect things but don't optimize context intelligently.

### The Solution

Cogito introduces four core abstractions:

1. **Project card & fingerprint** — A ~320-token project summary (card) and a minimal structural snapshot identifying hot/cold/frozen zones, modules, entry points and data flows.
2. **Symbol & file index** — Tree-sitter declarations indexed in SQLite with FTS5, keyed by language. Every `search_code`, `file_outline` and `related_files` call reads rows, not source files.
3. **Outline & reference lookup** — Exact line ranges for declarations and Markdown sections; import resolution for TypeScript/JavaScript, Rust, Python, Go and Java.
4. **Per-agent integration surface** — MCP tools, session-start card injection, and guardrails that block the first whole-file read and the first plain-name grep.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           COGITO CORE                               │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌────────────────────────────┐│
│  │   Code      │  │   Symbol    │  │   Incremental SQLite       ││
│  │ Fingerprint │  │   Index     │  │   / FTS5 store             ││
│  └─────────────┘  └─────────────┘  └────────────────────────────┘│
│  ┌─────────────┐  ┌─────────────┐  ┌────────────────────────────┐│
│  │   Import    │  │   Tree-     │  │   MCP Protocol            ││
│  │  Resolution │  │   sitter    │  │   Adapter                 ││
│  └─────────────┘  └─────────────┘  └────────────────────────────┘│
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                               │
                    ┌──────────┴──────────┐
                    │       CLI           │
                    │       MCP           │
                    │  (stdio subprocess) │
                    └──────────┬──────────┘
                               │
              ┌────────────────┴────────────────┐
              │   ANY AI AGENT (MCP client)    │
              │                                │
              │  Gentle Shell · Claude Code    │
              │  opencode  ·  Codex            │
              │  Gemini CLI  ·  Crush          │
              │  Cursor  ·  Zed                │
              └────────────────────────────────┘
```

---

## 3. Core Modules

### 3.1 `fingerprint` — Code Fingerprint Generator

**Location**: `crates/cogito-harness-core/src/fingerprint/`

Creates a minimal structural snapshot identifying hot/cold/frozen zones,
modules from manifests, entry points and data flows.

```rust
pub struct Fingerprint {
    pub project_id: Uuid,
    pub modules: Vec<ModuleInfo>,       // from Cargo.toml, package.json, etc.
    pub entry_points: Vec<PathBuf>,
    pub data_flows: Vec<DataFlow>,     // imports → resolved files
    pub zones: Vec<Zone>,              // hot / warm / cold / frozen
    pub language: Language,
    pub generated_at: DateTime<Utc>,
}
```

Algorithm:
1. Enumerate source files (respecting `.gitignore`, `/.cogito/`, hidden dirs)
2. Detect project language from manifest files
3. Parse imports with per-language parsers
4. Resolve imports to project-relative paths
5. Analyse git history to assign zone frequencies
6. Collect module structure from manifest (Rust crates, JS/TS packages, Go modules)

### 3.2 `index` — Symbol & File Index

**Location**: `crates/cogito-harness-core/src/index/`

Incremental SQLite/FTS5 index over source files, declarations and imports.
Syncs from the working tree on every call (content hash + size + mtime), so
uncommitted files are visible without a commit or `--refresh`.

```rust
// Three indexed tables:
//   indexed_files    — path, language, size, mtime, hash
//   indexed_symbols  — path, name, kind, line_start, line_end, doc
//                     (indexed by FTS5 with bm25 ranking)
//   indexed_imports  — importer, imported, resolved_path
```

`index::search` queries FTS5 with bm25 ranking and returns up to 12 hits.
`index::outline` reads declaration rows for a given file.
`index::imports` and `index::importers` return resolved project-relative paths.

### 3.3 `map` — Map, Outline, Symbol Extraction & Layout

**Location**: `crates/cogito-harness-core/src/map/`

Four submodules:

- **`symbols.rs`** — Tree-sitter declaration extraction for Rust,
  TypeScript, JavaScript, Python, Go, Java, C#, Ruby, PHP, Kotlin and Swift.
  Falls back to a regex path for unknown extensions. Markdown sections are
  handled separately (heading lines, no tree-sitter required).

- **`outline.rs`** — Consumes `symbols::declarations` for code files and
  parses Markdown headings for `.md` files. Returns `Vec<Symbol>` with exact
  inclusive line ranges.

- **`layout.rs`** — Resolves imports for TypeScript/JavaScript, Rust, Python,
  Go and Java. Input is one file's import statements; output is the resolved
  project-relative path for each import and a flag `is_external`.

- **`mod.rs`** — Renders output: `build` + `render` for the full project map,
  `render_outline` for a file's declarations, `render_related` for a file's
  neighbourhood, `render_search` for FTS5 hits, `render_card` for the ~320-token
  project card. Also contains `find_file`, which resolves a file argument
  (exact path, glob pattern, or fuzzy name) against the fingerprint.

### 3.4 `storage` — SQLite Persistence

**Location**: `crates/cogito-harness-core/src/storage/`

Two files:

- **`sqlite.rs`** — Schema v3 (`projects`, `fingerprints`, `indexed_files`,
  `indexed_symbols` with `indexed_symbols_fts`, `indexed_files_fts`,
  `indexed_imports`), WAL journal mode, FTS5 with the `unicode61` tokeniser,
  `content_hash` / `size` / `mtime_ns` sync keys, upserts for incremental
  updates, and forward migrations from schema v1 and v2.

- **`mod.rs`** — `db_path` (resolves `/.cogito/state.db`), `open` (creates
  dirs + WAL + migrates), `migrate_pub` (runs all migrations on an open conn).

### 3.5 `types` — Shared Data Types

**Location**: `crates/cogito-harness-core/src/types.rs`

```rust
pub enum Language {
    Rust, TypeScript, JavaScript, Python, Go, Java,
    CSharp, Ruby, Php, Kotlin, Swift, Unknown,
}

pub struct Project { id, path, name, language, created_at, updated_at }
pub struct Fingerprint { project_id, modules, entry_points, data_flows, zones, language, generated_at }
pub struct Symbol { name, kind, path, line_start, line_end, doc }
pub struct ModuleInfo { name, path, is_workspace }
pub struct DataFlow { from, flow_type, to, resolved }
pub enum FlowType { Import, Re-export, Submodule }
pub struct Zone { path, frequency: ZoneFrequency, last_modified, change_count }
pub enum ZoneFrequency { Hot, Warm, Cold, Frozen }
```

---

## 4. CLI Tool (`cogito-harness`)

**Location**: `crates/cogito-harness/src/`

Entrypoint: `main.rs` + `commands/mod.rs` (subcommands). `agents.rs` defines
the agent registry and config shapes. `claude_hook.rs` implements the
`PreToolUse` guardrail decision logic. `mcp.rs` exposes five tools over stdio.

### Command reference

```bash
cogito init                         # create .cogito/ in the current directory
cogito scan                         # build fingerprint + full index
cogito fingerprint show            # print stored fingerprint
cogito project <sub>               # list / delete managed projects
cogito mcp                         # run MCP server over stdio
cogito setup [agents...]           # install into detected agents
cogito uninstall [agents...]       # remove from agents
cogito stats                       # show index statistics
cogito map [--refresh] [--card]    # print project map or card
cogito outline <file>              # print file declarations with line ranges
cogito search <query>              # search symbols, files, doc sections
cogito related <file>              # show imports and importers for a file
cogito doctor                      # diagnose which agents are configured
cogito bench [--repo .] [--json]   # token benchmark
cogito hook <name>                 # run agent hook (hidden, called by agents)
```

Global flags: `--verbose`, `--quiet`, `--json`.

---

## 5. MCP Protocol Adapter

**Location**: `crates/cogito-harness/src/mcp.rs`

Transport: stdio subprocess. Five tools, each taking an explicit `path`
(project root). All five open the SQLite index, run `sync_index` to bring it
up to date with the working tree, then read rows.

| Tool | Input | Output |
|---|---|---|
| `context_pack` | `path`, `task`, `budget?` | Ranked fragments inside the budget, each with its reason: a name match, a path match, or one import hop (`imported by X`) |
| `project_map` | `path`, `refresh?` | Full project map (modules, entry points, zones, busy files) |
| `search_code` | `path`, `query` | Up to 12 FTS5 hits with bm25 score, file:line, kind |
| `file_outline` | `path`, `file` | Declarations (code) or sections (Markdown) with line ranges |
| `related_files` | `path`, `file` | Imports, importers, external packages for the given file |

No resource URI scheme, no `add_tag`, no `compress_context`, no REST endpoints.

---

## 6. Per-Agent Integration Surface

See [`docs/integrations.md`](docs/integrations.md) for the full per-agent
reference. Summary:

| Agent | MCP | Session-start card | Read/grep guardrail | `AGENTS.md` block |
|---|---|---|---|---|
| Gentle Shell (pi) | ✅ direct | ✅ (`cogito-harness` npm) | — | — |
| Claude Code | ✅ | ✅ (hook script) | ✅ (`PreToolUse` hook) | — |
| opencode | ✅ | ✅ (plugin) | ✅ (plugin) | ✅ |
| Codex | ✅ | — | — | ✅ |
| Gemini CLI | ✅ | — | — | — |
| Crush | ✅ | — | — | — |

`setup` writes agent configs and backs up every file it touches. It is
idempotent: re-running updates, never duplicates.

---

## 7. Design Decisions

### Tree-sitter over regex for symbols

Regex fails to extract scopes, ignores imports inside functions, and cannot
handle multi-line declarations. Tree-sitter handles all of these and is the
same library already used for the fingerprint import analysis.

### SQLite / FTS5 over in-process maps

The index must survive between calls. An in-memory map rebuilt on every
`cargo run` would make every `search_code` call slow and would not handle
uncommitted files. SQLite with WAL allows concurrent readers without
blocking writers.

### Incremental sync by size, mtime and content hash

`size` and `mtime_ns` decide whether a file is even read: when both match the
stored row the file is skipped outright, which is what makes a warm sync cheap.
When they differ the file is read and a 64-bit FNV-1a hash of its bytes is
compared with the stored one, so a file that was only touched, and not edited,
updates its metadata without reparsing and without churning its symbols. A real
content change replaces that path's rows. Together they handle the common edit
cycle without a `--refresh` flag.

### FTS5 with the `unicode61` tokeniser

`unicode61` is what splits an identifier like `requireSession` into an indexable
token and handles non-ASCII identifiers. `porter` stemming is deliberately not
used: code identifiers are not English prose, and stemming them would fold
distinct names together.

### FTS5 ranking, then match-tier ranking in Rust

Candidates come from FTS5 with the built-in bm25 ranking, which is where the
phase-boundary matters: the phase collects a bounded candidate set without an
`ORDER BY` so FTS5 can stop early on a common token. The candidates are then
ranked in Rust by match quality - exact name, then prefix, then substring -
with source paths before test paths, then path and line, so the ordering is
deterministic and stays useful when the query is half an identifier.

### Per-language import resolution keyed on file language

A polyglot repository may have a `.py` file that imports a local `.js` file
and vice versa. Resolving each import in the context of the importer's language
(TS/JS imports, Rust `mod`, Python imports, Go imports, Java imports) avoids
false matches and keeps the resolution rules where the language grammar is.

---

## 8. Known Limits

### Cogito cannot force an agent to use it

The card guides, the guardrails nudge, the tools are there — and the model
still decides. An agent that ignores the card and calls `grep` directly
bypasses Cogito entirely.

### The agent's own system prompt dominates the token budget

Roughly 24k tokens in Gentle Shell and roughly 58k in Claude Code, of which
Cogito's card is about 320. The card's contribution to the total budget is
small but consistent.

### Performance at scale

At 100k files:
- p95 query time: ~17 ms
- First full index build: ~17.7 s
- Warm call on a clean worktree: ~185 ms (git status scan)
- Warm call while files are being edited: higher, because the working tree is walked

### Codex `AGENTS.md` is the only mechanism here

Codex has a hooks system documented in its CLI, but the hook contract was not
verified against a live Codex CLI session, so it is not implemented. The
`AGENTS.md` block is the only integration.

---

## 9. Storage Architecture

See [`docs/integrations.md`](docs/integrations.md) for the full schema reference.
Key design points:

- **File location**: `<project>/.cogito/state.db` (WAL beside it)
- **Incremental sync**: `indexed_files` keyed on `content_hash` + `size` + `mtime_ns`
- **FTS5**: `indexed_symbols_fts` and `indexed_files_fts`, `unicode61`, bm25
- **WAL journal mode**: concurrent readers never block writers
- **No legacy JSON migration**: the SQLite store is the only format; there is no
  reader for the old JSON tags and decisions files. Forward migrations do run:
  schema v2 drops the memory-era tables and schema v3 adds the index tables.

---

## 10. Future Work

> These items are out of scope for Level 1.

- **Reference graph / impact analysis** — Which files does a change affect?
- **Context pack** — Bundle a card + relevant outlines for a specific task
- **Embeddings** — Semantic similarity search alongside FTS5 keyword search
- **Shared/CI-built index** — One index shared across a team
- **Multi-repo, team server** — Aggregate across repositories
- **SSO, audit logs** — Enterprise requirements
- **Running `cargo publish`, `npm publish` or pushing a Homebrew tap** — needs
  credentials and a human decision; the mechanism and documentation are shipped
