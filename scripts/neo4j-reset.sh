#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NEO4J_USER="${NEO4J_USER:-neo4j}"
NEO4J_PASSWORD="${NEO4J_PASSWORD:-testpassword}"

docker compose exec -T neo4j cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
  "MATCH (n) DETACH DELETE n"

echo "Database reset: all nodes and relationships deleted."
