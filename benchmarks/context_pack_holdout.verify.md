# Verification of `benchmarks/context_pack_holdout.json`

Every expected path and symbol in the holdout set is checked against the pinned
commit of its repository, **not** against a working tree, so a task cannot be
satisfied by a file that only exists on some newer checkout.

Repositories and commits (same pins as `benchmarks/context_pack_gold.json`,
copied from `benchmarks/repos.json`):

| repo | commit |
| --- | --- |
| ripgrep | `3fce3b5bb0236da2df6d99672afb8a719642eca7` |
| bat | `d9559c69f541873f8c47ac8ad81d985f4441edf6` |

## How the checkouts are obtained

No clone and no download of its own: the check reuses the cache that
`cogito bench` already fills. `ensure_clone` in
`crates/cogito-harness/src/commands/bench.rs` clones each manifest entry into
`.cogito/bench-cache/<name>` and checks out the pinned commit, so an existing
cache already has the pinned objects; it was copied into this worktree's
`.cogito/bench-cache` instead of being fetched again (about 25 MB, and
`.cogito/` is gitignored). Both checkouts were confirmed to be at the pinned
commits before this check:

```sh
git -C .cogito/bench-cache/ripgrep rev-parse HEAD   # 3fce3b5bb0236da2df6d99672afb8a719642eca7
git -C .cogito/bench-cache/bat rev-parse HEAD      # d9559c69f541873f8c47ac8ad81d985f4441edf6
```

## Reproducible check

`benchmarks/context_pack_holdout.verify.sh` reads the JSON and re-runs the check
below for every entry. It exits non-zero if a path is missing at the pinned
commit or a symbol is not declared in one of that task's expected files.

```sh
# from the repository root; the cache is found at .cogito/bench-cache by default
benchmarks/context_pack_holdout.verify.sh

# or point it at an existing cache (a sibling worktree's cache also works)
COGITO_BENCH_CACHE=/path/to/.cogito/bench-cache benchmarks/context_pack_holdout.verify.sh
```

## The command used for each entry

The two commands, with `<cache>` standing for the bench cache root
(`.cogito/bench-cache`), `<repo>` for `ripgrep` or `bat`, and `<commit>` for
that repository's pinned commit:

- **path**: `git -C <cache>/<repo> cat-file -e <commit>:<path>` — succeeds only
  if the blob exists in that commit.
- **symbol**: `git -C <cache>/<repo> grep -nE '(pub(\([^)]*\))? )?(struct|enum|trait|fn|type|const|static) <symbol>\b' <commit> -- <file>` —
  must print the declaration inside the expected file.

### ripgrep `3fce3b5bb0236da2df6d99672afb8a719642eca7`

