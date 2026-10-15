# Nivel 2 v1 — measured semantics and impact, in that order

Branch: `docs/level-2-plan`, from `main` at `47ec452`.
Baseline: Nivel 1 merged (PR #29). Public benchmark: recall@budget 66.7 % (in 83.3 %, es 41.7 %). FTTH private set: 34.2 % (es 19.2 %, en 66.7 %).

## Goal

Close the two gaps Nivel 1 documented as not closable by tuning the lexical ranker:

1. **Semantic gap:** a task whose words do not appear in the code (behaviour described in Spanish over English code).
2. **Impact gap:** "what does this change affect?", which no index can answer today because cogito has no reference graph, only import edges.

Every step is measured before it is adopted. A step that does not beat the frozen baseline on the holdout sets is reverted, as the glossary and short-term experiments were in Nivel 1.

## Non-goals (v1)

- No change to the default behaviour of `context_pack` until a track passes its acceptance on the holdout sets.
- No embeddings model enabled by default. It stays behind a feature flag.
- No full call graph, no type resolution (no LSP), no cross-repo analysis.
- No dead-code detection, community clustering or UI.
- No index format change unless a track forces it, and then bump `FINGERPRINT_FORMAT` in the same commit.

## Measurement protocol (applies to all tracks)

Adopted from `docs/MEASURING_SAVINGS.md` in codebase-memory-mcp (MIT; ideas only, no copied code).

- **Freeze before measuring.** Record the repository SHA, the task set, the tokenizer, the budget, and the number of repetitions before looking at results.
- **Clean worktrees.** Each condition runs in its own detached worktree at the frozen SHA, with `git status` empty. Artifacts are written outside both worktrees.
- **Keep everything.** Failed runs, zero-result queries and regressions are kept in the artifacts. Nothing is dropped after the fact.
- **Three separate questions.** Report answer quality (recall@budget and hit rank), latency (index sync and query time), and cost (tokens in the pack, tool calls per task) as three columns, never merged into one score.
- **Holdout discipline.** Tuning uses the existing sets only. Acceptance uses sets committed before any implementation of the track they judge.

## Tracks

Track A (evaluation) runs first. B and C start only after A's sets are committed. B and C may run in parallel after that.

Ownership: one integrator owns `crates/cogito-harness-core/src/pack.rs`. B and C send their changes to it as small, reviewed patches, never as parallel edits.

### Track A — evaluation sets, committed before measuring

Branch: `feat/eval-holdout`.

#### A1 — Holdout set for the public repositories
`benchmarks/context_pack_holdout.json`: at least 10 new tasks over ripgrep and bat at their pinned commits. Mix: half name the identifier, half describe behaviour only, and at least 3 are Spanish descriptions of English code. Each task lists the files or symbols a person would need.
- [ ] Acceptance: every expected path and symbol verified to exist at the pinned commit with `git grep` or the outline, recorded in the file.
- [ ] Evidence:

#### A2 — Holdout set for FTTH (private)
Five or more new tasks added to `ftth-gold.json` in `cogito-private`, written before any Track B or C code exists. Not committed to the public repository; the public repo only receives the results summary.
- [ ] Acceptance: each expected path verified at the pinned private commit.
- [ ] Evidence:

#### A3 — Measurement harness
`cogito bench` reports latency and tool-call count next to recall, and writes raw run artifacts outside the worktree. Baseline numbers for `main` are frozen as the reference for Tracks B and C.
- [ ] Acceptance: `cogito bench` output on `main` reproduces the frozen baseline within the stated repeat variance; artifacts are kept for every run.
- [ ] Evidence:

### Track B — semantic search, behind a feature flag

Branch: `feat/embeddings`. Starts after A3.

#### B1 — Feasibility prototype, time-boxed, not integrated
A standalone measurement, not wired into `pack.rs`: model size on disk, index time for a 100k-file repository on CPU, and query latency on CPU. Pass limits proposed: model at or below 100 MB, query at or below 50 ms, indexing time reported without a limit but recorded.
- [ ] Decision gate: the user accepts or rejects the model size and the limits. Nothing in B2 starts before this decision.
- [ ] Evidence:

#### B2 — Embeddings as an optional signal
Vector scores added as one signal next to the lexical rank, behind the `embeddings` feature flag, off by default. Stored in the index database, rebuilt on fingerprint change.
- [ ] Acceptance: on the A1 and A2 holdout sets, recall@budget improves on the Spanish-over-English subset without regressing the identifier subset, with flag on. With flag off, output is byte-identical to `main`.
- [ ] Acceptance: latency and tool-call numbers reported in the three columns.
- [ ] Evidence:

#### B3 — Adopt or revert
If B2 does not pass, revert the feature and record the numbers in `docs/benchmarks.md`. A null result is kept as documentation.
- [ ] Evidence:

### Track C — impact of a change, from the diff

Branch: `feat/impact`. Starts after A3. Can run in parallel with B.

#### C1 — `impact` from uncommitted changes (minimal)
New MCP tool `impact(path, base?)`. Steps: take the changed hunks from `git diff` (uncommitted by default, or `base..HEAD`), map each hunk to the declarations it touches using the existing outline ranges, then list the files that import those files (`related_files` reverse edges) and the files that name those symbols (index search). Output: changed symbols, affected files, and a risk tier based only on count of importers, with the reason stated.
- [ ] Acceptance: on a fixture repository with a known change, the affected set equals the expected set exactly; a change with no declarations (only comments or whitespace) returns an explicit "no declarations touched" message.
- [ ] Acceptance: tool surface change updates `mcp_test.rs` and the agent instructions in the same commit.
- [ ] Evidence:

#### C2 — Reference edges for TypeScript and Rust
Name-based reference edges (a call or type use of an imported or same-file name), resolved through the import edges already in the index. No type inference. Only TypeScript and Rust in v1; other languages report "not supported" for impact instead of guessing.
- [ ] Acceptance: precision and recall of reference edges measured on a hand-checked sample from the TypeScript and Rust holdout repositories, reported with the sample size.
- [ ] Evidence:

#### C3 — Impact in the pack (only if C2 passes)
`context_pack` may add impacted files as neighbours at a lower weight, when the task names a symbol that is being changed. Adopted only if it passes the A1 and A2 holdouts with flag on.
- [ ] Acceptance: as B2, plus no growth in fragments for tasks that do not touch changed code.
- [ ] Evidence:

## Track D — no re-reading the project (the goal the user cares about most)

Goal: a model working on a project should not reread the whole project to continue. Today the guardrail only nudges, and it has three gaps, verified in `crates/cogito-harness/src/claude_hook.rs`:

1. **A repeat read always passes.** `first_time` records the key per session, so the second whole-file read of the same unchanged file goes through. A model that insists rereads everything.
2. **Only `Read` and `Grep` are covered.** `Bash` with `cat`, `head`, `sed -n` or `rg` on large files is not guarded.
3. **No memory of what was already read.** Nothing tells the model "this file is unchanged since the read at step N".

Work units:

- **D1 — Unchanged-file dedupe.** Keep, per session, the content hash of each whole-file read. A repeat of an unchanged file returns the outline and "unchanged since the read at <step>", not the full content, and the reason is stated. A changed file is read normally. Acceptance: fixture test with a repeat read of an unchanged file returns the short answer; after an edit it returns the full content.
- **D2 — Guard `Bash` file reads.** Same rule for `cat`, `head`, `tail`, `sed -n` and `nl` on files above the outline threshold. Acceptance: test per command; a non-file command is never blocked.
- **D3 — Real-session measurement.** Count input tokens of tool results per session from the transcripts, with and without the guardrail, on the same three tasks. Report it in the three columns (quality, latency, cost) with the sample size. Acceptance: numbers in `odd/tasks/session-tokens-v1.md`, raw transcripts referenced, no claim without a run.
- **D4 — Project card as the first step.** Confirm that the session card and `context_pack` are what a model sees first. Acceptance: a test that the card names the entry points for a fixture project.

Non-goal: blocking a model from reading a file it has been told is unchanged. The guardrail explains and returns the cheap answer; the model can still ask for the full file in an edit.

## Decisions the user owns

Item 1 is decided; items 2 and 3 are still the recommendation.

1. **Embedding model.** Decided by the user: the size cap moves from 100 MB to ~170 MB, and the model must be a code model. Chosen: `jinaai/jina-embeddings-v2-base-code`, ONNX quantized (`onnx/model_quantized.onnx`, 161.9 MB, 768d, Apache-2.0). Measured in B1 follow-up: query p50 5.3 ms, p95 5.8 ms on CPU; indexing 469 ms per file on the 1.000-file subset, which extrapolates to ~13 h for 100k files and is a known cost to design around (incremental index, not full rebuilds). Still behind the `embeddings` flag, off by default. Quality is not yet measured: B2 must show recall gains on the holdouts before it is adopted.
2. **Languages for references.** TypeScript and Rust first (C2), as proposed. Recommendation: keep it; Python and Go come after measurement.
3. **Start order.** Track A first, unconditionally. Recommendation: keep it.

## Risks

- **Overfitting to the sets we know.** Mitigated by A1 and A2 being written before any Track B or C code. Any change to a holdout after it is committed must be recorded as a new version, not edited in place.
- **Semantic gains that are actually lexical.** A vector score can match the same words the lexical rank already matches. B2 must show gains on the Spanish-over-English subset specifically, not on the total.
- **Reference edges with false precision.** Name-based edges will match unrelated symbols with the same name. C2 must report precision on a sample, and C1 must state in its output that impact is an over-approximation.
- **Tool surface growth.** Each new MCP tool adds instruction text the agent reads every session. C1 is the only new tool in v1; `context_pack` stays the default entry point.
- **Measured against ourselves.** The CBM preprint reports 83 % answer quality for its graph tool against 92 % for file-by-file exploration. Token savings in cogito are only meaningful if answer quality stays at baseline; the three-column report exists for that reason.

## Out of scope for this file

Publishing v0.1.0 to crates.io, npm or Homebrew. That is a separate decision, after Nivel 2 is measured.
