# Embeddings feasibility prototype (B1)

Prototype harness for the Level-2 embeddings feasibility check on this CPU
machine. **It is not wired into the cogito workspace build.** It is a stand-alone
Python tool that lives under `prototypes/` so `cargo build --all` keeps ignoring
it.

## What it measures

1. On-disk size of a `fastembed` / ONNX model after the first download.
2. Wall-clock time to embed a Rust source corpus (every `.rs` under the chosen
   roots, up to a per-doc byte cap), and per-file rate.
3. Query latency: p50, p95, p99 over 150 single-query encode calls (3 rounds of
   the 50-query fixed set, after one warmup).

## Models used

| model | license | measured size | code-specific |
| --- | --- | --- | --- |
| `BAAI/bge-small-en-v1.5` | MIT | 64.75 MB (fastembed resolves to `Qdrant/bge-small-en-v1.5-onnx-Q`) | no (general text) |
| `jinaai/jina-embeddings-v2-base-code` | Apache-2.0 | 616.69 MB (FP32 ONNX) | yes |

Both are pulled from Hugging Face by `fastembed`. Weights land in
`~/.cache/cogito-embeddings-prototype/` and are **never committed**. There is no
code-specialised ONNX model under 100 MB in `fastembed` today; a 154 MB
quantised variant exists on HF but `fastembed` does not pick it by default.

## How to run

```bash
# One-time setup
pip install --user fastembed numpy

# Measure one model (downloads weights on first run)
python3 prototypes/embeddings/measure.py \
    BAAI/bge-small-en-v1.5 \
    /tmp/bge-small.json \
    --max-files 1000 --max-doc-bytes 4000 --batch-size 256

python3 prototypes/embeddings/measure.py \
    jinaai/jina-embeddings-v2-base-code \
    /tmp/jina-code.json \
    --max-files 1000 --max-doc-bytes 4000 --batch-size 4

# Build the markdown results table from the JSON outputs
python3 prototypes/embeddings/report.py \
    --out odd/tasks/embeddings-feasibility-v1.md \
    /tmp/bge-small.json /tmp/jina-code.json
```

The measure script accepts these flags:

| flag | default | meaning |
| --- | --- | --- |
| `--max-files` | 0 (all) | cap the corpus to the first N files after sorting |
| `--max-doc-bytes` | 200000 | skip corpus files larger than this many bytes |
| `--batch-size` | 256 | ONNX encode batch (smaller for long-context models to avoid OOM) |
| `--rounds` | 3 | query rounds (3 × 50 queries = 150 samples after warmup) |
| `--corpus-roots` | `~/.cargo/registry/src` + the cogito source tree | roots to scan for `*.rs` |

## Corpus

The largest Rust corpus on this machine is the cargo registry cache
(`~/.cargo/registry/src`) plus the cogito source tree, totalling 20,530 `.rs`
files (10,260 ≤ 4 KB after the per-doc cap). The prototype measures on a
1,000-file subset of the sorted corpus (so every model sees the same inputs)
and linearly extrapolates to 100,000 files in the report, with the assumption
stated explicitly.

## Limits checked

- model size ≤ 100 MB
- query p95 ≤ 50 ms

Each row in the results table says yes/no and cites the measured number.

## Reproducibility

The two measured JSON files (`result-bge-small.json`, `result-jina-code.json`)
live next to this README; they are the inputs that produced
`odd/tasks/embeddings-feasibility-v1.md` and let anyone re-build the table
without re-running the (slow) indexing step.