| task | kind | value | command | observed |
| --- | --- | --- | --- | --- |
| rg-h01 | path | `crates/printer/src/standard.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/printer/src/standard.rs` | exists |
| rg-h01 | symbol | `StandardBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) StandardBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/printer/src/standard.rs` | `crates/printer/src/standard.rs:99:pub struct StandardBuilder {` |
| rg-h02 | path | `crates/printer/src/json.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/printer/src/json.rs` | exists |
| rg-h02 | symbol | `JSONBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) JSONBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/printer/src/json.rs` | `crates/printer/src/json.rs:54:pub struct JSONBuilder {` |
| rg-h03 | path | `crates/ignore/src/overrides.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/ignore/src/overrides.rs` | exists |
| rg-h03 | symbol | `Override` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) Override\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/ignore/src/overrides.rs` | `crates/ignore/src/overrides.rs:47:pub struct Override(Gitignore);` |
| rg-h03 | symbol | `OverrideBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) OverrideBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/ignore/src/overrides.rs` | `crates/ignore/src/overrides.rs:115:pub struct OverrideBuilder {` |
| rg-h04 | path | `crates/globset/src/lib.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/globset/src/lib.rs` | exists |
| rg-h04 | symbol | `GlobSetBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) GlobSetBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/globset/src/lib.rs` | `crates/globset/src/lib.rs:566:pub struct GlobSetBuilder {` |
| rg-h04 | symbol | `GlobSet` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) GlobSet\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/globset/src/lib.rs` | `crates/globset/src/lib.rs:309:pub struct GlobSet {` |
| rg-h05 | path | `crates/ignore/src/types.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/ignore/src/types.rs` | exists |
| rg-h05 | symbol | `Types` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) Types\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/ignore/src/types.rs` | `crates/ignore/src/types.rs:165:pub struct Types {` |
| rg-h05 | symbol | `TypesBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) TypesBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/ignore/src/types.rs` | `crates/ignore/src/types.rs:307:pub struct TypesBuilder {` |
| rg-h06 | path | `crates/cli/src/decompress.rs` | `git -C <cache>/ripgrep cat-file -e 3fce3b5bb0236da2df6d99672afb8a719642eca7:crates/cli/src/decompress.rs` | exists |
| rg-h06 | symbol | `DecompressionMatcherBuilder` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) DecompressionMatcherBuilder\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/cli/src/decompress.rs` | `crates/cli/src/decompress.rs:15:pub struct DecompressionMatcherBuilder {` |
| rg-h06 | symbol | `DecompressionMatcher` | `git -C <cache>/ripgrep grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) DecompressionMatcher\b' 3fce3b5bb0236da2df6d99672afb8a719642eca7 -- crates/cli/src/decompress.rs` | `crates/cli/src/decompress.rs:147:pub struct DecompressionMatcher {` |

### bat `d9559c69f541873f8c47ac8ad81d985f4441edf6`

| task | kind | value | command | observed |
| --- | --- | --- | --- | --- |
| bat-h01 | path | `src/style.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/style.rs` | exists |
| bat-h01 | symbol | `StyleComponent` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) StyleComponent\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/style.rs` | `src/style.rs:8:pub enum StyleComponent {` |
| bat-h02 | path | `src/output.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/output.rs` | exists |
| bat-h02 | symbol | `OutputType` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) OutputType\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/output.rs` | `src/output.rs:66:pub enum OutputType {` |
| bat-h03 | path | `src/nonprintable_notation.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/nonprintable_notation.rs` | exists |
| bat-h03 | symbol | `NonprintableNotation` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) NonprintableNotation\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/nonprintable_notation.rs` | `src/nonprintable_notation.rs:5:pub enum NonprintableNotation {` |
| bat-h04 | path | `src/pager.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/pager.rs` | exists |
| bat-h04 | symbol | `Pager` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) Pager\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/pager.rs` | `src/pager.rs:74:pub(crate) struct Pager {` |
| bat-h05 | path | `src/decorations.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/decorations.rs` | exists |
| bat-h05 | symbol | `LineNumberDecoration` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) LineNumberDecoration\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/decorations.rs` | `src/decorations.rs:22:pub(crate) struct LineNumberDecoration {` |
| bat-h05 | symbol | `LineChangesDecoration` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) LineChangesDecoration\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/decorations.rs` | `src/decorations.rs:81:pub(crate) struct LineChangesDecoration {` |
| bat-h06 | path | `src/diff.rs` | `git -C <cache>/bat cat-file -e d9559c69f541873f8c47ac8ad81d985f4441edf6:src/diff.rs` | exists |
| bat-h06 | symbol | `get_git_diff` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) get_git_diff\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/diff.rs` | `src/diff.rs:43:pub fn get_git_diff(filename: &Path) -> Option<LineChanges> {` |
| bat-h06 | symbol | `LineChange` | `git -C <cache>/bat grep -nE '(pub(\([^)]*\))? )?(struct\|enum\|trait\|fn\|type\|const\|static) LineChange\b' d9559c69f541873f8c47ac8ad81d985f4441edf6 -- src/diff.rs` | `src/diff.rs:11:pub enum LineChange {` |

## Output of the check

Captured from `COGITO_BENCH_CACHE=.cogito/bench-cache benchmarks/context_pack_holdout.verify.sh`,
exit code `0`:

```text
# ripgrep @ 3fce3b5bb0236da2df6d99672afb8a719642eca7
# cache: .cogito/bench-cache/ripgrep
ok    rg-h01   path   crates/printer/src/standard.rs
ok    rg-h01   symbol StandardBuilder
ok    rg-h02   path   crates/printer/src/json.rs
ok    rg-h02   symbol JSONBuilder
ok    rg-h03   path   crates/ignore/src/overrides.rs
ok    rg-h03   symbol Override
ok    rg-h03   symbol OverrideBuilder
ok    rg-h04   path   crates/globset/src/lib.rs
ok    rg-h04   symbol GlobSetBuilder
ok    rg-h04   symbol GlobSet
ok    rg-h05   path   crates/ignore/src/types.rs
ok    rg-h05   symbol Types
ok    rg-h05   symbol TypesBuilder
ok    rg-h06   path   crates/cli/src/decompress.rs
ok    rg-h06   symbol DecompressionMatcherBuilder
ok    rg-h06   symbol DecompressionMatcher

# bat @ d9559c69f541873f8c47ac8ad81d985f4441edf6
# cache: .cogito/bench-cache/bat
ok    bat-h01  path   src/style.rs
ok    bat-h01  symbol StyleComponent
ok    bat-h02  path   src/output.rs
ok    bat-h02  symbol OutputType
ok    bat-h03  path   src/nonprintable_notation.rs
ok    bat-h03  symbol NonprintableNotation
ok    bat-h04  path   src/pager.rs
ok    bat-h04  symbol Pager
ok    bat-h05  path   src/decorations.rs
ok    bat-h05  symbol LineNumberDecoration
ok    bat-h05  symbol LineChangesDecoration
ok    bat-h06  path   src/diff.rs
ok    bat-h06  symbol get_git_diff
ok    bat-h06  symbol LineChange

12 expected paths and 18 expected symbols checked
all verified
```

## Composition of the set

- 12 tasks: 6 over ripgrep, 6 over bat.
- `names_the_identifier`: 6 true (`rg-h01`, `rg-h02`, `rg-h03`, `bat-h01`, `bat-h02`, `bat-h03`), 6 false.
- `language`: 8 `en`, 4 `es` (`rg-h02`, `rg-h05`, `bat-h02`, `bat-h05`).
- No expected file appears in `benchmarks/context_pack_gold.json`.
