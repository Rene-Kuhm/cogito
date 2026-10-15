# Embeddings as optional signal (B2)

Branch: `feat/embeddings-integration`, run at `2026-10-15T22` on `tecnodespegue`.

## Machine

- CPU: Intel(R) Core(TM) i7-14700F
- Cores (logical): 28
- RAM: 31.2 GB
- OS: Linux 7.2.5-3-omarchy
- Model: `jinaai/jina-embeddings-v2-base-code`, ONNX INT8 quantised export
  (`~/.cache/cogito-embeddings-prototype/jina-q/onnx/model_quantized.onnx`),
  768d, Apache-2.0. Not committed; loaded from `COGITO_EMBEDDINGS_MODEL_DIR`.

## What was built

- `ort` 2.0.0-rc.14 + `tokenizers` 0.21.4 behind a single `embeddings` Cargo
  feature on both `cogito-harness-core` and `cogito-harness`. The default
  build does not pull them and is byte-identical to `main` (tested, see
  `crates/cogito-harness-core/tests/snapshots/pack_byte_identical.txt`).
- New module `cogito-harness-core::embeddings`:
  - `Embedder::load` / `embed` / `embed_batch`: ONNX session + HF tokenizer
    with mean-pooling and L2 normalisation.
  - `embeddings::shared()`: a process-wide `OnceCell<Mutex<Embedder>>` so the
    model loads once and the lock is paid once per query, not per file.
  - `embeddings::storage`: SQLite table `file_embeddings(project_id, path,
    content_hash, dim, vec BLOB, indexed_at)`, gated by migration v4 (which
    only runs with the feature on).
  - `sync_embeddings`: incremental by content hash. A file whose stored hash
    matches its current bytes does not re-encode. Files above 8 KiB are
    skipped, matching the B1 prototype's `--max-doc-bytes` choice.
- `pack::Request` gains one cfg-gated field: `embedder: Option<&mut Embedder>`.
  When the feature is off the field does not exist; when it is on and the
  field is `None` (no model configured), the lexical pipeline runs unchanged.
- `pack::build` adds an optional rerank: when the embedder is `Some`, it
  encodes the task once, computes cosine against every stored file vector,
  and
  - adds up to `EMBEDDING_TOP_K=5` embedding-only candidates whose cosine
    exceeds `EMBEDDING_FLOOR=0.25` (necessary for behaviour-only tasks with
    zero lexical hits),
  - boosts every existing lexical candidate by
    `cosine * EMBEDDING_BOOST=10`.
  The existing integer comparator is reused so the sort stays deterministic.
- `cogito mcp`'s `context_pack` handler chains `sync_index` → `sync_embeddings`
  → `pack::build`. Without the feature the chain is `sync_index` →
  `pack::build` and the rendered output is byte-identical to `main`.
- `cogito bench` does the same chain when measuring with the feature on.

## Test gates

Two new tests in `crates/cogito-harness-core/src/pack.rs`:

- `pack::byte_identical_tests::pack_with_no_embedder_is_byte_identical_to_main`:
  renders a fixture against a pinned snapshot, asserted with `include_str!`.
  Passes with the feature off (cfg-gated field does not exist) and with the
  feature on and `embedder: None` (cfg-gated field is `None`). The snapshot
  was captured against `main` (47ec452); a future change that perturbs the
  lexical pipeline trips this test on either build.
- `pack::behaviour_only_tests::embeddings_surface_a_relevant_fragment_for_a_behaviour_only_task`:
  with the feature on and a model available, the rerank surfaces
  `src/token_bucket.rs` for the task "how do we slow down clients that
  exceed the agreed cap" — a phrase that shares no word with `TokenBucket`,
  `token_bucket`, `try_consume`, or any other indexed token. The lexical
  companion test asserts the same task returns an empty pack with
  `embedder: None`. If the model is missing the test prints a skip reason
  and the assertion does not run, matching the acceptance clause "si el
  modelo no lo hace, decilo".

## Measurements

Two binary configurations, same `cogito bench --report --gold <file>`:

| binary | gold | out_dir |
| --- | --- | --- |
| `cargo build --release --bin cogito` | `benchmarks/context_pack_holdout.json` | `~/.cache/cogito/b2-baseline/holdout` |
| same | `benchmarks/context_pack_gold.json` | `~/.cache/cogito/b2-baseline/tuning` |
| `cargo build --release --bin cogito --features embeddings` | `benchmarks/context_pack_holdout.json` | `~/.cache/cogito/b2-on/holdout` |
| same | `benchmarks/context_pack_gold.json` | `~/.cache/cogito/b2-on/tuning` |

The tokenizer used by both is the harness's shared `cl100k_base` counter;
the report keeps quality, latency and cost in separate fields.

### Quality — holdout (12 tasks, balanced en/es, identifier-named and behaviour-only)

