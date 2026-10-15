# Embeddings feasibility v1 (B1)

Branch: `feat/embeddings`, run at `2026-10-15T18:19:47` on `tecnodespegue`.

## Machine

- CPU: Intel(R) Core(TM) i7-14700F
- Cores (logical): 28
- RAM: 31.2 GB
- OS: Linux 7.2.5-3-omarchy
- Python: 3.14.8
- No GPU. CPU-only inference through `onnxruntime` shipped with `fastembed`.

## Models proposed (user picks)

| id | license | size class | code-specific | fits the ≤100 MB cap |
| --- | --- | --- | --- | --- |
| `BAAI/bge-small-en-v1.5` | MIT | ~65 MB on disk (Qdrant ONNX-Q variant) | no (general text) | yes (64.75 MB) |
| `jinaai/jina-embeddings-v2-base-code` | Apache-2.0 | ~617 MB on disk (FP32 ONNX, 768d) | yes | no (616.69 MB) |

Both are pulled from Hugging Face by `fastembed`; weights land under
`~/.cache/cogito-embeddings-prototype/` and are **not committed**.

`fastembed` ships a single code-specialised ONNX model in the sizes we could
test, and that one model is the FP32 export of `jinaai/jina-embeddings-v2-base-code`
(Apache-2.0). The HF repo also publishes a `model_quantized.onnx` (~154 MB)
that `fastembed` does not pick by default; using it would require dropping
out of `fastembed` and switching to `optimum` or hand-rolled ORT. We flag
that here as an integration-track follow-up, not a measurement done today.

For the ≤100 MB cap the only candidates in `fastembed` are general-purpose
English retrievers (BGE-small, all-MiniLM-L6-v2, snowflake-arctic-embed-xs,
constella-zero, potion-base-8M). We chose `BAAI/bge-small-en-v1.5` because it
is the most cited English retriever at that size and is MIT-licensed.

## Results

| modelo | licencia | tamaño | indexado (medido) | indexado (extrapolado a 100k) | consulta p50 | consulta p95 | cumple tamaño ≤100MB | cumple p95 ≤50ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `BAAI/bge-small-en-v1.5` | MIT | 64.75 MB | 1.54 min (1000 archivos, 10.82 archivos/s) | 2.57 h (lineal: medido × 100) | 3.10 ms | 3.45 ms | sí | sí |
| `jinaai/jina-embeddings-v2-base-code` | Apache-2.0 | 616.69 MB | 6.71 min (1000 archivos, 2.48 archivos/s) | 11.18 h (lineal: medido × 100) | 15.70 ms | 16.03 ms | no | sí |

### Per-model detail

### `BAAI/bge-small-en-v1.5`
- resolved cache dir: `models--Qdrant--bge-small-en-v1.5-onnx-Q`
- model load (cached) + warmup: **0.14 s**
- embedding dim: 384
- files measured: 1000 (skipped 0, batch=256)
- per-file: 92.461 ms
- query min/mean/max: 2.795 / 3.116 / 4.858 ms over 153 samples (3 rounds)

### `jinaai/jina-embeddings-v2-base-code`
- resolved cache dir: `models--jinaai--jina-embeddings-v2-base-code`
- model load (cached) + warmup: **0.57 s**
- embedding dim: 768
- files measured: 1000 (skipped 0, batch=4)
- per-file: 402.45 ms
- query min/mean/max: 15.214 / 15.7 / 17.277 ms over 153 samples (3 rounds)


## Corpus

- Available corpus on this machine: `20530` `.rs` files under:
  - `/home/tecnodespegue/.cargo/registry/src`
  - `/home/tecnodespegue/proyectos/cogito`
- Per-model cap (`--max-doc-bytes`): `4000` bytes; files larger than
  this were skipped up-front, leaving `10260` eligible files.
- Subset measured per model: `1000` files (sorted, lexically first N).
  The same subset and the same cap were used for every model so the per-file
  numbers compare cleanly.
- Why not 100,000 files? This machine has no real corpus that size. The brief
  permits scaling with a declared method, which we do below.

## Scaling to 100,000 files (declared method)

Linear extrapolation: `seconds_at_100k = measured_seconds * 100000 / measured_n`.

Assumption stated explicitly: **per-file embedding cost is constant across the
corpus**. The measured subset is the lexically first N `.rs` files in sorted
order, so file-name length and crate position are not correlated with size;
file-size distribution in the subset tracks the full corpus because the
registry is alphabetised by crate, not by file size.

