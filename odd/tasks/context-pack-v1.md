# context_pack v1 — one call that returns the fragments a task needs

Branch: `feat/context-pack`, from `fix/bench-baseline-gitignore` at `76e651a`.
Baseline: 211 tests pass, 0 fail, 3 ignored.

## Goal

An MCP tool `context_pack(path, task, budget)` that, in a single call, returns the
code and documentation fragments a task needs, ranked and inside a token budget.
Today the same result costs four or five round trips (`search_code`, then
`file_outline`, then reads, then `related_files`).

## Non-goals (v1)

- No reference graph and no impact analysis.
- No embeddings or semantic search: that is a later Nivel 2 step.
- No change to the index format or `FINGERPRINT_FORMAT` unless forced.

## Contract

- Input: free-text task (English or Spanish), budget in tokens (default 2000,
  maximum 8000), and `path` like every other tool.
- Candidates: symbols (name, kind, file, line), Markdown sections, and files by
  path.
- Neighbours: one hop of resolved imports per candidate, at a lower weight.
- Fragment range: the outline range, never the whole file.
- Ranking: deterministic. Term match in the name beats the path beats being a
  neighbour. Tie-break: sources before tests, then more changes, then path.
- Budget: fragments are added in rank order until the limit, measured with the
  same tokenizer as `cogito bench` (`cl100k_base`), not bytes/4.
- Output: one header line per fragment (`path:start-end`, kind, name, short
  reason) followed by the fragment's own text, plus tokens used and the budget.
  No candidates means a clear "no matches" message, never generic filler.

## Work units

### WU1 — Evaluation set, committed alone before any implementation
`benchmarks/context_pack_gold.json`: 10 tasks over the two pinned repositories
(ripgrep `3fce3b5`, bat `d9559c6`), each with the task text and the files or
symbols a person would consider necessary. Deliberately mixed: some tasks name
the identifier, some only describe behaviour, because the second kind is what
tests the ranking.
- [ ] Acceptance: every expected path and symbol verified to exist at the pinned commit.
- [ ] Evidence: commit `____`

### WU2 — One token counter, shared
`TokenCounter` lives in `commands/bench.rs` where `mcp.rs` cannot reach it.
Extract it to a small crate-level module used by both, with no behaviour change.
- [ ] Acceptance: `cogito bench` output unchanged; tests untouched.
- [ ] Evidence: commit `____`

### WU3 — Selection and rendering, in core, with an injected counter
New core module that turns `(task, budget)` into a ranked, budgeted pack. It takes
the token counter as a parameter, so `tiktoken` stays in the CLI crate and the
logic is unit-testable without it.
- [ ] Acceptance: determinism test; budget never exceeded, forced with a tiny budget; every fragment has `start <= end` within the file's real line count; a no-match task returns the empty message; a project without git behaves the same.
- [ ] Evidence: commit `____`

### WU4 — The MCP tool
`context_pack` registered with its request struct; `INSTRUCTIONS` names it as the
first option for a task; `test_mcp_lists_the_map_tools_only` updated.
- [ ] Acceptance: the tool appears in `tools/list`, returns fragments inside the budget for a real workspace, and the instructions mention it first.
- [ ] Evidence: commit `____`

### WU5 — Recall, measured
Harness over the pinned repositories producing recall@budget per task, with the
table committed to `docs/benchmarks.md`. The average is reported only with that
table behind it.
- [ ] Acceptance: the table is reproducible from the committed harness and the gold set.
- [ ] Evidence: commit `____`

### WU6 — Docs and report
README, SPEC and the integration tool tables describe `context_pack`; the closing
report states the numbers, the limits, and any departure from the brief.
- [ ] Evidence: commit `____`

## Risks

- The brief names `search_paths` as available. It no longer exists: it was
  replaced by `index::search` in `1945db8`. The candidates come from the index
  module instead. This is a deliberate departure to report.
- "Return the fragments" and "one line per fragment" pull in different
  directions. The pack carries each fragment's text, because returning only
  pointers would not save the round trips the tool exists to save.
- A lexical ranker cannot find code from behaviour alone. The eval set measures
  that honestly instead of hiding it.
