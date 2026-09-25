#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 <config-path> <repo-id> <branch>" >&2
  echo "" >&2
  echo "Runs a single-repo, single-branch index via ckg-indexer." >&2
  echo "Environment:" >&2
  echo "  CKG_INDEXER   path to an installed ckg-indexer binary" >&2
  echo "  CKG_CACHE_DIR cache directory (default: .ckg-cache)" >&2
  echo "  NEO4J_URI / NEO4J_USER / NEO4J_PASSWORD / NEO4J_DATABASE" >&2
  exit 64
}

CONFIG="${1:-}"
REPO_ID="${2:-}"
BRANCH="${3:-}"

if [[ -z "$CONFIG" || -z "$REPO_ID" || -z "$BRANCH" ]]; then
  usage
fi

if [[ -n "${CKG_INDEXER:-}" ]]; then
  BIN=("$CKG_INDEXER")
elif command -v ckg-indexer >/dev/null 2>&1; then
  BIN=(ckg-indexer)
else
  BIN=(cargo run -q -p ckg-indexer --)
fi

CACHE_DIR="${CKG_CACHE_DIR:-.ckg-cache}"

exec "${BIN[@]}" index \
  --config "$CONFIG" \
  --cache-dir "$CACHE_DIR" \
  --repo "$REPO_ID" \
  --branch "$BRANCH"