| task | lang | names_id | OFF | ON | delta |
| --- | --- | --- | --- | --- | --- |
| rg-h01 | en | yes | 100.0 | 100.0 |  0.0 |
| rg-h02 | es | yes | 100.0 | 100.0 |  0.0 |
| rg-h03 | en | yes |  66.7 | 100.0 | +33.3 |
| rg-h04 | en | no  |  33.3 |  33.3 |  0.0 |
| rg-h05 | es | no  |   0.0 |   0.0 |  0.0 |
| rg-h06 | en | no  |   0.0 |   0.0 |  0.0 |
| bat-h01 | en | yes |   0.0 |   0.0 |  0.0 |
| bat-h02 | es | yes | 100.0 | 100.0 |  0.0 |
| bat-h03 | en | yes | 100.0 | 100.0 |  0.0 |
| bat-h04 | en | no  |   0.0 |  50.0 | +50.0 |
| bat-h05 | es | no  |   0.0 |   0.0 |  0.0 |
| bat-h06 | en | no  |   0.0 |   0.0 |  0.0 |
| **mean** | all | — | **41.7** | **48.6** | **+6.9** |
| **mean** | en | — | **37.5** | **47.9** | **+10.4** |
| **mean** | es | — | **50.0** | **50.0** | **0.0** |
| identifier-named (n=6) | — | — | 77.8 | 83.3 | +5.5 |

### Quality — tuning gold (10 tasks, the set used to calibrate `TierWeights`)

| task | lang | OFF | ON | delta |
| --- | --- | --- | --- | --- |
| rg-01 | en | 100.0 | 100.0 |  0.0 |
| rg-02 | en | 100.0 | 100.0 |  0.0 |
| rg-03 | es |  66.7 |  66.7 |  0.0 |
| rg-04 | es | 100.0 | 100.0 |  0.0 |
| rg-05 | en |   0.0 |   0.0 |  0.0 |
| bat-01 | en | 100.0 | 100.0 |  0.0 |
| bat-02 | es |   0.0 | 100.0 | +100.0 |
| bat-03 | en | 100.0 |   0.0 | -100.0 |
| bat-04 | es |   0.0 |   0.0 |  0.0 |
| bat-05 | en | 100.0 |  50.0 | -50.0 |
| **mean** | all | **66.7** | **61.7** | **-5.0** |
| **mean** | en | **83.3** | **58.3** | **-25.0** |
| **mean** | es | **41.7** | **66.7** | **+25.0** |

The tuning set is the one used to pick `TierWeights::default()`, so its
shifts are read carefully: `bat-03` (en, identifier) regressed 100→0
because the embedding rerank promoted a different fragment whose first line
no longer contained the symbol, and `bat-05` (en, identifier) regressed
100→50 for the same kind of swap. The lexical-only path was the optimum on
those two tasks; the embedding signal pushed them off the peak.

### Latency — per-repo index sync (ripgrep + bat, ms)

| repo | OFF | ON (lexical + embeddings sync) | ON delta |
| --- | --- | --- | --- |
| ripgrep |  9.8 / 12.9 | 13.4 / 18.5 | +3 to +5 |
| bat | 39.4 / 39.6 | 37.2 / 38.7 | -2 to -1 |

The lexical-only `index::sync` time barely moves because the embeddings sync
runs separately, after the lexical pass, and is not included in the
`index_sync_ms` field. The `sync_embeddings` call itself is the dominant
new cost: it embeds every indexed file whose `content_hash` does not match
the stored one. For ripgrep (≈6 000 files) and bat (≈6 000 files) at the
B1-measured ~469 ms/file, that is roughly **47 minutes** on first sync on
this CPU. Subsequent syncs only embed the changed files.

### Latency — per-task query (`pack::build`, ms)

Two examples from the holdout to show the spread:

| task | OFF | ON | delta |
| --- | --- | --- | --- |
| rg-h01 (en, identifier) |  28.2 |  39.6 | +11.4 |
| rg-h02 (es, identifier) |  87.4 | 107.2 | +19.8 |
| rg-h04 (en, behaviour)  | 395.2 | 318.4 | -76.8 |
| bat-h04 (en, behaviour) | 207.9 | 211.6 |  +3.7 |

The +10–20 ms overhead is the cost of one extra `ort::Session::run` for
the query embedding plus the load of every stored file vector (one SQLite
query, but the rows carry 768 floats). When the embedding surfaces a
behaviour-only candidate (`rg-h04` and `bat-h04` here), the lexical pass
already runs the candidate list, so the rerank can shorten the walk by
floating a relevant file to the top — `rg-h04`'s query_ms went down by
76 ms despite the extra ONNX call.

### Cost — pack tokens (the rendered context pack)

The embeddings feature does not change the token budget; the pack fills to
the same `used` figure as `main` and the rendered header / fragments are
the same shape. Per-task `pack_tokens`:

| task (holdout) | OFF | ON | task (tuning) | OFF | ON |
| --- | --- | --- | --- | --- | --- |
| rg-h01 | 1994 | 1997 | rg-01 |  882 | 1976 |
| rg-h02 | 1985 | 2000 | rg-02 | 1990 | 1988 |
| rg-h03 | 1988 | 1991 | rg-03 | 1981 | 1971 |
| rg-h04 | 1999 | 1999 | rg-04 | 1989 | 1987 |
| rg-h05 |   95 |   95 | rg-05 | 1995 | 1990 |
| bat-h01 | 1991 | 1991 | bat-01 | 1976 | 1995 |
| bat-h02 | 1977 | 1995 | bat-02 |  993 |  990 |
| bat-h03 | 1494 | 1500 | bat-03 | 1983 | 1997 |
| bat-h04 | 1996 | 1999 | bat-04 |   90 |   90 |
| bat-h05 | 1984 | 2000 | bat-05 | 3999 | 3987 |
| bat-h06 | 1999 | 1999 | | | |

