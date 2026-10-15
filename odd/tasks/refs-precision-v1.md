# Reference edges v1 — precision and recall

Measured for `feat(impact): name-based reference edges for TypeScript and Rust`
(T1 of `odd/tasks/refs-graph-v1.md`). Two runs are recorded here: the original
Rust one, whose instrument is `crates/cogito-harness-core/tests/refs_measure_test.rs`
(opt-in and `#[ignore]`d, and whose numbers below are its verbatim output), and
the TypeScript one that closed the gap that run left, driven ad hoc against the
same `impact::build`.

## What was measured (Rust)

- Repos: the project's holdout set (`benchmarks/repos.json`) — ripgrep
  `3fce3b5` and bat `d9559c6`, both Rust, read from a copy under
  `.cogito/refs-eval/` (gitignored). The measured repositories were only read:
  one line inside each sampled declaration is changed and restored immediately
  after the impact is built, and `git status` was clean in both copies
  afterwards.
- Sample size: **8 samples**, where a sample is one (declaring file, declared
  name) pair — 4 for bat, 4 for ripgrep. Each sample is measured by a real
  uncommitted diff of one line inside that declaration, so the only touched
  symbol is the sampled one (the harness asserts that, and the assertion passed
  for all 8).
- Labels: **made by an agent reading the source, not by a person.** The truth
  rule a human would apply: a file is a true reference when it uses the name and
  imports the module that declares it — by a `use`/`import` statement or an
  inline qualified path naming that module. Re-export bindings (`pub use`) are
  not uses; a file that declares its own homonym is not a reference.
- Definition: predicted = non-self files in `impact.references` for that sample
  (that is the affected-file claim). Precision is `tp / (tp + fp)`, recall is
  `tp / (tp + fn)`,`n/a` when a sample predicts nothing.

## Results (Rust)

| repo | file | symbol | predicted | truth | tp | fp | fn |
|---|---|---|---|---|---|---|---|
| bat | src/lessopen.rs | LessOpenPreprocessor | src/controller.rs | src/controller.rs | 1 | 0 | 0 |
| bat | src/diff.rs | LineChange | src/decorations.rs | src/decorations.rs | 1 | 0 | 0 |
| bat | src/input.rs | OpenedInput | src/assets.rs, src/controller.rs, src/printer.rs | src/assets.rs, src/controller.rs, src/lessopen.rs, src/printer.rs | 3 | 0 | 1 |
| bat | src/bin/bat/config.rs | system_config_file | — | src/bin/bat/main.rs | 0 | 0 | 1 |
| ripgrep | crates/printer/src/util.rs | NiceDuration | crates/printer/src/stats.rs | crates/printer/src/stats.rs | 1 | 0 | 0 |
| ripgrep | crates/printer/src/util.rs | DecimalFormatter | crates/printer/src/hyperlink/mod.rs | crates/printer/src/hyperlink/mod.rs, crates/printer/src/standard.rs | 1 | 0 | 1 |
| ripgrep | crates/printer/src/hyperlink/mod.rs | HyperlinkAlias | crates/printer/src/hyperlink/aliases.rs | crates/printer/src/hyperlink/aliases.rs | 1 | 0 | 0 |
| ripgrep | crates/ignore/src/walk.rs | DirEntry | crates/ignore/src/pathutil.rs | crates/core/haystack.rs, crates/ignore/src/dir.rs, crates/ignore/src/pathutil.rs | 1 | 0 | 2 |

| repo | tp | fp | fn | precision | recall |
|---|---|---|---|---|---|
| bat | 5 | 0 | 2 | 1.000 | 0.714 |
| ripgrep | 4 | 0 | 3 | 1.000 | 0.571 |
| **total (8 samples)** | **9** | **0** | **5** | **1.000** | **0.643** |

## Reading the numbers (Rust)

Precision is 1.000 over 9 predicted edges: no false positive in the sample, in
particular no homonym. That is the point of the change — the previous signal
(`files_declaring`) would have reported, for `OpenedInput` or `DirEntry`, every
file that *declares* a name of its own.

