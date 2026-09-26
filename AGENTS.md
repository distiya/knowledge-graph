# Global Rules

## Knowledge-graph-first call resolution (mandatory)

A `ckg` MCP server is registered (code knowledge graph backed by Neo4j, indexed via tree-sitter with 1-based file paths, line numbers AND column numbers).

**Before any repo-wide `grep`/`rg`/`glob` whose goal is a code-relationship question** — who calls X, what X calls, where X is defined/referenced/implemented, what breaks if X changes — you MUST query the `ckg` MCP first. Only fall back to grep when the graph explicitly has no answer (see fallbacks below).

### Resolution workflow

1. **Locate the symbol**: `find_symbol(name, branch=<current git branch>)` → node(s) with `id` and
   `location.{path, start_line, start_column, end_line, end_column, commit_sha}` (1-based, from tree-sitter).
   Prefer the entry whose `path` is in the area you are working on.
2. **Resolve the call graph**:
   - `get_callers(id, depth)` / `get_callees(id, depth)`
   - **Gotcha: these return edge IDs only** — `{edge:{kind,from,to},depth}`, no file/line/column.
     You MUST follow each distinct `from`/`to` id with `get_symbol(id)` to obtain locations.
   - `get_change_impact(id)` for blast radius (`root` and `impacted[]` are full nodes with locations).
   - `get_references(id)` for reference sites (also edge-only → follow up with `get_symbol`).
   - `get_code_context(paths=["src/file.rs"])` to list every symbol in a file before deciding to grep.
3. **Smart grep from line/column** (do this instead of blind searching):
   - With `path` + `start_line`..`end_line` in hand, **skip grep entirely**: `read` the file with
     `offset=start_line`, `limit=end_line - start_line + 1` (add ±2 lines of context if needed).
   - If you still need to grep, scope it to the one known file — never repo-wide:
     `rg -n --fixed-strings '<qualified_name>' <path>`
   - Use `start_column`/`end_column` to disambiguate: when a line contains several matches (same
     identifier used twice, import + usage, overload), keep only the hit whose column falls within
     `[start_column, end_column]`.
   - Always prefer `qualified_name` over bare `name` as the pattern — far fewer false hits.
   - Widen scope only in this order: single file → that file's directory → repo-wide grep.
4. **Freshness and limits**:
   - Compare `location.commit_sha` against `git rev-parse HEAD`. On mismatch, treat positions as
     approximate: read the target window and verify the symbol actually sits there.
   - Depth is capped at 3 and results at 50. If `truncated: true`, refine (smaller depth, narrower
     name, `repo`/`branch` filter) instead of assuming the list is complete.

### When grep is still the right tool

- The symbol is absent from the graph (newly written / not yet re-indexed code).
- The MCP server is unreachable (usually Neo4j is down: run `./scripts/neo4j-up.sh` in
  `/home/disti/Documents/coding/knowledge-graph`, then re-index).
- The query is really text search: docs, config, comments, commit messages, conceptual wording,
  or arbitrary substring/regex sweeps across many files.

In every fallback case, say so briefly ("graph had no entry for X, grepping instead").

### Full procedure

For exact tool call sequences, result JSON shapes, and grep/read recipes, load the `ckg-callgraph`
skill when this workflow applies.
