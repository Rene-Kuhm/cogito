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

- The quality of search results (relevance ranking)
- Round-trip latency to the LLM
- Cost of thinking or planning before calling tools
- Any costs after the first read (e.g., when the agent continues exploring)

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
```

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
`tiktoken-rs` and the `cl100k_base` vocabulary. The manifest pins each
repository to a full commit SHA, and the terms are the ones listed for that
repository in `benchmarks/repos.json`.

### ripgrep (BurntSushi/ripgrep) at `3fce3b5b`

| term | bl_find | bl_read | co_find | co_read | savings | pct% |
|------|--------:|--------:|--------:|--------:|--------:|-----:|
| regex (5) | 17,919 | 35,710 | 350 | 6,778 | 46,501 | 86.7 |
| Matcher (5) | 10,690 | 24,455 | 335 | 5,354 | 29,456 | 83.8 |
| search (5) | 43,952 | 44,252 | 384 | 14,755 | 73,065 | 82.8 |
| flags (5) | 5,338 | 50,228 | 397 | 4,171 | 50,998 | 91.8 |
| **TOTAL** | **77,899** | **154,645** | **1,466** | **31,058** | **200,020** | **86.0** |

### bat (sharkdp/bat) at `d9559c69`

| term | bl_find | bl_read | co_find | co_read | savings | pct% |
|------|--------:|--------:|--------:|--------:|--------:|-----:|
| syntax (5) | 174,355 | 28,934 | 1,385 | 19,445 | 182,459 | 89.8 |
| Theme (5) | 12,466 | 40,106 | 233 | 4,556 | 47,783 | 90.9 |
| highlighter (5) | 4,829 | 71,217 | 207 | 8,347 | 67,492 | 88.8 |
| assets (5) | 119,961 | 28,853 | 275 | 8,878 | 139,661 | 93.8 |
| **TOTAL** | **311,611** | **169,110** | **2,100** | **41,226** | **437,395** | **91.0** |

### Combined

| | baseline | cogito | savings | pct% |
|--|---------:|-------:|--------:|-----:|
| Both repositories, all eight terms | 713,265 | 75,850 | 637,415 | 89.4 |

Each row is a full run of the two strategies over the same matched files, so
the comparison isolates how the code is read, not how it is found.

### Interpretation

- **Finding cost varies with how common the term is.** For a term that appears
  in many files, the baseline's recursive search prints every matching line,
  which is where bat's `syntax` and `assets` rows spend most of their budget
  (174,355 and 119,961 tokens to find, against 1,385 and 275 with Cogito). For
  a term that appears in few files, listing and searching is cheap either way.
- **Reading cost is where the gap is stable.** Reading matched files whole
  costs 154,645 tokens on ripgrep and 169,110 on bat; outlines plus the matched
  line ranges cost 31,058 and 41,226. That is the part the product actually
  changes, and it holds across every term measured.
- **Combined: 89.4% fewer tokens** across both repositories and all eight
  terms, from 713,265 down to 75,850.

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