Recall is 0.643, and **every one of the 5 misses is upstream of the reference
logic, in the import edges the index already holds** (T1 was told to resolve
with those edges, not to rebuild them). No miss is a use the identifier scan
failed to see:

1. Nested `use crate::{ … module::{Name} … }` (3 of 5: bat `src/lessopen.rs`,
   ripgrep `src/standard.rs`, ripgrep `src/dir.rs`). `fingerprint::extract_imports`
   captures only `crate::` from that form, which resolves to nothing.
2. A binary target (bat `src/bin/bat/main.rs`): `crate::config` resolves to
   `<pkg>/src/config.rs` because `Resolver::crate_src` assumes a `src/` module
   root; for a bin target the root is `src/bin/bat/`.
3. A cross-crate inline qualified path (ripgrep `crates/core/haystack.rs` writes
   `ignore::DirEntry`): there is no import statement naming the module at all.

Two more limitations the sample shows but does not count as misses: a `mod`
based path (`super::foo`) also leaves no import row, and a `pub use` re-export
(`crates/printer/src/lib.rs`) is a binding, not a use, so it is deliberately not
reported.

## TypeScript: measured (ky @ 3541888)

The gap this section used to describe is closed with one TypeScript repository,
cloned outside the worktrees and measured with the same protocol. The exact-set
fixtures in `crates/cogito-harness-core/tests/refs_test.rs` (importing users in,
imported but unused out, homonym declaration out, comment and string mentions
out, self use in) are still there; this is the figure from a real repository
that they cannot give.

### What was measured

- Repo: `sindresorhus/ky` (MIT), pinned at commit
  `3541888878c4d303a8bc2ad61815f4e048f55a5d` (the `2.2.0` commit, 2026-10-09).
  115 tracked files, 4 917 lines under `source/` and 30 831 under `test/`. Its
  pinned entry is `benchmarks/repos-ts.json`. The clone is
  `~/.cache/cogito/ts-bench/ky`, outside every worktree, made with
  `git clone https://github.com/sindresorhus/ky.git`.
- Sample size: **9 samples**, where a sample is one (declaring file, declared
  name) pair: five internal functions or consts, one interface, one option
  validator and two public error surfaces. Same protocol as the Rust run: one
  line inside the sampled declaration gets ` // refs-measure` appended, the
  impact of that uncommitted diff is read through the MCP `impact` tool (the same
  `impact::build` the Rust instrument calls), and the line is restored
  immediately after. `git status` was clean in the clone afterwards. The index
  lives in the clone's `.cogito/` directory (gitignored); nothing else in the
  clone was written.
- Labels: **made by an agent reading the source, not by a person**, under the
  same truth rule as the Rust run, spelled out here: a file is a true reference
  when it uses the name and reaches the declaration through the module surface a
  person would follow — a relative import of the declaring file, or an import of
  the package's `source/index.ts` barrel, which re-exports it. An
  `import type … from` counts; a re-export (`export … from`) is a binding, not a
  use; a mention inside a comment or a string literal is not a use.

### Results

