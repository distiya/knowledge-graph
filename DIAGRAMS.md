# Structurizr workspace - distiya organisation

Diagram-as-code (`workspace.dsl`) for the code repositories indexed for this
organisation in the code knowledge graph (ckg).

- **Indexed today:** `distiya/taskboard` (`config/workspace.oss.toml`)
- **Evidence:** `distiya/taskboard @ a79d20ca8f7aa27953d5a06a1df4a1d0494c4875`, branch `main`

## Diagrams in the workspace

| View | Key | Shows |
| --- | --- | --- |
| System context | `SystemContext` | Task User -> Web Browser -> Taskboard |
| Container | `Containers` | Web Application (Spring Boot 4.1 / Java 25) + in-memory store |
| Component | `Components` | Hexagonal layout: web UI/controllers -> use cases -> domain ports -> adapters |
| Dynamic | `ViewBoard` | Signed-in user opens their board |
| Dynamic | `SyncBoard` | Board edit posted as JSON from `board.js` |
| Dynamic | `SignUp` | New user registers |

Adding another repository later: declare it as its own `softwareSystem` in `model`,
then reference it from its own `systemContext`/`container`/`component` views.

## Serve with Docker

The file is already named `workspace.dsl`, which is what Structurizr looks for in
its data directory.

```bash
docker pull structurizr/structurizr

docker run --rm -p 8080:8080 \
  -u "$(id -u):$(id -g)" \
  -v "$PWD":/usr/local/structurizr \
  -e STRUCTURIZR_EDITABLE=false \
  -e STRUCTURIZR_AUTOSAVEINTERVAL=0 \
  -e STRUCTURIZR_AUTOREFRESHINTERVAL=2000 \
  structurizr/structurizr local
```

Then open <http://localhost:8080> (it redirects to `/workspace/1`).

### Why these flags

- `-u "$(id -u):$(id -g)"` is **required**: the image runs as uid `65532`, so
  without it the data directory is not writable and startup fails with
  `Data directory /usr/local/structurizr is not writable`. A `:ro` bind mount
  fails for the same reason - the server needs a writable data directory to keep
  its cache/logs (it refuses to start otherwise).
- `STRUCTURIZR_EDITABLE=false` + `STRUCTURIZR_AUTOSAVEINTERVAL=0`: the UI cannot
  save, so `workspace.dsl` is never written by the server (verified by checksum).
- `STRUCTURIZR_AUTOREFRESHINTERVAL=2000`: edit `workspace.dsl` in your editor and
  the diagrams reload within 2s (the DSL is re-parsed on every refresh).
- Different port: `-e PORT=9090 -p 9090:9090`.
- Detached: replace `--rm` with `-d --name structurizr`, stop with `docker stop structurizr`.
- Several workspaces in one container: add `-e STRUCTURIZR_WORKSPACES=*` and put each
  workspace in a numeric subdirectory (`1/workspace.dsl`, `2/workspace.dsl`, ...).

### Generated files

The server writes two derived artefacts next to `workspace.dsl`; both are safe to
delete at any time (the DSL remains the single source of truth):

| Path | What it is |
| --- | --- |
| `workspace.json` | Layout/cache of the workspace, regenerated from the DSL |
| `.structurizr/` | Server logs and the Lucene search index |

Clean up:

```bash
docker rm -f structurizr; rm -rf workspace.json .structurizr
```

### Alternative images

```bash
# Structurizr Lite (legacy, single-user, no longer receiving updates)
docker run --rm -p 8080:8080 -u "$(id -u):$(id -g)" \
  -v "$PWD":/usr/local/structurizr structurizr/lite
```

## Other ways to render

```bash
# Structurizr CLI (export only, no server)
structurizr-cli export -workspace workspace.dsl -format plantuml -output out

# or paste the file into https://structurizr.com/dsl
```

## Validation

`workspace.dsl` parses with the official Structurizr DSL parser
(`com.structurizr:structurizr-dsl:6.2.3`) and exports cleanly to PlantUML and
Mermaid (6 diagrams x 2 formats).
