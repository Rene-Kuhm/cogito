# Token Savings Benchmark

This document describes the `cogito bench` command, which measures the token
savings when an agent uses Cogito's tools versus exploring a repository the
ordinary way.

## What the Benchmark Measures

The benchmark compares two strategies for the question "find this term and read
the code around it":

### Baseline (an agent without Cogito)

1. List every project file path
2. Run a recursive text search (`grep -rn <term>`) for the term
3. Read each matched file in full

### Cogito

1. Print the project card (`cogito map --card`)
2. Call `search_code` with the term
3. Call `file_outline` on each matched file
4. Read only the line ranges that the match actually occupies

### What it does not measure

- Whether the answer is right. Recall is measured separately with the labelled
  gold set, through `cogito bench --report`; a token reduction only counts when
  the answer it reduces still meets the quality bar.
- Round-trip latency to the LLM
- Cost of thinking or planning before calling tools
- Any costs after the first read (e.g., when the agent continues exploring)
- The agent's own model tokens or tool calls. `cogito bench` counts the tool
  calls each strategy makes, not what a model spends thinking between them.

## Token Counting

Tokens are counted with **tiktoken-rs** using the `cl100k_base` vocabulary,
which is the closest available match to current coding-agent tokenizers
(Claude, GPT-4, etc.). If tiktoken-rs fails to initialise, the benchmark
falls back to `byte_count / 4`, and the output notes which counter was used.

Both raw bytes and token counts are reported, so readers can verify the
token-to-byte ratio independently.

## Output Format

The benchmark reports three numbers per term:

| Column | Description |
|--------|-------------|
| `bl_find` | Baseline finding cost: listing files + grep output |
| `bl_read` | Baseline reading cost: reading matched files in full |
| `co_find` | Cogito finding cost: card + search_code output |
| `co_read` | Cogito reading cost: file_outline + matched line ranges |
| `savings` | Token reduction from using Cogito |
| `pct%` | Savings as a percentage of baseline total |

The "no matches" case is reported separately and does not cause errors or
division by zero.

## How to Run

### Against a local checkout

```bash
# Run with default terms (search_code, project_map)
cogito bench --repo .

# With specific terms
cogito bench --repo . --terms search_code,project_map,map

# Limit matched files per term (default: 5)
cogito bench --repo . --terms search_code --limit 3

# Machine-readable JSON output
cogito bench --repo . --terms search_code --json

# The three-section report: quality, latency and cost, kept apart
cogito bench --repo . --terms search_code --report

# Repeat the measurement, and choose where the raw artifacts go
cogito bench --repeat 3 --out-dir ~/.cache/cogito/bench-runs
```

### Quality, latency and cost are reported separately

A fast wrong answer and a slow right one are not the same thing, so
`--report` prints three sections and never a combined score. Each section
answers one question on its own:

| Section | Question | Columns |
|---------|----------|---------|
| `QUALITY` | Did the pack return what a person needs? | `recall@budget` per task, per language and overall, over the labelled gold set |
| `LATENCY` | How long does it take? | `index_sync_ms` per repository, `query_ms` per task or term |
| `COST` | What does it spend? | `pack_tokens` (what the pack actually used) and `tool_calls`, against the calls plain exploration needs |

The report also prints its own provenance, because a number without it cannot
be reproduced: the exact commit SHA of every measured worktree, whether that
worktree was `clean` or `dirty` (with the count of changed paths), how many
repetitions were run, which tokenizer produced the counts, and the artifact
directory the run was written to. A worktree where git could not be read is
reported as `no-git`, never as clean.

Recall is measured through the same `pack::build` and the same shared
`cl100k_base` counter the `context_pack` tool uses, so it describes the tool an
agent gets rather than a reimplementation of it. A repository that is not
checked out is reported and skipped: it is never a miss and never a zero. A pack
that failed to build is recorded with its reason and left out of the average,
for the same reason.