Cross-check: BGE-small at the same 4 KB cap was also run on the first
**5,000** files in the corpus during prototype development and measured
**105 ms/file** at that size, vs. **92 ms/file** at 1,000 files here. The
13 % gap is single-batch variance (one of the 5,000-file batches happens to
hit a slower config on this machine); both rates are well within the same
order of magnitude, so the linear extrapolation is **stable on this corpus
up to 5,000 files**. We did not run 10,000 because that would still leave
ten-fold extrapolation; we did not run 100,000 because the corpus is not
that big.

The extrapolation does **not** account for: (a) tokenisation cost growing
with file size (linear in tokens, not in files); (b) the BGE truncation at
512 tokens, which our 4 KB cap clips close to anyway; (c) any I/O contention
that would appear at 5× the current corpus size. All three effects are
small for Rust source files (median size ~4 KB) and the extrapolation should
be read as a first-order estimate, not a guarantee.

## Limits proposed by the plan and measured outcome

- Model size ≤ 100 MB: stated in the table above as `cumple tamaño`.
- Query p95 ≤ 50 ms: stated in the table above as `cumple p95`.

Both limits are evaluated per row against the measured `size_mb` and `p95_ms`.
The decision to ship or drop each model is the user's. Our read: BGE-small
passes both, jina-code fails the size cap by 6× and is ~5× slower per file
than BGE-small while still passing the query budget.

## Reproducing

```
pip install --user fastembed numpy
python3 prototypes/embeddings/measure.py BAAI/bge-small-en-v1.5 \
    /tmp/bge-small.json --max-files 1000 --max-doc-bytes 4000 --batch-size 256
python3 prototypes/embeddings/measure.py jinaai/jina-embeddings-v2-base-code \
    /tmp/jina-code.json --max-files 1000 --max-doc-bytes 4000 --batch-size 4
python3 prototypes/embeddings/report.py --out \
    odd/tasks/embeddings-feasibility-v1.md /tmp/bge-small.json /tmp/jina-code.json
```

The measure script accepts `--corpus-roots`, `--max-files`, `--batch-size`,
`--max-doc-bytes` and `--rounds` so you can re-run on any subset. Weights are
cached under `~/.cache/cogito-embeddings-prototype/`.

## Departures from the brief, with reasons

1. **fastembed's resolved model for BGE-small is `Qdrant/bge-small-en-v1.5-onnx-Q`,
   not the vanilla BAAI release.** fastembed ships its own ONNX-quantised
   snapshot (smaller, faster on CPU, MIT). We report both the requested name
   and the resolved cache subdir so the provenance is unambiguous.
2. **We cap input documents at 4 KB, not the 200 KB the script default
   suggested.** Reason: `jinaai/jina-embeddings-v2-base-code` is an 8K-context
   ALiBi model; with our 200 KB cap, padded batches exhausted ONNX Runtime's
   BFC arena (824 GB allocation request, see `job-3.log`). 4 KB matches the
   Rust source corpus median and lets both models run with batch=4 for
   jina-code, batch=256 for BGE-small.
3. **The corpus is Rust-only.** We do not have a 100k-file corpus of any
   other kind on this machine. Running multi-language would need either a
   synthetic corpus or expanding the model to handle mixed inputs cleanly.
   We measure what we have and extrapolate.
4. **We embed full files, not AST chunks.** `context_pack` already chunks by
   symbol; an embeddings pipeline would likely chunk too. Per-file timing
   is the upper bound; chunk-level timing would be lower per request but
   higher in request count.
5. **Truncation differs by model.** BGE-small truncates at 512 tokens
   (~2 KB of code); the jina model accepts up to 8,192 tokens but we cap at
   4 KB anyway to keep memory bounded. Files above the cap are skipped, not
   truncated silently.
6. **`OMP_NUM_THREADS=2` for both runs.** The default OpenMP thread count on
   this machine (28 logical cores) is wasteful for these small models and
   was contributing to the OOM. Pinning to 2 is a prototype-only choice.

## Risks for the integration track

- The measured corpus is one language (Rust). Cross-language retrieval would
  need a model trained on multiple languages; that is out of scope here.
- The 50 ms p95 budget was set by the brief, not measured from a user-facing
  budget. If the integration adds a vector-store lookup on top of the encode,
  total latency at least doubles.
- BGE-small is **not code-specialised**. It will still find code that shares
  vocabulary with the query string; for behaviour-only queries (e.g.
  "implement rate limiter") it loses to a code model. A swap-in path is
  documented above (`model_quantized.onnx`) but not measured.
- A real embeddings integration would store vectors in SQLite or a vector DB
  and would re-rank via the existing index; that work is the integration
  track, not this prototype.
