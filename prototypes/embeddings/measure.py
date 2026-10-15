#!/usr/bin/env python3
"""
B1 embeddings feasibility prototype.

Measures on this CPU machine:
  1. On-disk size of an embedding model (downloaded to ~/.cache/cogito-embeddings-prototype/).
  2. Wall-clock indexing time over the available Rust source corpus.
  3. Query latency (p50, p95) over at least 50 single-document encode calls.

Run as:
  python3 measure.py <model-name> <out.json> [--max-files N] [--rounds N]
                   [--batch-size N] [--max-doc-bytes N]

The script prints a one-line JSON summary to stdout for piping and writes a
full JSON to <out.json> for the report builder.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
from pathlib import Path

from fastembed import TextEmbedding

CACHE_DIR = Path.home() / ".cache" / "cogito-embeddings-prototype"
CACHE_DIR.mkdir(parents=True, exist_ok=True)

# Tell fastembed + huggingface_hub to use our cache root.
os.environ.setdefault("HF_HOME", str(CACHE_DIR))
os.environ.setdefault("HUGGINGFACE_HUB_CACHE", str(CACHE_DIR / "hub"))

# ONNX Runtime's intra-op thread pool can OOM on small machines for ALiBi-style
# long-context code models. Single-thread keeps memory bounded without making
# the numbers worse than the default for these model sizes.
os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("MKL_NUM_THREADS", "2")

# Query strings chosen to mimic code-search queries a developer would type.
# Each one is short, lowercase, no punctuation, English. Same set for every model.
QUERY_SET = [
    "parse json string into struct",
    "open file and read all lines",
    "create new sqlite database",
    "walk directory tree recursively",
    "compute sha256 of bytes",
    "spawn subprocess and capture stdout",
    "merge two sorted vectors",
    "find element by id in hashmap",
    "split string on whitespace",
    "format date as iso 8601",
    "http get request with timeout",
    "deserialize yaml to struct",
    "compress data with gzip",
    "match url against regex pattern",
    "retry failed operation three times",
    "load environment variable from file",
    "convert utf8 bytes to string",
    "implement binary tree traversal",
    "validate email address format",
    "generate uuid v4 random",
    "parse command line arguments",
    "concatenate string with separator",
    "check if path exists on disk",
    "render markdown to html",
    "compute cosine similarity between vectors",
    "sort vector by key descending",
    "serialize struct to yaml",
    "spawn tokio async task",
    "log error message with context",
    "handle websocket connection close",
    "implement lru cache eviction",
    "parse csv file into rows",
    "verify ed25519 signature",
    "find all files matching glob",
    "convert unix timestamp to datetime",
    "implement connection pool",
    "read parquet file column",
    "extract tar archive",
    "implement bloom filter",
    "spawn thread pool worker",
    "parse toml configuration file",
    "diff two strings char by char",
    "implement rate limiter token bucket",
    "build protobuf message from bytes",
    "traverse linked list in place",
    "compress string with brotli",
    "implement lock free queue",
    "send email via smtp",
    "parse avro schema",
    "implement merkle tree",
    "render graphviz dot to svg",
]


def dir_size_bytes(root: Path) -> int:
    """Sum real-file sizes under `root`, counting each inode only once.

    fastembed / huggingface_hub store weights as a content-addressed blob with
    snapshot symlinks pointing at it. Walking with os.walk + stat() double-counts
    the blob (once as the real file, once through each symlink). We dedupe by
    inode and use lstat() so symlinks contribute their own (tiny) length, not
    the target's.
    """
    seen_inodes: set[tuple[int, int]] = set()
    total = 0
    for dirpath, _, filenames in os.walk(root, followlinks=False):
        for name in filenames:
            p = Path(dirpath) / name
            try:
                st = p.lstat()
            except OSError:
                continue
            key = (st.st_dev, st.st_ino)
            if key in seen_inodes:
                continue
            seen_inodes.add(key)
            total += st.st_size
    return total


def model_size_bytes(model_dir: Path) -> int:
    """Size of a huggingface cache model dir, including its referenced blobs.

    The blob files themselves live at hub/blobs/<aa>/<hash> (shared between
    models), while the model dir only contains refs files. We follow the
    refs/main pointer to the snapshot, then walk every file (including
    symlinks) and resolve each one to its underlying blob (which may sit
    outside model_dir). Each blob is counted once via inode dedupe.
    """
    refs_main = model_dir / "refs" / "main"
    if not refs_main.exists():
        return dir_size_bytes(model_dir)
    commit = refs_main.read_text().strip()
    snap = model_dir / "snapshots" / commit
    seen: set[tuple[int, int]] = set()
    total = 0
    if snap.is_dir():
        # Walk recursively because some models (e.g. jina-embeddings-v2-base-code)
        # nest weights under onnx/model.onnx instead of flat in the snapshot root.
        for dirpath, _, filenames in os.walk(snap, followlinks=False):
            for name in filenames:
                p = Path(dirpath) / name
                try:
                    target = p.resolve()
                except OSError:
                    continue
                if not target.is_file():
                    continue
                try:
                    st = target.stat()
                except OSError:
                    continue
                key = (st.st_dev, st.st_ino)
                if key in seen:
                    continue
                seen.add(key)
                total += st.st_size
    # Also count refs files themselves (tiny, but for honesty).
    total += dir_size_bytes(model_dir)
    return total


def discover_corpus(roots: list[Path], max_bytes: int = 200_000) -> list[Path]:
    """Return all *.rs files under `roots`, sorted, skipping empty files.

    Files larger than `max_bytes` are skipped (their bytes would be truncated
    before the model sees them anyway, and skipping them up-front prevents
    OOM in batched embedding for long-context code models).
    """
    files: list[Path] = []
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*.rs"):
            if not path.is_file():
                continue
            try:
                size = path.stat().st_size
            except OSError:
                continue
            if size == 0 or size > max_bytes:
                continue
            files.append(path)
    files.sort()
    return files


def measure_size(model_name: str) -> dict:
    """Report on-disk size of the model's hub dir.

    Assumes the model has already been downloaded (caller is responsible for
    triggering the download during the load+warmup step). We locate the dir
    by name match: the requested name's last component must appear inside
    the resolved dir name (fastembed may resolve to a different repo).
    """
    hub = CACHE_DIR / "hub"
    short = model_name.split("/")[-1].lower()
    candidates = [p for p in hub.iterdir() if p.is_dir()]
    matched = [p for p in candidates if short in p.name.lower()]
    if len(matched) > 1:
        matched.sort(key=lambda p: (not p.name.endswith("-Q"), p.name))
    target = matched[0] if matched else (candidates[0] if candidates else hub)
    size_b = model_size_bytes(target)
    return {
        "model": model_name,
        "cache_subdir": str(target),
        "cache_subdir_name": target.name,
        "size_bytes": size_b,
        "size_mb": round(size_b / (1024 * 1024), 2),
    }


def measure_indexing(
    model: TextEmbedding, files: list[Path], batch_size: int
) -> dict:
    """Embed every file and measure wall time.

    Files larger than the per-doc cap (already filtered in discover_corpus) are
    skipped. The remaining corpus is fed to model.embed in chunks of batch_size
    to keep ONNX memory bounded.
    """
    docs: list[str] = [f.read_text(errors="replace") for f in files]
    skipped = 0

    t0 = time.perf_counter()
    embeddings = list(model.embed(docs, batch_size=batch_size))
    t1 = time.perf_counter()
    elapsed = t1 - t0

    dim = len(embeddings[0]) if embeddings else 0
    rate = len(docs) / elapsed if elapsed > 0 else 0.0
    return {
        "files_indexed": len(docs),
        "files_skipped": skipped,
        "dim": dim,
        "batch_size": batch_size,
        "index_seconds": round(elapsed, 3),
        "files_per_second": round(rate, 2),
        "ms_per_file": round(1000.0 * elapsed / max(len(docs), 1), 3),
    }


def measure_query_latency(model: TextEmbedding, queries: list[str], rounds: int = 3) -> dict:
    """Run each query through model.embed and record per-query latency.

    Single-query encode is what a real client pays per search query; we
    deliberately do not batch so the timing matches a real request. The model
    is assumed to already be loaded + warmed up by the caller.
    """
    samples_ms: list[float] = []
    for _ in range(rounds):
        for q in queries:
            t0 = time.perf_counter()
            _ = list(model.embed([q]))
            t1 = time.perf_counter()
            samples_ms.append((t1 - t0) * 1000.0)
    samples_ms.sort()
    n = len(samples_ms)
    p50 = samples_ms[int(0.50 * (n - 1))]
    p95 = samples_ms[int(0.95 * (n - 1))]
    p99 = samples_ms[int(0.99 * (n - 1))]
    return {
        "queries_run": n,
        "warmup_queries": 1,
        "rounds": rounds,
        "min_ms": round(samples_ms[0], 3),
        "p50_ms": round(p50, 3),
        "p95_ms": round(p95, 3),
        "p99_ms": round(p99, 3),
        "max_ms": round(samples_ms[-1], 3),
        "mean_ms": round(statistics.mean(samples_ms), 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", help="fastembed model name (e.g. BAAI/bge-small-en-v1.5)")
    parser.add_argument("out", help="output JSON path")
    parser.add_argument(
        "--corpus-roots",
        nargs="+",
        default=[
            str(Path.home() / ".cargo" / "registry" / "src"),
            "/home/tecnodespegue/proyectos/cogito",
        ],
        help="filesystem roots to scan for *.rs files",
    )
    parser.add_argument("--rounds", type=int, default=3, help="query rounds (default 3 = 150 queries)")
    parser.add_argument("--max-files", type=int, default=0, help="cap corpus (0 = all)")
    parser.add_argument("--batch-size", type=int, default=256, help="encoding batch size (default 256)")
    parser.add_argument(
        "--max-doc-bytes",
        type=int,
        default=200_000,
        help="skip corpus files larger than this many bytes (default 200000)",
    )
    args = parser.parse_args()

    roots = [Path(r) for r in args.corpus_roots]
    print(f"[measure] model={args.model}", file=sys.stderr)
    print(f"[measure] cache={CACHE_DIR}", file=sys.stderr)
    print(f"[measure] roots={roots}", file=sys.stderr)
    print(
        f"[measure] max-doc-bytes={args.max_doc_bytes} batch-size={args.batch_size} rounds={args.rounds}",
        file=sys.stderr,
    )

    # 1. Build the corpus file list (before any model load to avoid timing noise).
    all_files = discover_corpus(roots, max_bytes=args.max_doc_bytes)
    if args.max_files and len(all_files) > args.max_files:
        files = all_files[: args.max_files]
    else:
        files = all_files
    print(
        f"[measure] corpus files={len(files)} (of {len(all_files)} eligible)",
        file=sys.stderr,
    )

    # 2. Single model load + warmup (timed). This is the cold-start cost the
    #    user actually pays once per process.
    hub = CACHE_DIR / "hub"
    hub.mkdir(parents=True, exist_ok=True)
    t0 = time.perf_counter()
    model = TextEmbedding(model_name=args.model, cache_dir=str(hub))
    _ = list(model.embed(["warmup"]))
    load_seconds = time.perf_counter() - t0
    print(f"[measure] model load + warmup = {load_seconds:.2f} s", file=sys.stderr)

    # 3. Size on disk after download.
    size_info = measure_size(args.model)
    print(
        f"[measure] size={size_info['size_mb']} MB at {size_info['cache_subdir']}",
        file=sys.stderr,
    )

    # 4. Indexing (warm embed, model already loaded).
    index_info = measure_indexing(model, files, batch_size=args.batch_size)
    print(
        f"[measure] index {index_info['files_indexed']} files in {index_info['index_seconds']} s "
        f"({index_info['files_per_second']} files/s, {index_info['ms_per_file']} ms/file, "
        f"batch={index_info['batch_size']})",
        file=sys.stderr,
    )

    query_info = measure_query_latency(model, QUERY_SET, rounds=args.rounds)
    print(
        f"[measure] queries p50={query_info['p50_ms']} ms p95={query_info['p95_ms']} ms "
        f"p99={query_info['p99_ms']} ms over {query_info['queries_run']} samples",
        file=sys.stderr,
    )

    out = {
        "model": args.model,
        "size": size_info,
        "load_seconds": round(load_seconds, 3),
        "index": index_info,
        "query": query_info,
        "corpus_roots": [str(r) for r in roots],
        "corpus_file_count_discovered": len(discover_corpus(roots, max_bytes=args.max_doc_bytes)),
        "max_doc_bytes": args.max_doc_bytes,
        "batch_size": args.batch_size,
    }
    Path(args.out).write_text(json.dumps(out, indent=2))
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())