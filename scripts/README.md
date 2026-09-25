# Indexer scripts

## `ckg-hook.sh`

Runs a single-repo, single-branch index. Useful from git hooks or CI.

```bash
scripts/ckg-hook.sh <config-path> <repo-id> <branch>
```

Resolution order for the binary:

1. `$CKG_INDEXER` (path to an installed `ckg-indexer` binary)
2. `ckg-indexer` on `PATH`
3. `cargo run -q -p ckg-indexer --` from the workspace

Neo4j connection comes from `NEO4J_URI`, `NEO4J_USER`, `NEO4J_PASSWORD`,
`NEO4J_DATABASE` (or the CLI flags via the underlying `index` command defaults).

Cache directory defaults to `.ckg-cache`; override with `CKG_CACHE_DIR`.

### post-commit (inside a monitored working copy)

```bash
# .git/post-commit
/path/to/repo/scripts/ckg-hook.sh \
  /path/to/workspace/config/workspace.toml \
  payments-api \
  "$(git rev-parse --abbrev-ref HEAD)" &
```

### post-receive (bare repo / server-side)

```bash
# hooks/post-receive
while read old new ref; do
  branch="${ref#refs/heads/}"
  [ "$branch" = "$ref" ] && continue   # not a branch ref
  /path/to/scripts/ckg-hook.sh \
    /path/to/workspace/config/workspace.toml \
    payments-api "$branch" &
done
```

### Webhook alternative

If `ckg-indexer serve` is running, point the GitHub webhook at
`POST /hooks/git` instead of installing hooks:

```json
{ "repo": "payments-api", "ref": "refs/heads/main", "old": "<sha>", "new": "<sha>" }
```

GitHub-style payloads (`repository.full_name` + `ref` + `before`/`after`) are
also accepted. The server enqueues an index for monitored branches and
responds `202`.