Repetitions keep every run. Quality and cost are deterministic, so repeats agree;
latency is not, which is why `--repeat` exists and everything is kept.

### Raw artifacts for every run

Every run writes `<out-dir>/<timestamp>/` outside any measured worktree, and
nothing is discarded: failures and zero-result terms are in the artifact.
Writing them inside the tree being measured would dirty it between runs and
change what the next run sees. The default root is
`~/.cache/cogito/bench-runs`, overridable with `--out-dir` or
`COGITO_BENCH_OUT_DIR`. The two files are:

- `run.json` — the raw record: every repetition, every term and task, the
  provenance, and the error when the run failed.
- `report.txt` — exactly the text the run printed.

Without `--report` the printed output is unchanged; the artifact path is
announced on stderr so stdout keeps carrying only the table (or the JSON).

### Against the pinned manifest

The manifest at `benchmarks/repos.json` lists small, well-known repositories
with meaningful search terms. When `--repo` is omitted, `cogito bench`
clones each repository into `.cogito/bench-cache/` at its pinned commit.

```bash
# Clone and benchmark all repositories in the manifest
cogito bench
```

If a clone fails, the benchmark prints an error and continues with the
remaining repositories. No single network failure stops the entire run.

## Results Table

Produced by running `cogito bench` against the pinned manifest with
`tiktoken-rs` and the `cl100k_base` vocabulary, at commit `0e3eebc` (2026-10-13).
The manifest pins each repository to a full commit SHA, and the terms are the
ones listed for that repository in `benchmarks/repos.json`.

The baseline lists and searches what an agent would see: tracked files and
untracked files that `.gitignore` does not exclude. Earlier runs walked the
directory tree instead, which inflated the baseline on repositories with
ignored output. These are the corrected numbers.

### ripgrep (BurntSushi/ripgrep) at `3fce3b5b`

| term | bl_find | bl_read | co_find | co_read | savings | pct% |
|------|--------:|--------:|--------:|--------:|--------:|-----:|
| regex (5) | 18,747 | 35,710 | 349 | 6,787 | 47,321 | 86.9 |
| Matcher (5) | 11,801 | 24,455 | 334 | 5,383 | 30,539 | 84.2 |
| search (5) | 43,680 | 44,252 | 383 | 14,765 | 72,784 | 82.8 |
| flags (5) | 6,643 | 50,228 | 396 | 4,185 | 52,290 | 91.9 |
| **TOTAL** | **80,871** | **154,645** | **1,462** | **31,120** | **202,934** | **86.2** |

### bat (sharkdp/bat) at `d9559c69`

| term | bl_find | bl_read | co_find | co_read | savings | pct% |
|------|--------:|--------:|--------:|--------:|--------:|-----:|
| syntax (5) | 179,645 | 28,934 | 1,385 | 19,457 | 187,737 | 90.0 |
| Theme (5) | 20,115 | 40,106 | 233 | 4,570 | 55,418 | 92.0 |
| highlighter (5) | 12,687 | 71,217 | 207 | 8,362 | 75,335 | 89.8 |
| assets (5) | 126,367 | 28,853 | 275 | 8,890 | 146,055 | 94.1 |
| **TOTAL** | **338,814** | **169,110** | **2,100** | **41,279** | **464,545** | **91.5** |

### Combined

| | baseline | cogito | savings | pct% |
|--|---------:|-------:|--------:|-----:|
| Both repositories, all eight terms | 743,440 | 75,961 | 667,479 | 89.8 |

Each row is a full run of the two strategies over the same matched files, so
the comparison isolates how the code is read, not how it is found.

### Interpretation

- **Finding cost varies with how common the term is.** For a term that appears
  in many files, the baseline's search prints every matching line, which is
  where bat's `syntax` and `assets` rows spend most of their budget (179,645
  and 126,367 tokens to find, against 1,385 and 275 with Cogito). For
  a term that appears in few files, listing and searching is cheap either way.
