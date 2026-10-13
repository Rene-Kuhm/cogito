# Level 1 — Product solid v1.0

Branch: `feat/level-1-v1`
Start: `9e16ff0`
Baseline: `cargo test --all` green (58 + 23 + 17 + 10 + 3 + 2 + 3 + 14 + 6 + 5 + 5 + 7 + 1 = 164 tests), rustc 1.99.0.

## Goal

Close the seven Level-1 gaps measured on `9e16ff0`:

1. Symbol/outline precision via tree-sitter instead of line regexes; imports
   resolved for Python, Go and Java.
2. Incremental index: a new uncommitted file appears in `search_code` without
   a manual `--refresh`.
3. Scale: file/symbol index in SQLite tables with FTS5, sub-50 ms queries on a
   100k-file repository.
4. Agent parity: project card + guardrails for Codex (via `AGENTS.md`) and
   opencode, not only Claude Code and Pi.
5. Cleanup: remove the "memory" era (decisions, semantic tags, compressor,
   REST API crate, legacy JSON persistence).
6. Publishing: releases with binaries, crates.io, npm and Homebrew.
7. `cogito bench`: a reproducible token benchmark over open-source repos.

## Non-goals

- Reference graph / impact analysis, `context_pack`, embeddings (Level 2).
- Shared/CI-built index, multi-repo, team server, SSO (Level 3).
- Running `cargo publish`, `npm publish` or pushing a Homebrew tap: those need
  credentials and stay a human decision. The mechanism plus docs were shipped.

## Work units

### WU1 — Remove the memory era
Delete `crates/cogito-harness-api/`, `core/src/decision/`, `core/src/semantic/`,
`core/src/persistence.rs`, `core/src/compressor/`, the decisions/tags SQLite
tables, `add_tag`/`search_tags`/`add_decision`/`search_decisions`/
`compress_context` from `Cogito`, the `compress|query|decision|tag|serve|export`
CLI subcommands, the `add_tag` MCP tool and the compressor benchmarks.
- [x] Acceptance: `cargo test --all` green, `rg 'compressor|semantic|Decision|SemanticTag' crates/` returns no production hit, workspace has 4 members.
- [x] Evidence: `5b3727a` — "refactor!: drop the memory era and keep only the context harness"

### WU2 — Tree-sitter symbols and Python/Go/Java imports
Split in two writers because they touch different concerns:

**WU2a — tree-sitter declaration extraction.** New `core/src/map/symbols.rs`:
tree-sitter declaration extraction for Rust, TS/TSX/JS, Python, Go, Java, C#,
Ruby, PHP, Kotlin, Swift; regex fallback. `map/outline.rs` consumes it for code
(Markdown headings stay line-based), and so does the fingerprint symbol list.
- [x] Acceptance: per-language declaration tests (kind, name, exact inclusive line range), never fewer declarations than the old regex path for files it understood.
- [x] Evidence: `dac67f8` — "feat(outline): extract declarations with tree-sitter"

**WU2b — Python, Go and Java imports.** `fingerprint::extract_imports` gains
Java; `layout::Resolver` resolves Python, Go and Java imports into project
files, so `related_files` works on those stacks.
- [x] Acceptance: import-resolution tests for `.py`, `.go` and `.java` in a temp project.
- [x] Evidence: `084f6bb` — "feat(related-files): resolve Python, Go and Java imports"

### WU3 — Incremental SQLite index with FTS5
Schema v2: `files`, `symbols`, `symbols_fts`, `imports`, `modules`. Indexing is
incremental by content hash + size + mtime over the working tree (uncommitted
files included), never keyed on git HEAD alone. `search_code` queries FTS5 with
bm25 ranking; `related_files` and `file_outline` read rows instead of rescanning.
- [x] Acceptance: a file created after the last scan is found by `search_code` with no `--refresh`; an edited file changes its symbols; a deleted file disappears. Scale check: p95 query time < 50 ms on a 100k-file synthetic repo.
- [x] Evidence: `bf8da28` (index + incremental sync) — "feat(index): keep an incremental file and symbol index in SQLite"
- [x] Evidence: `0ff4f39` (bounded cost + ranking) — "perf(index): bound search cost and rank source above tests"
- [x] Evidence: `1945db8` (index-backed queries) — "feat(index): answer search, outline and related from the index"

### WU4 — Codex and opencode parity
`cogito setup` maintains a managed block in `AGENTS.md` (repo root and/or
`~/.codex/AGENTS.md`) that hands Codex the project card and the read/grep
guardrails. The opencode plugin gains the same card injection and guardrail the
Pi package has.
- [x] Acceptance: idempotent block upsert (no duplicate block on re-run), uninstall removes it; opencode plugin test asserts card + guard behaviour.
- [x] Evidence: `454750a` — "feat(agents): give Codex and opencode the card and the guardrails"

### WU5 — `cogito bench`
Reproducible token benchmark: pinned open-source repo + commit, a fixed task
set, baseline (unassisted agent: list + grep + whole-file reads) vs cogito
(card + `search_code` + `file_outline`), reported in tokens, calls and bytes,
as markdown and JSON.
- [x] Acceptance: deterministic output for the same input; token counts from a
  real tokenizer; `docs/benchmarks.md` carries a committed result table.
- [x] Evidence: `850acc1` — "feat(bench): measure token savings reproducibly"

### WU6 — Publishing
`rust-version` + publishable metadata, a gated `publish-crates` job, a gated npm
publish job, a Homebrew formula template and a `docs/publishing.md` that lists
every secret and the exact human command.
- [x] Acceptance: `cargo publish --dry-run` clean for every crate, `cargo package --list` has no stray asset, formula renders against a release artifact name.
- [x] Evidence: `21cb3bf` — "feat(publish): ship the crates.io, npm and Homebrew paths"

### WU7 — Docs truth pass
README, SPEC, `docs/integrations.md` and `docs/benchmarks.md` describe only what
ships.
- [x] Acceptance: no doc mentions decisions, tags, compressor, REST API or `cogito query`; every documented command exists in `cogito --help`.
- [x] Evidence: `____` (this task — commit not yet assigned)

---

## Summary

All seven Level-1 gaps are closed. The product ships:

- **Tree-sitter** declaration extraction for 10 languages + Markdown
- **Incremental** SQLite/FTS5 index synced from the working tree
- **Import resolution** for TS/JS, Rust, Python, Go, Java
- **Four MCP tools**: `project_map`, `search_code`, `file_outline`, `related_files`
- **Card + guardrails** for Gentle Shell, Claude Code, opencode, Codex
- **`cogito bench`** with reproducible results and a committed table
- **Publishing docs** for crates.io, npm and Homebrew
- **Clean storage**: SQLite v2, no legacy JSON migration

---

## Known Limits

These limits are honest and belong in the documentation:

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

### Codex hook contract not verified

Codex has a hooks system documented in its CLI, but the hook contract was not
verified against a live Codex CLI session, so it is not implemented. The
`AGENTS.md` block is the only integration. The opencode plugin guardrail
behaviour was tested in `crates/cogito-harness/assets/opencode/cogito.js`.