| repo | file | symbol | predicted | truth | tp | fp | fn |
|---|---|---|---|---|---|---|---|
| ky | source/utils/is.ts | isNonArrayObject | source/core/Ky.ts, source/utils/merge.ts, source/utils/normalize.ts | source/core/Ky.ts, source/utils/merge.ts, source/utils/normalize.ts | 3 | 0 | 0 |
| ky | source/utils/merge.ts | mergeHooks | source/core/Ky.ts | source/core/Ky.ts | 1 | 0 | 0 |
| ky | source/core/retry-timing.ts | getRetryTimingHeader | source/core/Ky.ts, test/retry-timing.ts | source/core/Ky.ts, test/retry-timing.ts | 2 | 0 | 0 |
| ky | source/utils/normalize.ts | normalizeRetryOptions | source/core/Ky.ts, test/normalize.ts | source/core/Ky.ts, test/normalize.ts | 2 | 0 | 0 |
| ky | source/core/constants.ts | supportsAbortSignal | source/core/Ky.ts, source/utils/merge.ts | source/core/Ky.ts, source/utils/merge.ts | 2 | 0 | 0 |
| ky | source/utils/merge.ts | validateAndMerge | source/index.ts, test/merge.ts, test/retry.ts | source/index.ts, test/merge.ts, test/retry.ts | 3 | 0 | 0 |
| ky | source/types/options.ts | NormalizedOptions | source/core/Ky.ts, source/errors/HTTPError.ts, source/types/hooks.ts, test/hooks.ts | source/core/Ky.ts, source/errors/HTTPError.ts, source/types/hooks.ts, test/hooks.ts | 4 | 0 | 0 |
| ky | source/errors/KyError.ts | KyError | source/errors/ForceRetryError.ts, source/errors/HTTPError.ts, source/errors/NetworkError.ts, source/errors/ResponseSizeError.ts, source/errors/TimeoutError.ts, source/utils/type-guards.ts | source/errors/ForceRetryError.ts, source/errors/HTTPError.ts, source/errors/NetworkError.ts, source/errors/ResponseSizeError.ts, source/errors/TimeoutError.ts, source/utils/type-guards.ts, test/errors.ts, test/hooks.ts, test/main.ts, test/response-size.ts | 6 | 0 | 4 |
| ky | source/utils/type-guards.ts | isHTTPError | source/core/Ky.ts | source/core/Ky.ts, test/context.ts, test/errors.ts, test/hooks.ts, test/http-error.ts, test/type-guards.ts | 1 | 0 | 5 |

| repo | tp | fp | fn | precision | recall |
|---|---|---|---|---|---|
| ky (9 samples) | 24 | 0 | 9 | 1.000 | 0.727 |

### Reading the numbers

Precision is 1.000 over 24 predicted edges: every file the tool named does use
that name. The re-export is not one of them either: `source/index.ts` is linked
to `source/errors/KyError.ts` by the index and is correctly left out of the six,
because `export {KyError} from …` is a binding. No sampled name is declared
twice in ky, so this sample does not exercise the homonym case; the Rust run's
`DirEntry` and the fixture in `refs_test.rs` do.

Recall is 0.727, and **all 9 misses are upstream of the name scan, in the import
edges** the index already holds: `impact` looks one hop from the changed file, so
a reference that reaches the declaration through the `source/index.ts` barrel is
invisible. `source/index.ts` is imported by 38 files; the two shapes in the
sample are

1. a test importing the barrel relatively (4 of 9): `test/errors.ts:16` imports
   `../source/index.js`, so the file is an importer of `source/index.ts`, not of
   `source/errors/KyError.ts`;
2. the same for a type guard (5 of 9): `test/context.ts`, `test/errors.ts`,
   `test/hooks.ts`, `test/http-error.ts` and `test/type-guards.ts` reach
   `isHTTPError` through the same barrel.

No miss is a use the identifier scan failed to see: every file the index links to
the changed file and that uses the name was reported. A stricter truth rule
("imports the declaring module, not a barrel") would report recall 1.000 for this
sample; the wider rule is the one a person applying the change would use, which
is why the misses are counted.

### Limitations observed

- **One hop, so barrels are the dominant limitation** (the 9 misses above; it
  is the same shape as the Rust run's misses where the use reaches the
  declaration through another file — the `ignore` crate root for
  `crates/core/haystack.rs` — instead of through an import of the declaring
  file). Ky's public surface is a barrel imported by 38 files, so a public
  symbol loses every reference that goes through it, which is most of that
  symbol's test references. A test that imports the declaring file directly is
  found: `test/retry-timing.ts`, `test/normalize.ts`, `test/merge.ts` and
  `test/retry.ts` are all true positives above.
