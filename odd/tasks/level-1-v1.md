# Level 1 — Producto sólido v1.0

Branch: `feat/level-1-v1`
Start: `9e16ff0`
Baseline: `cargo test --all` green (100 + 3 + 7 + 3 + 7 + 1 tests), rustc 1.99.0.

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
  credentials and stay a human decision. We ship the mechanism plus docs.

## Execution order

WU2 → WU1 → WU3 → WU4 → WU5 → WU6 → WU7.

WU2 lands while `compressor/ast_*.rs` is still present, because those modules are
the working reference for tree-sitter node kinds per grammar. WU1 then deletes
that reference together with the memory era, and WU3 rebuilds storage on a clean
surface.

## Work units

### WU1 — Remove the memory era
Delete `crates/cogito-harness-api/`, `core/src/decision/`, `core/src/semantic/`,
`core/src/persistence.rs`, `core/src/compressor/`, the decisions/tags SQLite
tables, `add_tag`/`search_tags`/`add_decision`/`search_decisions`/
`compress_context` from `Cogito`, the `compress|query|decision|tag|serve|export`
CLI subcommands, the `add_tag` MCP tool and the compressor benchmarks.
- [ ] Acceptance: `cargo test --all` green, `rg 'compressor|semantic|Decision|SemanticTag' crates/` returns no production hit, workspace has 3 members.
- [ ] Evidence: commit `____`

### WU2 — Tree-sitter symbols and Python/Go/Java imports
Split in two writers because they touch different concerns:

**WU2a — tree-sitter declaration extraction.** New `core/src/map/symbols.rs`:
tree-sitter declaration extraction for Rust, TS/TSX/JS, Python, Go, Java, C#,
Ruby, PHP, Kotlin, Swift; regex fallback. `map/outline.rs` consumes it for code
(Markdown headings stay line-based), and so does the fingerprint symbol list.
- [ ] Acceptance: per-language declaration tests (kind, name, exact inclusive line range), never fewer declarations than the old regex path for files it understood.
- [ ] Evidence: commit `____`

**WU2b — Python, Go and Java imports.** `fingerprint::extract_imports` gains
Java; `layout::Resolver` resolves Python, Go and Java imports into project
files, so `related_files` works on those stacks.
- [ ] Acceptance: import-resolution tests for `.py`, `.go` and `.java` in a temp project.
- [ ] Evidence: commit `____`

### WU3 — Incremental SQLite index with FTS5
Schema v2: `files`, `symbols`, `symbols_fts`, `imports`, `modules`. Indexing is
incremental by content hash + size + mtime over the working tree (uncommitted
files included), never keyed on git HEAD alone. `search_code` queries FTS5 with
bm25 ranking; `related_files` and `file_outline` read rows instead of rescanning.
- [ ] Acceptance: a file created after the last scan is found by `search_code` with no `--refresh`; an edited file changes its symbols; a deleted file disappears. Scale check: p95 query time < 50 ms on a 100k-file synthetic repo.
- [ ] Evidence: commit `____`

### WU4 — Codex and opencode parity
`cogito setup` maintains a managed block in `AGENTS.md` (repo root and/or
`~/.codex/AGENTS.md`) that hands Codex the project card and the read/grep
guardrails. The opencode plugin gains the same card injection and guardrail the
Pi package has.
- [ ] Acceptance: idempotent block upsert (no duplicate block on re-run), uninstall removes it; opencode plugin test asserts card + guard behaviour.
- [ ] Evidence: commit `____`

### WU5 — `cogito bench`
Reproducible token benchmark: pinned open-source repo + commit, a fixed task
set, baseline (unassisted agent: list + grep + whole-file reads) vs cogito
(card + `search_code` + `file_outline`), reported in tokens, calls and bytes,
as markdown and JSON.
- [ ] Acceptance: deterministic output for the same input; token counts from a
  real tokenizer; `docs/benchmarks.md` carries a committed result table.
- [ ] Evidence: commit `____`

### WU6 — Publishing
`rust-version` + publishable metadata, a gated `publish-crates` job, a gated npm
publish job, a Homebrew formula template and a `docs/publishing.md` that lists
every secret and the exact human command.
- [ ] Acceptance: `cargo publish --dry-run` clean for every crate, `cargo package --list` has no stray asset, formula renders against a release artifact name.
- [ ] Evidence: commit `____`

### WU7 — Docs truth pass
README, SPEC, `docs/integrations.md` and `docs/benchmarks.md` describe only what
ships.
- [ ] Acceptance: no doc mentions decisions, tags, compressor, REST API or `cogito query`; every documented command exists in `cogito --help`.
- [ ] Evidence: commit `____`

## Risks

- WU3 changes an on-disk format; `.cogito/state.db` needs a forward migration
  and a rebuild path for fingerprints written by the old schema.
- Removing `compressor/` deletes ~4.5k lines of tested code; the `ast_*` modules
  are the reference implementation for the tree-sitter symbol extractor in WU2,
  so WU2 must land before WU1 deletes them — or WU2 keeps its own helpers.
- Codex `AGENTS.md` and opencode plugin hooks are external, versioned APIs;
  WU4 needs a researched contract, not a guess.