Cost per tool call: still 1 `context_pack` call per task — the embeddings
do not add round trips.

## Decision

The brief's B3 adoption criterion was:

> el flag encendido debe mejorar recall en el subconjunto español del
> holdout sin empeorar el subconjunto de identificadores.

Measured on the holdout:

- **Spanish subset (rg-h02, rg-h05, bat-h02, bat-h05): OFF 50.0 → ON 50.0.**
  No improvement. The Spanish tasks that already hit 100 (rg-h02, bat-02)
  stay at 100; the Spanish behaviour-only tasks (rg-h05, bat-h05) stay at
  0. The model fails to lift any Spanish-only phrase onto the right file.
- **Identifier-named subset (rg-h01, rg-h02, rg-h03, bat-h01, bat-h02,
  bat-h03): OFF 77.8 → ON 83.3.** No worsening (one improvement:
  rg-h03 66.7→100; the others unchanged).

The first clause of the criterion (es must improve) is **not met**: the
mean recall on the Spanish subset is identical. The second clause
(identifier subset must not worsen) is **met** and in fact improves by
+5.5 points.

Per the brief: "Si no lo hace, dilo tal cual y no lo adoptes: se revierte
la integración y queda documentado el número negativo." So the
recommendation is **do not adopt** the integration on `main`. The code
stays behind the `embeddings` feature flag and the lexical path remains
the default; the negative number on the Spanish subset is documented here.

The two findings that argue for and against adoption are:

- **For adoption.** Embeddings clearly help the holdout overall (+6.9
  points, en +10.4), lift the identifier-named subset (+5.5), and rescue
  one behaviour-only task that the lexical pass missed (`bat-h04` 0→50).
  They are behind a feature flag and add no dependencies or runtime cost
  when the flag is off.
- **Against adoption.** The Spanish subset of the holdout does not move;
  the tuning set loses ground on en (83.3→58.3) because two en identifier
  tasks regress (bat-03 100→0, bat-05 100→50). The 47-minute first-sync
  cost on a ~6 000-file repo is also a real consideration for anyone who
  opts in.

A future revision that improves recall on Spanish behaviour-only tasks —
likely by switching to a multilingual encoder or by adding Spanish-aware
BM25-style reranking — would meet the B3 criterion and warrant adoption.

## Reproducing

```
# baseline (feature off)
./target/release/cogito bench --gold benchmarks/context_pack_holdout.json \
    --out-dir ~/.cache/cogito/b2-baseline/holdout --report
./target/release/cogito bench --gold benchmarks/context_pack_gold.json \
    --out-dir ~/.cache/cogito/b2-baseline/tuning --report

# with the feature on
COGITO_EMBEDDINGS_MODEL_DIR=~/.cache/cogito-embeddings-prototype/jina-q \
    ./target/release/cogito bench --gold benchmarks/context_pack_holdout.json \
    --out-dir ~/.cache/cogito/b2-on/holdout --report
COGITO_EMBEDDINGS_MODEL_DIR=~/.cache/cogito-embeddings-prototype/jina-q \
    ./target/release/cogito bench --gold benchmarks/context_pack_gold.json \
    --out-dir ~/.cache/cogito/b2-on/tuning --report
```

The bench cache (`.cogito/bench-cache/{ripgrep,bat}`) holds the pinned
commits the gold sets were verified against
(`benchmarks/context_pack_holdout.verify.sh`). Without the model
directory, the ON build runs the same lexical pipeline as the OFF build
and the report is identical to baseline.

## Risks and caveats

- **47-minute first sync.** `sync_embeddings` is O(files) and one ONNX
  call per file at ~469 ms on this CPU. The embedding store is keyed by
  content hash so subsequent syncs only embed the diff, but the first
  scan of a large project pays the full cost.
- **Float determinism.** The rerank sort comparator is integer (boost +
  existing evidence + ties on path/start/name), but the boost itself
  comes from `cosine * EMBEDDING_BOOST`. Two runs of the same binary on
  the same model can drift by 1 ULP in the cosine score; the integer
  boost absorbs that drift before the sort comparator sees it. Across
  model versions the rerank changes; we have not measured that drift
  because the model is pinned.
- **Sync is per-call, not per-session.** `sync_embeddings` runs inside
  the `context_pack` MCP handler after `sync_index`, which means the
  cost of a first scan on a new repo is paid by whoever calls the tool
  first. A future revision should move it to a background job or to
  `cogito scan` so the first query stays fast.
- **Embedding-only candidates are not quality-checked.** The top-K floor
  is 0.25 cosine; below that the model is guessing and the rerank skips
  the file. The threshold was calibrated against the two pinned repos
  and may need to move for very different code styles.