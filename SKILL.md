---
name: ckg-callgraph
description: Use when resolving call graphs — who calls X, what X calls, where a symbol is defined/referenced, blast radius of a change — instead of grepping; and when using the ckg MCP tools (find_symbol, get_symbol, get_callers, get_callees, get_references, get_change_impact, get_code_context) or when narrowing grep results with file path, line and column numbers from the knowledge graph.
---

# ckg knowledge graph — call-graph-first resolution

The `ckg` MCP server exposes a Neo4j-backed code knowledge graph indexed from Git via tree-sitter.
Every symbol node carries `location.{repository, commit_sha, path, start_line, start_column, end_line, end_column, branch}` —
**1-based lines and columns**, exactly as tree-sitter reports them. Use them to read or grep surgically
instead of spraying patterns across the repo.

## Rule

Graph first, grep second. Resolve the relationship in the graph; use grep/read only to open the exact
window the graph pointed at.

## Tool reference and result shapes

All tools wrap results as `{ data, provenance, truncated }`. Server policy clamps `depth` to 3 and
result counts to 50 — check `truncated` before treating a result set as complete.

| Tool | Required input | `data` shape | Locations included? |
|---|---|---|---|
| `find_symbol` | `name` (+ `repo`?, `branch`?, `limit`?) | `GraphNode[]` | yes |
| `get_symbol` | `id` | `GraphNode \| null` | yes |
| `get_callers` | `id` (+ `depth`?, `branch`?) | `CallEdge[]` = `{edge:{kind,from,to},depth}` | **no — IDs only** |
| `get_callees` | `id` (+ `depth`?, `branch`?) | `CallEdge[]` | **no — IDs only** |
| `get_references` | `id` (+ `limit`?, `branch`?) | `GraphEdge[]` | **no — IDs only** |
| `get_change_impact` | `id` (+ `depth`?, `branch`?) | `{root, callers, dependents, impacted[]}` | root + impacted: yes |
| `get_code_context` | `ids[]` and/or `paths[]` | `{symbols[], path_matches[], contexts[]}` | yes |
| `get_architecture_context` | `id` | `{root, nodes[], edges[]}` | nodes: yes |

`GraphNode` location:

```json
{"path": "src/main.rs", "start_line": 10, "start_column": 5,
 "end_line": 12, "end_column": 2, "commit_sha": "abc123", "branch": "main"}
```

`find_symbol` matches by case-insensitive substring, not exact name — filter candidates by `qualified_name`.

## Exact workflow

1. **Branch**: get the current branch (`git rev-parse --abbrev-ref HEAD`) and pass it as `branch`.
   Omitting `branch` unions all indexed branches — fine for discovery, noisy for verification.
2. **Locate**: `find_symbol(name, branch=…)` → pick the node whose `path` matches the code you're in →
   keep its `id` and `location`.
3. **Call graph**:
   - `get_callers(id, depth=1)` and/or `get_callees(id, depth=1)`.
   - **Each result is edge IDs only.** Collect distinct `edge.from` / `edge.to` values, then call
     `get_symbol(id)` on each to recover `path` + line + column. Never report a caller/callee without
     resolving at least the locations you are about to act on.
   - Increase `depth` to 2–3 only if depth 1 is inconclusive (budget: 50 results, `truncated` flag).
4. **Blast radius**: `get_change_impact(id)` → `root` + `impacted[]` already carry locations, no
   follow-up needed; `callers[]` again needs `get_symbol` per id.
5. **File census** (instead of grepping a whole file for definitions): `get_code_context(paths=["src/foo.rs"])`.

## Smart grep / smart read with line & column

Given a resolved `location`:

- **Read, don't grep**, when the window is known:
  `read` with `offset=start_line`, `limit=end_line - start_line + 1` (extend by ±2 for context).
  A 3-line symbol costs 3 lines of tool output instead of a repo-wide sweep.
- **Scoped grep** when you need occurrences inside that symbol:
  `rg -n --fixed-strings '<qualified_name>' <path>` — one file, never the repo, when a path is known.
  Pattern priority: `qualified_name` > `name`. Callers of `parse_config` are better found by grepping
  `parse_config(` scoped to the graph's file list than by grepping `parse` repo-wide.
- **Column disambiguation**: a line like `let a = parse(b); let c = parse(d);` yields two hits at
  different columns. Keep only the hit whose column lies in `[start_column, end_column]` (1-based,
  `rg`/`grep -n` columns are 1-based too — verify with `rg --column` or `grep -o -b` when it matters).
  Also use the column to distinguish `foo.bar` (method) from `foo::bar` (free function).
- **Order of scope expansion**: single known file → that file's directory (`rg … <dir>`) → the
  graph's file set → only then the whole repo. State which level you're at.

## Freshness, limits, fallbacks

- Compare `location.commit_sha` to `git rev-parse HEAD`. Mismatch ⇒ positions are approximate:
  open the window and confirm the symbol is really there before editing; mention the staleness.
- Graph is stale after new code lands until re-indexed (see ops below). Unindexed code legitimately
  needs grep — just say so.
- `truncated: true` ⇒ refine (lower `depth`, narrower `name`, pin `repo`/`branch`), don't conclude
  "no other callers" from a truncated list.
- Grepping is correct for: docs/markdown/config/comments, commit messages, conceptual wording,
  substring sweeps where you don't have a symbol id, and whenever the MCP server is down.
- MCP down / tool errors: Neo4j almost certainly isn't running — see ops — then fall back to grep.

## Ops (knowledge-graph repo: `/home/disti/Documents/coding/knowledge-graph`)

```bash
./scripts/neo4j-up.sh                     # start Neo4j (bolt://localhost:7687, neo4j/testpassword)
cargo run -q -p ckg-indexer -- index --config config/workspace.oss.toml \
  --cache-dir .ckg-cache --full            # full re-index
cargo run -q -p ckg-indexer -- watch --config config/workspace.oss.toml \
  --cache-dir .ckg-cache --interval 30     # or keep it fresh automatically
scripts/ckg-hook.sh config/workspace.oss.toml <repo-id> <branch>   # git post-commit hook
```

Indexed workspaces come from `config/*.toml` (currently: repo `taskboard` at
`/home/disti/Documents/coding/poc/taskboard`, branch `main`). The MCP process itself is launched by
opencode (`cargo run -q -p ckg-mcp`, cwd = knowledge-graph repo).
