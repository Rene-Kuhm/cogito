# Reference edges v1 — precision and recall

Measured for `feat(impact): name-based reference edges for TypeScript and Rust`
(T1 of `odd/tasks/refs-graph-v1.md`). The instrument is
`crates/cogito-harness-core/tests/refs_measure_test.rs`, opt-in and `#[ignore]`d;
the numbers below are its verbatim output.

## What was measured

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

## Results

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

## Reading the numbers

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

## TypeScript: not measured

`.cogito/bench-cache/` holds only `bat` and `ripgrep`, both Rust; there is no
TypeScript repository cloned there and the task did not authorize cloning one.
**So there is no precision/recall figure for TypeScript from a holdout repo.**
What TypeScript has instead is the exact-set fixtures in
`crates/cogito-harness-core/tests/refs_test.rs` (importing users in, imported but
unused out, homonym declaration out, comment and string mentions out, self use
in). Missing to close this: a checked-out TypeScript repository with a pinned
commit, added to the holdout cache.

## Reproduce

```bash
cp -a <bench-cache>/bat .cogito/refs-eval/bat
cp -a <bench-cache>/ripgrep .cogito/refs-eval/ripgrep
COGITO_REFS_MEASURE=1 \
COGITO_REFS_ROOT="$PWD/.cogito/refs-eval" \
COGITO_REFS_STATE="$PWD/.cogito/refs-eval/state" \
  cargo test -p cogito-harness-core --test refs_measure_test -- --ignored --nocapture
```

## Threats to validity

- The labels are an agent's, not a person's; the rule above is stated so it can
  be challenged.
- 8 samples is small, and each sample measures one line, one symbol. Treat the
  precision figure as "no false positive was observed in 8 samples", not as a
  general estimate.
- The truth rule counts same-crate and cross-crate references alike, which is
  what makes the cross-crate miss visible; a stricter file-level rule would
  raise the reported recall without changing the tool.
