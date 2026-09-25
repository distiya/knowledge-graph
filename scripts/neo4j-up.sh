#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

docker compose up -d neo4j

echo "Waiting for Neo4j to become ready ..."
for i in $(seq 1 60); do
  ready=0

  if command -v curl >/dev/null 2>&1; then
    if curl -sf -o /dev/null http://localhost:7474; then
      ready=1
    fi
  elif command -v nc >/dev/null 2>&1; then
    if nc -z localhost 7474 2>/dev/null && nc -z localhost 7687 2>/dev/null; then
      ready=1
    fi
  else
    if (exec 3<>/dev/tcp/localhost/7687) 2>/dev/null; then
      exec 3>&- 3<&-
      ready=1
    fi
  fi

  if [ "$ready" -eq 1 ]; then
    if command -v nc >/dev/null 2>&1 && nc -z localhost 7687 2>/dev/null; then
      echo "Neo4j is ready (HTTP :7474, bolt :7687)."
      exit 0
    fi
    if command -v cypher-shell >/dev/null 2>&1; then
      if cypher-shell -a bolt://localhost:7687 -u neo4j -p testpassword "RETURN 1" >/dev/null 2>&1; then
        echo "Neo4j is ready."
        exit 0
      fi
    fi
    if ! command -v nc >/dev/null 2>&1 && ! command -v cypher-shell >/dev/null 2>&1; then
      echo "Neo4j HTTP is ready."
      exit 0
    fi
  fi

  sleep 2
done

echo "Neo4j did not become ready in time." >&2
exit 1