- **Reading cost is where the gap is stable.** Reading matched files whole
  costs 154,645 tokens on ripgrep and 169,110 on bat; outlines plus the matched
  line ranges cost 31,120 and 41,279. That is the part the product actually
  changes, and it holds across every term measured.
- **Combined: 89.8% fewer tokens** across both repositories and all eight
  terms, from 743,440 down to 75,961.

### Caveats

- These numbers are specific to the two pinned repositories (ripgrep and bat)
  and the eight terms listed. Other repositories and terms will produce
  different savings percentages.
- A term that matches thousands of files may show lower savings because
  the baseline reads more files. Use `--limit` to cap this.
- The baseline reading strategy (whole-file reads) is intentionally naive.
  Real agents might use heuristics to avoid reading irrelevant sections.

## Benchmarking Other Repositories

To add a repository to the pinned manifest:

1. Choose a small, permissively licensed open-source repository
2. Identify terms that appear in the codebase and produce meaningful matches
3. Pin to a specific git commit (not `main` or `master`)
4. Add an entry to `benchmarks/repos.json`

```json
{
  "manifest_version": "1.0",
  "repos": [
    {
      "name": "ripgrep",
      "url": "https://github.com/BurntSushi/ripgrep",
      "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7",
      "terms": ["regex", "Matcher", "search", "flags"]
    }
  ]
}
```

---

## Context pack recall

`context_pack` answers a task in one call by returning the fragments it needs,
ranked and inside a token budget. Whether it returns the *right* fragments is
measured, not asserted.

The gold set is [`benchmarks/context_pack_gold.json`](../benchmarks/context_pack_gold.json):
ten tasks over the two repositories already pinned for `cogito bench`, each with
the files and symbols a person would consider necessary. Three tasks name the
identifier; seven only describe behaviour, because the descriptive ones are the
honest test of a lexical ranker.

**recall@budget** = expected entries that appear in the pack, over the total
expected, per task, with the pack built at that task's budget. A file counts as
found when a fragment's path matches; a symbol when a fragment's name matches.

Reproduce it with:

```bash
COGITO_RECALL=1 cargo test --release -p cogito-harness \
    --test context_pack_recall -- --ignored --nocapture
```

The repositories must be in `.cogito/bench-cache`, which is where `cogito bench`
clones them; a missing checkout is reported and skipped rather than counted as a
miss. The harness drives the real MCP tool through a real process, so it measures
the path an agent gets, including the real `cl100k_base` tokenizer, and it runs
every task twice and requires identical answers.

Three tables come out of one run of that command.

### Every task, with its budget and what it spent

| task | lang | budget | used | expected | hits | recall |
|---|---|---:|---:|---:|---:|---:|
| rg-01 | en | 2000 | 882 | 2 | 2 | 100% |
| rg-02 | en | 2000 | 1994 | 2 | 2 | 100% |
| rg-03 | es | 2000 | 1972 | 3 | 2 | 67% |
| rg-04 | es | 2000 | 1989 | 2 | 2 | 100% |
| rg-05 | en | 2000 | 2000 | 4 | 0 | 0% |
| bat-01 | en | 2000 | 1988 | 2 | 2 | 100% |
| bat-02 | es | 1000 | 989 | 2 | 0 | 0% |
| bat-03 | en | 2000 | 1978 | 2 | 2 | 100% |
| bat-04 | es | 2000 | 90 | 2 | 0 | 0% |
| bat-05 | en | 4000 | 3998 | 2 | 2 | 100% |

### By language

| lang | tasks | recall |
|---|---:|---:|
| en | 6 | 83.3% |
| es | 4 | 41.7% |
| all | 10 | 66.7% |

The split is the finding. One average over both languages hides that an English
task about English code works well and a Spanish description of the same code
does not: `bat-04`, `¿cómo se mapea el nombre de un archivo a una sintaxis
concreta?`, shares no token with `src/syntax_mapping.rs` or `SyntaxMapping`.

