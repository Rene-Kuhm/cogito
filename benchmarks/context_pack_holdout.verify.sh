#!/usr/bin/env bash
#
# Verifies every expected path and symbol in benchmarks/context_pack_holdout.json
# against the pinned commit of each repository.
#
# The check runs over the same cache `cogito bench` fills
# (`.cogito/bench-cache/<name>`, see crates/cogito-harness/src/commands/bench.rs),
# so it needs no clone of its own. Override the cache with COGITO_BENCH_CACHE.
#
# Exits non-zero if any path is missing at the pinned commit, or if any expected
# symbol is not declared in one of the task's expected files.
#
# Usage:
#   benchmarks/context_pack_holdout.verify.sh
#   COGITO_BENCH_CACHE=/path/to/bench-cache benchmarks/context_pack_holdout.verify.sh

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cache=${COGITO_BENCH_CACHE:-$repo_root/.cogito/bench-cache}
set_file=$repo_root/benchmarks/context_pack_holdout.json

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 2
fi

# A declaration of $1 in one of the files named by $2.., at commit $3.
declared() {
  local sym=$1 commit=$2 dir=$3
  shift 3
  git -C "$dir" grep -qE \
    "(pub(\([^)]*\))? )?(struct|enum|trait|fn|type|const|static) $sym\b" \
    "$commit" -- "$@"
}

status=0
paths=0
symbols=0

while IFS=$'\t' read -r repo commit; do
  dir=$cache/$repo
  if [[ ! -d $dir ]]; then
    echo "SKIP  $repo: no checkout at $dir (run 'cogito bench --repo $repo' first)" >&2
    status=1
    continue
  fi
  echo "# $repo @ $commit"
  echo "# cache: $dir"

  while IFS= read -r id; do
    mapfile -t files < <(jq -r --arg i "$id" '.tasks[] | select(.id==$i) | .expect.files[]' "$set_file")
    for path in "${files[@]}"; do
      paths=$((paths + 1))
      if git -C "$dir" cat-file -e "$commit:$path" 2>/dev/null; then
        printf 'ok    %-8s path   %s\n' "$id" "$path"
      else
        printf 'FAIL  %-8s path   %s\n' "$id" "$path"
        status=1
      fi
    done

    while IFS= read -r sym; do
      symbols=$((symbols + 1))
      if declared "$sym" "$commit" "$dir" "${files[@]}"; then
        printf 'ok    %-8s symbol %s\n' "$id" "$sym"
      else
        printf 'FAIL  %-8s symbol %s (not declared in %s)\n' "$id" "$sym" "${files[*]}"
        status=1
      fi
    done < <(jq -r --arg i "$id" '.tasks[] | select(.id==$i) | .expect.symbols[]' "$set_file")
  done < <(jq -r --arg r "$repo" '.tasks[] | select(.repo==$r) | .id' "$set_file")

  echo
done < <(jq -r '.repos[] | [.name, .commit] | @tsv' "$set_file")

echo "$paths expected paths and $symbols expected symbols checked"
if [[ $status -eq 0 ]]; then
  echo "all verified"
fi
exit "$status"