- **Documentation is read as code when extracting imports and declarations.**
  `cogito related source/utils/merge.ts` lists `ky` among its imports, which comes
  from the `import ky, {replaceOption} from 'ky'` line inside the JSDoc of
  `replaceOption`, and `cogito outline source/utils/merge.ts` lists `base`
  (42-44) and `extended` (47-51) as declarations, both inside that same JSDoc
  example. Neither changed a label here, but a JSDoc example inflates a file's
  declaration count, which is the side `uses_name` subtracts.
- **An import of the package's own name does not resolve.** Ky's
  `package.json` points at `distribution/index.js`, which is not checked out, so
  the index holds the `ky` spec unresolved. No real file in ky imports its own
  package (only doc examples do), so the sample does not exercise what would be
  missed if one did.
- The sample has no `.tsx`, no `import * as ns` and no inline qualified path,
  so it says nothing about those; it does say that `import type` resolves, since
  `NormalizedOptions` is reached through one.

### Threats to validity

- The labels are an agent's, not a person's; the rule above is stated so it can
  be challenged, and it is what decides the 9 misses.
- 9 samples is small, and each sample measures one line, one symbol. Read the
  1.000 as "no false positive was observed in 9 samples", not as an estimate for
  TypeScript.
- Every miss in this sample has the same cause, so the sample says nothing about
  how the name scan behaves where the import edge exists and the use is unusual
  (a namespace import, a computed member, an inline qualified path).

### Reproduce (TypeScript run)

```bash
cargo build -p cogito-harness            # $cogito = target/debug/cogito
git clone https://github.com/sindresorhus/ky.git ~/.cache/cogito/ts-bench/ky
cd ~/.cache/cogito/ts-bench/ky && git checkout -q 3541888878c4d303a8bc2ad61815f4e048f55a5d

# One sample: append the marker to the line named in the table, ask for the
# impact, then restore the line.
sed -i '5s/$/ \/\/ refs-measure/' source/utils/is.ts
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"refs-measure","version":"1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"impact\",\"arguments\":{\"path\":\"$PWD\"}}}" \
  | "$cogito" mcp 2>/dev/null \
  | python3 -c 'import json,sys
for line in sys.stdin:
    if line.strip() and json.loads(line).get("id") == 2:
        print(json.loads(line)["result"]["content"][0]["text"])'
git restore source/utils/is.ts
```

The line each of the other samples marks is `mergeHooks` (merge.ts:220),
`getRetryTimingHeader` (retry-timing.ts:26), `normalizeRetryOptions`
(normalize.ts:38), `supportsAbortSignal` (constants.ts:34), `validateAndMerge`
(merge.ts:64), `NormalizedOptions` (options.ts:541), `KyError` (KyError.ts:9) and
`isHTTPError` (type-guards.ts:60). The driver that looped over them was an
ad-hoc script calling this same MCP tool; folding the TypeScript samples into
`refs_measure_test.rs` (or reading `benchmarks/repos-ts.json` there) is the
follow-up that would make this run part of the checked-in instrument.

Nothing reads `benchmarks/repos-ts.json` yet: `cogito bench` loads only
`benchmarks/repos.json` (`crates/cogito-harness/src/commands/bench.rs:225`). The
entry is the pinned record of the repository this measurement used.

## Reproduce (Rust)

```bash
cp -a <bench-cache>/bat .cogito/refs-eval/bat
cp -a <bench-cache>/ripgrep .cogito/refs-eval/ripgrep
COGITO_REFS_MEASURE=1 \
COGITO_REFS_ROOT="$PWD/.cogito/refs-eval" \
COGITO_REFS_STATE="$PWD/.cogito/refs-eval/state" \
  cargo test -p cogito-harness-core --test refs_measure_test -- --ignored --nocapture
```

## Threats to validity (Rust)

- The labels are an agent's, not a person's; the rule above is stated so it can
  be challenged.
- 8 samples is small, and each sample measures one line, one symbol. Treat the
  precision figure as "no false positive was observed in 8 samples", not as a
  general estimate.
- The truth rule counts same-crate and cross-crate references alike, which is
  what makes the cross-crate miss visible; a stricter file-level rule would
  raise the reported recall without changing the tool.