### Tier weight sensitivity

The relative strength of the name, path and neighbour tiers is a judgement, so
it is measured rather than asserted. `COGITO_PACK_TIERS` lets the harness run
alternatives through the real tool:

| weights | overall | en | es |
|---|---:|---:|---:|
| 10/3/1 (default) | 66.7% | 83.3% | 41.7% |
| 5/2/1 | 61.7% | 75.0% | 41.7% |
| 20/5/1 | 66.7% | 83.3% | 41.7% |

The spread is 5.0 percentage points, under the 10 the harness declares as its
margin, so the default holds on this set. That is a weak claim and worth saying
plainly: with ten tasks a single task moves the overall average by up to ten
points, so this shows the choice is **stable, not optimal**. A different set
could separate the three.

### What the misses are, measured

- **Vocabulary overlap decides more than anything else.** The three tasks that
  name the identifier all score 100%, and the six English tasks average 83.3%
  against 41.7% for the four Spanish ones.
- **`rg-05` needs a semantic step.** "How are the command-line flags turned into
  a configured search?" expects `crates/core/flags/hiargs.rs` and its `HiArgs`
  and `LowArgs` types. The word `flags` is not discriminating here: 1323 indexed
  symbols sit under paths containing it, because one large definitions file
  shares that directory, so the rarest word in the task is `configured` and
  `ConfiguredHIR` wins. Reaching the expected answer needs the leap a person
  makes and a lexer cannot: flags means the flags module means those two types.
- **`bat-02` and `bat-04` are Spanish descriptions of English code**, which is
  the whole reason the table is split. `bat-04` found no candidate at all and
  used 90 tokens, which is the correct behaviour: it says so instead of returning
  something generic to fill the budget.
- **`rg-03` finds the gitignore matcher but not the walker** that reads the
  rules while traversing, because only the first shares a word with the task.

### What was tried and did not hold

Three changes to the ranking were measured and kept out of the product, so the
numbers above describe the code that ships:

- **Ordering by coverage first** (how many distinct task terms a candidate
  matches, before how rare they are): the labelled set stayed at 66.7%, and a
  second set of new tasks in a private project stayed at the same numbers. It
  moved nothing, so it was reverted.
- **A Spanish-to-English glossary of about fifty development terms**, added to
  the task terms next to the originals: the labelled set stayed at 66.7%. On a
  second set of five tasks written before the measurement, in areas the
  glossary had not been built from, recall fell by 13.4 points, because one
  added English word (`device`) pulled a large, weakly relevant group of
  fragments into the budget. It was reverted. Tuning the glossary on the same
  tasks would only fit those tasks.
- **Earlier checks on the naming rule.** A word inside a longer word must not
  match: `defined` must not match `undefined`. That rule is kept, with tests.

The lesson the three share: a term that the code does not spell the same way
can add noise faster than it adds signal, and a change that helps the tasks it
was tuned on is not evidence on tasks it has not seen.

### Where the product stops

Stated as limits, not as roadmap:

- **Queries in Spanish about English code** get markedly less recall than
  English queries, on the public set and on every other set measured.
- **A broad question can fill its budget with the first candidates** that share
  a common word, and drop a more relevant one that matches a rarer word only
  once. The budget is respected, but the choice inside it is lexical.
- **Concepts that the code names differently from the question** (a flag
  module, a configured search, a registry of adapters) need a step from meaning
  to identifier that the ranker does not take.
- **Measured on two repositories and ten tasks.** The numbers are calibrated on
  this set. Other repositories will differ, and a new set should be written
  before a change is tried on it.

The honest summary: this is a lexical ranker. It is strong when the task shares
vocabulary with the code, which is the usual case for an agent that has just
read an error message or a symbol name, and it does not bridge morphology across
languages or multi-step domain knowledge. Closing that needs embeddings or the
reference graph, not more tuning of this ranking.
