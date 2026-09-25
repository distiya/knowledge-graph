# Enterprise Code Knowledge Graph

Rust workspace implementing a branch-aware, incrementally updated code knowledge graph with MCP access for coding agents.

**Toolchain:** Rust **1.98.1** / edition **2024** (pinned via `rust-toolchain.toml`).  
**tree-sitter 0.27** · **55 languages** registered · mega-binary with optional `dynamic-grammars`.

## Architecture

```
repos (remote or local_path) → watch / hook / webhook / CLI
  → diff-scoped Tree-sitter analysis (registry, 55 langs)
  → normalization → branch-safe graph delta → Neo4j
  → graph API (branch-scoped) → MCP server
Admin UI/API (:8080): repos, branches, default_branches, trigger index
```

## Language support

| Tier | Count | Examples |
|------|-------|----------|
| 1 — first-class | 6 | rust, javascript, typescript, python, go + tsx |
| 2 — grammar + extraction | 33 | **java, c, cpp, kotlin, sql**, ruby, php, csharp, scala, shell, elixir, dart, lua, r, html, css, yaml, toml, markdown, hcl/terraform, proto, powershell, make, haskell, julia, zig, … |
| 3 — detect-only | 16 | swift, groovy, vue, svelte, clojure, erlang, fortran, batch, … |

Detection: shared `ckg-langs` registry + `linguist` fallback + `.gitattributes`/shebang-friendly filenames.  
Optional: `--features dynamic-grammars` on `ckg-tree-sitter-analyzer` (language-pack backend).

## Quick start

```bash
# 1. Start Neo4j
./scripts/neo4j-up.sh

# 2. Index configured OSS repos
cargo run -p ckg-indexer -- index \
  --config config/workspace.oss.toml --cache-dir .ckg-cache --full \
  --neo4j-uri bolt://localhost:7687 --neo4j-user neo4j --neo4j-password testpassword

# 3. Run MCP server (stdio JSON-RPC)
cargo run -p ckg-mcp -- --neo4j-uri bolt://localhost:7687 --neo4j-user neo4j --neo4j-password testpassword

# 4. Incremental re-index (skips unchanged SHAs; diff-scoped parse)
cargo run -p ckg-indexer -- index --config config/workspace.oss.toml --cache-dir .ckg-cache \
  --neo4j-uri bolt://localhost:7687 --neo4j-user neo4j --neo4j-password testpassword
```

## Configuration

```toml
default_branches = ["main", "develop"]

[[repositories]]
id = "payments-api"
github = "acme/payments-api"           # optional when local_path set
clone_url = "https://github.com/acme/payments-api.git"
# local_path = "/home/dev/payments"    # in-place local repo (no clone)
enabled = true
monitored_branches = []                # empty → default_branches

[repositories.indexing]
languages = []                         # empty → all detected
analyzers = ["tree-sitter"]
```

Branch resolution: `monitored_branches` if non-empty → else `default_branches` → else `["main"]`.

## Cross-repository service dependencies

When `[links]` is enabled (default), each index pass re-derives service
dependencies across all configured repositories from extracted code contracts:

- HTTP/RPC calls → direct `CALLS_API` edge from caller to handler
  (`get_dependencies` / `get_dependents` / `get_change_impact` traverse them).
- Kafka/RabbitMQ/SQS/PubSub → shared `Topic` hub:
  `publisher -PUBLISH_TO-> Topic <-CONSUME_FROM- subscriber`.
- Matching: contracts → `links.base_urls` config → deployment/README evidence
  → string heuristic; every edge carries `evidence` + `confidence`
  (`high`/`medium`/`low`) and `branch_pairs`.
- Stale edges are deleted when the underlying code changes; unreferenced
  topics are garbage-collected — the graph always matches the code.

```toml
[links]
enabled = true
base_urls = { "https://api.orders.internal" = "orders-svc" }
channel_brokers = { "billing.ready" = "kafka" }
resources = { "legacy-exports" = "bucket:s3://legacy-prod-exports" }
```

## External resources and analytics

The same link pass derives dependencies on infrastructure — databases,
buckets, cloud functions, datasets/tables, and external APIs — from literal
DSNs/URLs, boto3/BigQuery/lambda usage, and environment aliases:

- One fine-grained edge per access: `READS_FROM`, `WRITES_TO`, `INVOKES`,
  `CONNECTS_TO`, `QUERIES` from the exact code node to a `Resource` node
  keyed by `resource_id(resource_type, identity)`.
- Resolution: `links.resources` registry (high) → literal normalized
  identity (high for URLs/ARNs, medium otherwise) → `.env` evidence for
  `env:NAME` aliases (medium) → unresolved placeholder (low). Every edge
  carries `evidence`, `confidence`, and `branch_pairs`.
- Stale resource edges and unreferenced `Resource` nodes are garbage
  collected exactly like `Topic` hubs; unmatched REST/WS/SSE consumes fall
  back to `CALLS_API -> Resource(api)`.

Analytics run through `get_dependency_analytics` (MCP) /
`GraphApi::get_dependency_analytics`:

- Modes: `blast_radius` (dependents of an id, with distances),
  `critical_resources`, `bridges` (betweenness), `clusters` (WCC).
- Engines degrade gracefully: APOC for blast radius, GDS PageRank /
  betweenness / WCC for ranking modes, plain Cypher otherwise; responses
  report `engine`, `degraded` + `note`, and `truncated`. Depth and limits
  are clamped; no user Cypher is accepted.
- The Docker Neo4j environment installs `apoc` and `graph-data-science`
  via `NEO4J_PLUGINS`.

## Serve, watch, and hooks

```bash
# Admin API + static UI on 0.0.0.0:8080 (creates empty config if missing)
cargo run -p ckg-indexer -- serve --config config/workspace.example.toml \
  --cache-dir .ckg-cache --port 8080

# Poll every 30s and index changed branches until ctrl-c
cargo run -p ckg-indexer -- watch --config config/workspace.example.toml \
  --cache-dir .ckg-cache --interval 30
```

### Endpoints (`ckg-indexer serve`)

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/` | Static admin UI |
| GET/POST | `/api/repos` | List / add repositories |
| DELETE/PATCH | `/api/repos/{id}` | Remove / update (`enabled`, `monitored_branches`) |
| POST | `/api/repos/{id}/index` | Trigger background index (202) |
| GET/PUT | `/api/default-branches` | Workspace default branches |
| GET | `/api/status` | Per-repo branch last/current SHAs |
| POST | `/hooks/git` | Git webhook → enqueue index (202) |

Webhook body: `{ "repo": "<id>", "ref": "refs/heads/main", "old": "...", "new": "..." }`
(GitHub-style `repository.full_name` + `before`/`after` also accepted).

Git-side hooks: see [scripts/README.md](scripts/README.md) for `post-commit` /
`post-receive` usage of `scripts/ckg-hook.sh`.


## Workspace crates

| Crate | Role |
|-------|------|
| `ckg-domain` | Canonical model: ids, nodes, edges, provenance, Analyzer trait |
| `ckg-langs` | Language registry: extensions, aliases, tiers, detection |
| `ckg-git` | Clone/fetch (or `local_path` open), branch SHAs, diffs |
| `ckg-repository-config` | TOML/JSON workspace, `default_branches`, CRUD |
| `ckg-tree-sitter-analyzer` | Structural extraction (55-lang registry, 38 grammars) |
| `ckg-scip-analyzer` / `ckg-joern-analyzer` | Stubs behind `Analyzer` trait |
| `ckg-normalizer` | Merge analyzer outputs → one graph |
| `ckg-graph-delta` | Deterministic diffs; location-aware |
| `ckg-neo4j-store` | `GraphStore` + Neo4j; **guarded deletes** (branch-safe) |
| `ckg-deployment` | Deployment env/records → graph delta |
| `ckg-graph-api` | Bounded queries with optional **branch filter** |
| `ckg-links` | Cross-repo service dependency link pass (`CALLS_API`, `Topic` hubs) |
| `ckg-mcp-server` | 16 MCP tools over stdio (15 accept `branch`) |
| `apps/indexer` | CLI: `index` / `watch` / `serve` / `status` + admin UI |
| `apps/mcp` | MCP binary |

## MCP tools

All accept optional `branch` where meaningful (omitted = cross-branch union):

`find_symbol`, `get_symbol`, `get_callers`, `get_callees`, `get_references`, `get_implementations`, `get_dependencies`, `get_dependents`, `get_branch_context`, `get_change_impact`, `get_code_context`, `compare_branch_state`, `get_deployment_state`, `get_implementation_status`, `get_architecture_context`, `get_dependency_analytics`

Unknown branches report `indexed: false` (not a silent "not found").

## Tests

```bash
./scripts/neo4j-up.sh
export NEO4J_URI=bolt://localhost:7687 NEO4J_USER=neo4j NEO4J_PASSWORD=testpassword
cargo test --workspace
```

The compose file installs `apoc` and `graph-data-science` via
`NEO4J_PLUGINS`; analytics integration tests use APOC/GDS when present and
assert the degraded Cypher fallback otherwise.

OSS integration tests clone `BurntSushi/aho-corasick` and `BurntSushi/memchr` (skip cleanly if offline).

## Design principles

1. Multiple analyzers, one canonical graph  
2. Git is source of truth; Neo4j is relationship store  
3. Incremental graph updates are first-class  
4. Branch state ≠ deployment state  
5. Agents get semantic tools, not raw Cypher  
