# Enterprise Code Knowledge Graph — Specification

## Problem Statement

The organization has a large number of software repositories whose code, dependencies, symbols, APIs, branches, releases, deployments, and architectural relationships are difficult to understand as a single connected system.

Existing code intelligence is typically repository-local and does not provide a unified, continuously updated representation of relationships across repositories and branches.

Coding agents also lack a reliable mechanism for discovering relationships such as:

- Which functions call a particular function?
- Which repositories depend on a particular module?
- Which implementations satisfy an interface?
- Which services consume an API?
- What code may be affected by a change?
- Which branch contains a particular implementation?
- Has a requirement already been implemented on `develop` or `main`?
- Has an implemented requirement actually been deployed to production?
- Is a requested change already present in the organization's codebase but not yet released?

The system must therefore maintain an enterprise-wide knowledge graph that is incrementally updated as configured repositories and branches change and can be queried by coding agents through MCP.

Git remains the source of truth for source code and repository history. The knowledge graph stores normalized code and architectural facts, relationships, metadata, provenance, branch state, deployment information, and change information rather than replacing Git as the source-code repository.

## Solution

Build an enterprise-wide Code Knowledge Graph using **Rust as the primary implementation language**.

The system will combine multiple complementary code-intelligence technologies:

- **Tree-sitter** for structural parsing and precise source locations.
- **SCIP** for semantic symbol identities, definitions, references, implementations, and cross-file relationships.
- **Joern** for deeper program analysis such as AST, CFG, DFG, calls, and data-flow relationships where supported.
- A Rust-based normalization layer to convert analyzer-specific results into one canonical graph model.
- A Rust-based graph-delta engine to incrementally update the graph.
- **Neo4j** as the final graph database.
- A Rust graph API providing bounded semantic graph queries.
- A Rust MCP server exposing graph capabilities to coding agents.
- Repository/branch configuration defining which GitHub repositories and branches are maintained in the graph.
- Deployment metadata allowing the graph to distinguish implemented code from code actually deployed to production.

The high-level architecture is:

```text
GitHub repositories
       │
       ▼
Repository / Branch Configuration
       │
       ▼
Change Detection
       │
       ├───────────────┐
       ▼               ▼
 Tree-sitter          SCIP
       │               │
       └───────┐ ┌─────┘
               ▼ ▼
             Joern
               │
               ▼
      Canonical Normalization
               │
               ▼
          Graph Delta
               │
               ▼
             Neo4j
               │
       ┌───────┴────────┐
       ▼                ▼
 Deployment State    Graph API
                        │
                        ▼
                   MCP Server
                        │
                        ▼
                  Coding Agents
```

The graph must represent **branch-specific repository state** where configured.

For example:

```text
Repository: payment-service

main
 └── Commit A
      └── deployed to production

develop
 └── Commit B
      └── contains newer implementation
      └── not deployed to production

feature/ABC-123
 └── Commit C
      └── experimental implementation
```

This allows an agent to reason about both **implementation state** and **deployment state**.

## User Stories

1. As a developer, I want the organization's repositories represented in one knowledge graph, so that I can understand relationships across repository boundaries.

2. As a developer, I want to register which GitHub repositories are monitored, so that the organization can explicitly control the scope of the knowledge graph.

3. As a developer, I want to configure which branches of each repository are maintained, so that the graph represents the branches that are relevant to the organization's development and deployment workflow.

4. As a developer, I want different repositories to have different monitored branch configurations, so that the system does not require every repository to follow the same branching strategy.

5. As a developer, I want the system to support repositories with branches such as `main`, `develop`, release branches, or other configured branches, so that the graph reflects the organization's actual development model.

6. As a developer, I want the system to detect new commits on monitored branches, so that the graph remains current.

7. As a developer, I want the system to ignore unconfigured repositories and branches, so that unnecessary indexing does not occur.

8. As a developer, I want source files represented in the graph, so that code entities can be traced back to their source locations.

9. As a developer, I want functions and methods represented as canonical graph entities, so that their relationships can be queried consistently.

10. As a developer, I want classes, interfaces, modules, packages, and symbols represented in the graph, so that structural and semantic relationships can be explored.

11. As a developer, I want function call relationships represented, so that I can discover callers and callees.

12. As a developer, I want symbol reference relationships represented, so that I can understand where symbols are used.

13. As a developer, I want implementation relationships represented, so that I can discover implementations of interfaces or abstract types.

14. As a developer, I want inheritance relationships represented, so that I can understand class hierarchies.

15. As a developer, I want import relationships represented, so that module and file dependencies can be explored.

16. As a developer, I want repository dependencies represented, so that cross-repository relationships can be identified.

17. As a developer, I want precise source start and end positions for code entities, so that an agent can locate the exact implementation in Git.

18. As a developer, I want content hashes associated with code entities, so that unchanged and changed implementations can be identified efficiently.

19. As a developer, I want commit information associated with graph facts, so that graph state can be related to a specific Git revision.

20. As a developer, I want branch information associated with repository state, so that I can determine where a particular implementation exists.

21. As a developer, I want the graph to be incrementally updated when a commit changes a monitored repository branch, so that the entire enterprise graph does not need to be rebuilt.

22. As a developer, I want unchanged files and symbols to be skipped where possible, so that indexing remains efficient.

23. As a developer, I want deleted files and symbols removed or invalidated correctly, so that stale relationships are not retained.

24. As a developer, I want renamed or moved entities handled without unnecessarily creating duplicate logical entities.

25. As a developer, I want graph updates to be idempotent, so that processing the same repository revision more than once does not corrupt the graph.

26. As a developer, I want analyzer failures recorded as incomplete or failed provenance rather than silently producing incorrect graph facts.

27. As a developer, I want Tree-sitter used for structural information and source ranges, so that exact syntax-level information is available.

28. As a developer, I want SCIP used for semantic symbol relationships, so that references and definitions can be resolved across files and repositories.

29. As a developer, I want Joern used where deeper program analysis provides additional value, so that the graph can represent relationships beyond basic syntax and symbol indexing.

30. As a developer, I want outputs from Tree-sitter, SCIP, and Joern normalized into one graph model, so that the graph does not contain duplicate representations of the same logical symbol.

31. As a developer, I want each graph fact to retain provenance, so that I can determine which analyzer or external source produced it.

32. As a developer, I want analyzer versions recorded, so that graph data can be re-indexed when analyzer behavior changes.

33. As a developer, I want the graph builder implemented in Rust, so that the indexing system has strong performance, concurrency, memory-efficiency, and type-safety characteristics.

34. As a developer, I want analyzer integrations isolated behind well-defined Rust interfaces, so that individual analyzers can be replaced or extended without redesigning the graph system.

35. As a developer, I want the core system organized as a Rust workspace, so that domain logic, analyzers, graph processing, storage, API, and MCP components remain independently testable.

36. As a developer, I want the indexing pipeline to process repositories concurrently, so that large numbers of repositories can be indexed efficiently.

37. As a developer, I want Neo4j to be the authoritative graph store, so that graph relationships can be efficiently traversed and queried.

38. As a developer, I want Neo4j integration tests to run against a Dockerized Neo4j instance, so that graph persistence and traversal behavior can be tested consistently on development machines and in CI.

39. As a developer, I want the test environment to be reproducible without requiring a separately managed development Neo4j installation, so that contributors can run integration tests consistently.

40. As a developer, I want semantic graph operations available through an API, so that clients do not need to construct raw database queries.

41. As a developer, I want MCP to expose graph capabilities to coding agents, so that agents can discover code and architectural relationships during development tasks.

42. As a coding agent, I want to find a symbol by name or identifier, so that I can locate relevant code before making changes.

43. As a coding agent, I want to retrieve a symbol's callers, so that I can understand who may be affected by changing it.

44. As a coding agent, I want to retrieve a symbol's callees, so that I can understand its dependencies.

45. As a coding agent, I want to retrieve symbol references, so that I can understand how a symbol is used.

46. As a coding agent, I want to retrieve implementations of an interface or abstraction, so that I can understand the available implementations.

47. As a coding agent, I want to retrieve repository dependencies and dependents, so that I can understand cross-repository impact.

48. As a coding agent, I want to retrieve architectural context around a code entity, so that I can understand how code connects to services, APIs, databases, and other resources.

49. As a coding agent, I want to analyze the potential impact of a changed symbol or file, so that I can inspect relevant affected code before making a change.

50. As a coding agent, I want to determine which monitored branch contains an implementation of functionality, so that I can distinguish existing functionality from functionality that still needs to be developed.

51. As a coding agent, I want to determine whether a requirement is already implemented on `develop`, so that I do not unnecessarily implement functionality that already exists there.

52. As a coding agent, I want to determine whether an implementation exists on `main`, so that I can distinguish development-state functionality from the production-oriented branch.

53. As a coding agent, I want to determine whether an implementation has been deployed to production, so that I can distinguish implemented code from deployed functionality.

54. As a coding agent, I want to compare implementation state between branches, so that I can determine whether a requirement exists in development but has not yet reached the production branch.

55. As a coding agent, I want deployment information associated with commits or releases, so that I can trace deployed functionality back to the source revision.

56. As a coding agent, I want graph results bounded by depth and result size, so that graph queries do not return an unmanageable amount of context.

57. As a coding agent, I want graph results to identify their provenance, so that I can distinguish authoritative relationships from derived or incomplete information.

58. As a developer, I want the agent to use Git/source files as the authority for actual implementation details, so that the knowledge graph is used for relationships rather than becoming a replacement for source code.

59. As a developer, I want the system to support additional architectural data sources in the future, so that the graph can evolve beyond static source-code analysis.

60. As a developer, I want APIs, services, databases, infrastructure, ownership, and runtime relationships represented where reliable source data exists, so that the graph eventually provides enterprise architectural context.

61. As a developer, I want deployment environments represented separately from source branches, so that a branch can be associated with one or more deployment environments without assuming that branch identity alone proves deployment.

62. As a developer, I want the system to support additional analyzers in the future, so that new language and analysis capabilities can be introduced without redesigning the canonical graph model.

63. As a developer, I want semantic search and graph traversal to remain complementary capabilities, so that conceptual search and explicit relationships can both be provided to agents.

64. As a developer, I want source retrieval, graph traversal, and semantic search to remain separate concerns, so that each system can be optimized for its specific purpose.

65. As a developer, I want the graph builder to be suitable for long-running enterprise infrastructure, so that it can continuously process repository changes.

66. As a developer, I want the system to provide a stable MCP interface independent of the underlying analyzer implementation, so that coding agents can continue using the same tools as the indexing architecture evolves.

## Implementation Decisions

### Core implementation language

The core Code Knowledge Graph platform will be implemented in **Rust**.

Rust will be used for:

- Repository/change processing.
- Tree-sitter integration.
- SCIP ingestion.
- Canonical domain modeling.
- Analyzer normalization.
- Graph-delta generation.
- Neo4j persistence.
- Graph query API.
- MCP server.
- CLI and indexing workers.
- Repository and branch configuration processing.

Python is not required for the core system. It may be introduced later for independent ML, embedding, semantic-search, or experimentation workloads where its ecosystem provides a specific advantage.

### Rust workspace architecture

The system should be structured as a Rust workspace with logical crates/components similar to:

```text
code-knowledge-graph/
│
├── crates/
│   ├── domain/
│   ├── git/
│   ├── repository-config/
│   ├── tree-sitter-analyzer/
│   ├── scip-analyzer/
│   ├── joern-analyzer/
│   ├── normalizer/
│   ├── graph-delta/
│   ├── neo4j-store/
│   ├── deployment/
│   ├── graph-api/
│   └── mcp-server/
│
└── apps/
    ├── indexer/
    └── mcp/
```

Exact crate names are implementation details and may change.

The important architectural boundary is that analyzer-specific implementations do not define the canonical graph model.

### Repository registration

The system must provide a mechanism for registering repositories to be monitored.

Each configured repository should contain sufficient information to identify its GitHub repository and indexing configuration.

Conceptually:

```text
Repository
 ├── GitHub identity
 ├── enabled/disabled
 ├── monitored branches
 └── indexing configuration
```

The configuration mechanism may initially be file-based or database-backed and can later expose an administrative API/UI.

The configuration must support:

- Adding repositories.
- Removing repositories.
- Enabling/disabling monitoring.
- Configuring monitored branches.
- Updating branch configuration.
- Triggering initial indexing.
- Triggering re-indexing when necessary.

### Branch configuration

Branch monitoring is a first-class requirement.

A repository may configure:

```text
payment-service
 ├── main
 └── develop
```

while another repository may configure:

```text
customer-service
 ├── main
 ├── develop
 └── release/*
```

The implementation must not assume that every repository uses the same branching model.

Only configured branches need to have continuously maintained branch-specific graph state.

### Branch-aware graph model

The graph must be capable of representing repository state at a particular Git revision and branch.

Important entities include:

- Repository.
- Branch.
- Commit.
- File.
- Symbol/code entity.

Relationships may include:

```text
Repository ──HAS_BRANCH──> Branch
Branch ──POINTS_TO──> Commit
Commit ──CONTAINS_STATE_OF──> CodeEntity
CodeEntity ──DEFINED_IN──> File
```

The exact schema can evolve during implementation.

A key requirement is that graph queries must be able to determine whether a code entity or relationship exists at a particular branch/revision.

### Deployment model

Deployment state must be modeled separately from source-code state.

The system should be able to represent concepts such as:

- Deployment environment.
- Deployment.
- Release/version where available.
- Deployed commit.
- Deployment timestamp.
- Deployment status.

Conceptually:

```text
Production
    │
    ▲
 DEPLOYED_TO
    │
 Commit
    ▲
    │
 Branch
```

The system must not assume that `main` means “deployed to production”.

A deployment relationship should be established from reliable deployment information.

Possible future sources include:

- CI/CD systems.
- Deployment pipelines.
- Kubernetes deployment metadata.
- Release systems.
- GitHub deployment records.
- Other authoritative deployment systems.

### Requirement/implementation/deployment distinction

The graph must distinguish at least three states:

```text
Requirement
     │
     ▼
Implemented in code
     │
     ▼
Present on branch/revision
     │
     ▼
Deployed to environment
```

Therefore an agent should be able to determine situations such as:

```text
Requirement: Add X capability

develop:
    implemented = yes

main:
    implemented = no

production:
    deployed = no
```

or:

```text
develop:
    implemented = yes

main:
    implemented = yes

production:
    deployed = no
```

or:

```text
develop:
    implemented = yes

main:
    implemented = yes

production:
    deployed = yes
```

The graph itself should provide factual state and provenance. Determining whether arbitrary natural-language requirements correspond to particular implementations may additionally require semantic search or agent reasoning.

### Analyzer responsibilities

**Tree-sitter**

Tree-sitter is responsible primarily for structural information, including:

- AST structure.
- Functions and methods.
- Classes and interfaces where supported by the language grammar.
- Imports and declarations.
- Precise source ranges.
- Structural change detection.

**SCIP**

SCIP is responsible primarily for semantic information, including:

- Symbol identity.
- Definitions.
- References.
- Implementations.
- Semantic relationships across files and packages.

**Joern**

Joern is responsible for deeper program analysis where its language/frontend support and analysis capabilities provide additional value, including:

- AST relationships.
- Call relationships.
- Control-flow relationships.
- Data-flow relationships.
- Program slices and related analysis.

Joern will be treated as an analyzer integration/external analysis engine rather than making the entire graph builder dependent on Joern's internal graph representation.

### Canonical model

Analyzer-specific outputs must be normalized into a canonical enterprise graph model.

The canonical model should support entities such as:

- Organization.
- Repository.
- Branch.
- Commit.
- File.
- Package/module.
- Class.
- Interface.
- Function/method.
- Symbol.
- API.
- Service.
- Database/resource.
- Deployment environment.
- Deployment/release.

Typical relationships include:

- `HAS_BRANCH`
- `POINTS_TO`
- `DEFINED_IN`
- `CALLS`
- `REFERENCES`
- `IMPLEMENTS`
- `EXTENDS`
- `IMPORTS`
- `DEPENDS_ON`
- `EXPOSES`
- `CONSUMED_BY`
- `USES`
- `DEPLOYED_TO`

The model should be extensible as additional architectural sources are introduced.

### Canonical symbol identity

A logical symbol must have a stable canonical identity independent of which analyzer discovered it.

The identity may incorporate information such as:

- Repository.
- Namespace/package/module.
- Type/class.
- Symbol/function/method.
- Signature where required.
- Language/ecosystem.

Analyzer-specific IDs must not become the canonical enterprise identity.

### Source locations

Code entities should retain:

- Repository.
- Commit SHA.
- Branch context where applicable.
- File.
- Start line/column.
- End line/column.

Tree-sitter should be used where necessary to provide precise structural boundaries.

Git remains the source of truth for retrieving the actual source code.

### Content hashes

Content hashes represent the implementation content associated with a particular version of a code entity.

They are used to:

- Detect unchanged implementations.
- Detect changed implementations.
- Avoid unnecessary reprocessing.
- Support incremental updates.
- Associate graph state with source versions.

Content hashes do not replace logical symbol identity.

### Graph delta

The normalization pipeline must produce deterministic graph changes rather than rebuilding the complete graph unnecessarily.

Conceptual operations include:

```text
CreateNode
UpdateNode
DeleteNode
CreateRelationship
UpdateRelationship
DeleteRelationship
```

The graph-delta layer is responsible for translating normalized analyzer results into these operations.

### Incremental processing

For each monitored repository branch revision:

1. Detect the new commit.
2. Determine added, modified, deleted, renamed, or moved files where available.
3. Parse affected files with Tree-sitter.
4. Identify affected structural entities.
5. Obtain semantic information through SCIP.
6. Run applicable Joern analysis.
7. Normalize all analyzer outputs.
8. Determine affected canonical entities and relationships.
9. Generate a graph delta.
10. Apply the delta to Neo4j.
11. Update the branch's current revision.
12. Record provenance and processing status.
13. Process deployment information independently when available.

The implementation should avoid full repository re-indexing when a reliable incremental path is available.

### Provenance

Graph facts should record their source where practical, such as:

- Git.
- Tree-sitter.
- SCIP.
- Joern.
- CI/CD.
- Deployment systems.
- Future architectural data sources.

Analyzer/version metadata should be retained sufficiently to support diagnostics and future re-indexing.

Deployment relationships must identify their authoritative deployment source where possible.

### Neo4j

Neo4j is the authoritative persistent graph store.

The persistence layer should be isolated behind a Rust graph-store abstraction so that the canonical domain and graph-delta logic are not tightly coupled to Neo4j-specific APIs.

Neo4j writes should support batching and transactional consistency appropriate to graph-delta operations.

### Dockerized Neo4j development environment

The development environment will use **Docker** to provide a reproducible Neo4j instance for local development and integration testing.

The project should provide a standard Docker-based configuration allowing developers to start the required Neo4j version and dependencies without manually installing Neo4j.

The Docker environment should support:

- Starting Neo4j for local development.
- Running integration tests against Neo4j.
- Resetting the test database.
- Running a clean test environment.
- Consistent Neo4j versions across developers and CI.

Integration tests should exercise the actual Neo4j database rather than replacing all database behavior with mocks.

The project should provide a convenient developer workflow conceptually equivalent to:

```text
Start Docker Neo4j
        ↓
Run Rust integration tests
        ↓
Create test graph
        ↓
Apply graph delta
        ↓
Query graph
        ↓
Verify expected state
        ↓
Destroy/reset test environment
```

The exact Docker orchestration mechanism is an implementation decision.

### Graph API

A graph API will sit between Neo4j and MCP.

The API should expose semantic operations rather than initially exposing unrestricted Cypher.

Candidate operations include:

- `find_symbol`
- `find_symbols`
- `get_symbol`
- `get_callers`
- `get_callees`
- `get_references`
- `get_implementations`
- `get_dependencies`
- `get_dependents`
- `get_repository_context`
- `get_branch_context`
- `get_architecture_context`
- `get_change_impact`
- `get_code_context`
- `compare_branch_state`
- `get_deployment_state`
- `get_implementation_status`
- `get_dependency_analytics`

The API should enforce bounded traversal depth, result limits, and appropriate filtering.

### Branch-aware agent queries

The agent-facing graph API must allow queries to be scoped by:

- Repository.
- Branch.
- Commit/revision.
- Environment.
- Deployment state.

For example, an agent should be able to establish:

```text
Does capability X exist in develop?
Does capability X exist in main?
Which commit introduced it?
Has that commit reached production?
```

The graph API should return factual evidence and source/provenance information rather than asserting conclusions that cannot be established from graph data.

### Cross-repository service dependencies

Repository dependencies (US16, US47) are derived from code evidence during the
indexing pipeline. After each repository branch is analyzed, the extractor
records two kinds of contracts per branch in a persistent inventory:

- **Service contracts** — provided endpoints (REST framework routes, proto
  services, JSON-RPC methods, WebSocket/SSE upgrades) and consumed endpoints
  (HTTP clients, gRPC stubs, JSON-RPC/WS/SSE clients).
- **Channel contracts** — publish/subscribe references to messaging channels
  (Kafka, RabbitMQ/AMQP, SQS, Google Pub/Sub), gated on the broker library
  actually appearing in the file.

A workspace **link pass** runs after all selected repositories are indexed. It
matches each consumed contract to provided contracts in *other* repositories
and materializes the result as graph edges:

- `REST`, `gRPC`, `JSON-RPC`, `WebSocket`, `SSE` → direct
  `CALLS_API` edge from the exact consuming code node to the exact providing
  code node (handler), carrying `http_method`, normalized `route`, and
  `branch_pairs`.
- Messaging → a shared `Topic` hub node keyed by `(broker, channel)`:
  `publisher -PUBLISH_TO-> Topic <-CONSUME_FROM- subscriber`. Topic nodes are
  members of branch state via `Commit -CONTAINS_STATE_OF-> Topic`.

Matching precedence, in order: **explicit contracts → configuration
(`links.base_urls`) → deployment/helm/env/README evidence → string
heuristic**. Every edge records `evidence` entries and a `confidence` of
`high`/`medium`/`low`; a repository restriction derived from configuration
raises confidence, ambiguity across several candidate repositories lowers it.

Each link pass computes the full link set for the workspace, diffs it against
the previous one, and applies only the difference:

- Edges disappear automatically when the code that declared them is deleted
  or changed — the graph never keeps a dependency the code no longer has.
- `Topic` hubs are garbage-collected (after their `CONTAINS_STATE_OF`
  references) once no publisher or subscriber references them.

Edge membership is recorded as `branch_pairs` labels rather than putting
branches into canonical ids, so symbol identity stays branch-independent.
Cross-repository edges are never derived from a single repository's
normalized output; they come exclusively from the persisted contract
inventories of all participating branches.

Configuration:

```toml
[links]
enabled = true
base_urls = { "https://api.orders.internal" = "orders-svc" }
channel_brokers = { "billing.ready" = "kafka" }
```

### External resource dependencies

Code depends on infrastructure beyond other repositories: databases, object
storage buckets, cloud functions, datasets/tables, and external APIs. The
extractor records **resource contracts** alongside service and channel
contracts:

- **Literal targets** — DSNs and URLs (`postgresql://...`, `jdbc:...`,
  `s3://...`, `gs://...`), boto3 `Bucket=` / `TableName=` / `FunctionName=`
  arguments, BigQuery dataset/table refs, and cloud-function / Cloud Run
  URLs, recognized across python, javascript/typescript, go, java/kotlin,
  rust, and csharp sources.
- **Environment aliases** — `os.environ["DATABASE_URL"]`-style references
  mint an `env:NAME` placeholder until evidence or configuration resolves
  them.

Resource nodes are `Resource` nodes keyed by
`resource_id(resource_type, identity)` with `resource_type` in
`database | bucket | function | dataset | table | api`; identity
normalization strips credentials, default ports, and provider noise and is
branch-free. Messaging channels stay on the shared `Topic` hub model.

Each reference becomes one fine-grained edge from the exact code node:

| Access  | Edge           | Typical source                        |
|---------|----------------|---------------------------------------|
| read    | `READS_FROM`   | `get_object`, `get_item`, SELECT reads |
| write   | `WRITES_TO`    | `put_object`, `put_item`, inserts      |
| invoke  | `INVOKES`      | lambda / cloud-function calls          |
| connect | `CONNECTS_TO`  | DSN and connection setup               |
| query   | `QUERIES`      | BigQuery / SQL query execution         |

Resolution precedence, in order: **`links.resources` registry** (exact →
case-insensitive → `"<resource_type>:"`-typed value; raises confidence to
`high`) → **literal normalized identity** (high when it carries `://`,
`arn:`, or `jdbc:`, else medium) → **`.env`/deployment evidence** matching
an `env:NAME` alias (medium) → **unresolved placeholder** keyed by the
alias (low). Template targets (`${...}`, `$(...)`, `{{...}}`) are skipped
unless the registry resolves them. Unmatched REST/WS/SSE consumes fall
back to a `CALLS_API -> Resource(api)` edge with `low` confidence instead
of being dropped.

Resource edges carry `evidence`, `confidence`, `resource_type`, `access`,
`mechanism`, and `branch_pairs`, and resources join branch state via
`Commit -CONTAINS_STATE_OF-> Resource`. Branch filtering and garbage
collection therefore work exactly as for topics: when the last reference
disappears, the resource node is deleted.

```toml
[links]
resources = { "legacy-exports" = "bucket:s3://legacy-prod-exports" }
```

### Dependency graph analytics

The store exposes one bounded analytics operation, `run_analytics(spec)`,
with four modes:

- `blast_radius` — nodes transitively depending on a target id, with a
  per-node `distance` (requires `id`).
- `critical_resources` — `Resource` nodes ranked by how much the codebase
  depends on them.
- `bridges` — nodes ranked by betweenness centrality.
- `clusters` — weakly connected components, reporting `component` id and
  `score` = component size.

Engines degrade gracefully: APOC (`apoc.path.subgraphAll`) accelerates
blast radius, GDS (`gds.pageRank`, `gds.betweenness`, `gds.wcc`) powers the
ranking modes, and plain Cypher (or an in-memory traversal in the mock
store) is the portable fallback. Every response reports the `engine` that
actually ran, `degraded` plus a `note` when the preferred engine was
unavailable, and `truncated` when a limit cut the result short. Depth and
limits are clamped by the API; user-supplied Cypher is never accepted.
`bridges` and `clusters` skip with an explanatory note when GDS is absent.

The Neo4j Docker environment installs `apoc` and `graph-data-science`
through `NEO4J_PLUGINS`; integration tests assert the chosen engine's
results wherever the plugins are present and the degraded fallback
otherwise.

### MCP

MCP will be the primary agent-facing interface.

The MCP server should expose semantic graph operations rather than exposing unrestricted database access.

Candidate MCP tools include:

```text
find_symbol
get_symbol
get_callers
get_callees
get_references
get_implementations
get_dependencies
get_dependents
get_branch_context
get_change_impact
get_code_context
compare_branch_state
get_deployment_state
get_implementation_status
get_architecture_context
get_dependency_analytics
```

The MCP layer should remain independent of the internal analyzer implementations.

A coding agent should be able to:

```text
Identify symbol
      ↓
Query graph relationships
      ↓
Discover relevant repositories/files
      ↓
Determine branch state
      ↓
Determine deployment state
      ↓
Retrieve actual source from Git/workspace
      ↓
Analyze and modify code
      ↓
Use graph again when validating impact
```

### Agent usage model

The graph is not intended to replace normal source-code inspection.

Agents should use:

- **Git/source** for actual implementation.
- **Neo4j graph** for explicit relationships and branch state.
- **Deployment metadata** for deployment state.
- **Semantic/vector search** for conceptual similarity when introduced.

Graph queries are particularly useful for:

- Cross-file relationships.
- Cross-repository relationships.
- Callers/callees.
- Implementations.
- Dependencies.
- Dependents.
- Architectural relationships.
- Branch comparison.
- Change-impact analysis.
- Implementation/deployment discovery.

### Requirement discovery

Natural-language requirements should not automatically be treated as graph entities unless there is a reliable source for them.

An agent may use the graph to investigate whether a requirement appears to have already been implemented by:

1. Searching relevant code/architecture.
2. Identifying candidate symbols or services.
3. Comparing branch state.
4. Inspecting commits.
5. Inspecting actual source.
6. Checking deployment information.

The graph should provide evidence for these steps rather than claiming that semantic equivalence has been proven when it has not.

### Additional architectural sources

The graph should eventually support reliable external sources such as:

- OpenAPI specifications.
- Service configuration.
- Kubernetes manifests.
- Infrastructure-as-code.
- Dependency manifests.
- Database schemas.
- Event definitions.
- Service catalogs.
- CI/CD metadata.
- Deployment systems.
- Runtime traces.

These sources should be normalized into the same canonical graph rather than creating separate disconnected graphs.

## Testing Decisions

The highest-level behavioral testing seam should be:

```text
Git change
    ↓
Code intelligence extraction
    ↓
Normalization
    ↓
Graph delta
    ↓
Neo4j graph state
```

Tests should primarily verify externally observable behavior rather than internal implementation details.

### Dockerized Neo4j integration tests

Neo4j integration testing is a first-class requirement.

Tests should run against a real Neo4j instance provisioned through Docker.

The integration-test environment should be isolated from a developer's personal Neo4j installation.

Representative tests should verify:

- Node creation.
- Node updates.
- Node deletion.
- Relationship creation.
- Relationship deletion.
- Branch state.
- Commit state.
- Cross-repository relationships.
- Deployment relationships.
- Graph traversal.
- Idempotent graph updates.
- Incremental graph updates.

The test environment should be reset between appropriate test runs so that tests do not depend on previous graph state.

### Core pipeline tests

The system should test scenarios such as:

- A new function creates the expected graph entity.
- A modified function updates its content hash and source metadata.
- An unchanged function does not receive unnecessary updates.
- A deleted function is removed or invalidated appropriately.
- A changed call relationship is reflected in the graph.
- A changed implementation relationship is reflected in the graph.
- A renamed or moved entity does not unnecessarily create a duplicate logical symbol.
- Reprocessing the same commit produces no unintended additional changes.
- Cross-file relationships are correctly represented.
- Cross-repository relationships are correctly represented.
- Analyzer provenance is retained.
- Analyzer failures are surfaced and represented appropriately.

### Branch tests

Tests should verify:

- A configured branch is indexed.
- An unconfigured branch is not continuously indexed.
- Two configured branches can contain different graph states.
- A commit on `develop` updates the `develop` state without incorrectly changing `main`.
- A commit on `main` updates the `main` state.
- Branch state can be compared.
- The same symbol can have different implementations or relationships across branches.

### Deployment tests

Tests should verify:

- A deployed commit can be associated with an environment.
- Production and non-production environments remain distinguishable.
- Deployment state does not automatically follow from branch name.
- A feature present on `develop` but not deployed to production can be represented correctly.
- A feature present on `main` but not yet deployed can be represented correctly.
- Deployment provenance is retained.
- Deployment state can be queried through the graph API and MCP.

### Analyzer adapter tests

Tree-sitter, SCIP, and Joern adapters should have focused tests verifying that their outputs are correctly translated into the canonical intermediate representation.

These tests should not make the rest of the system dependent on analyzer-specific representations.

### Normalization tests

Normalization tests should verify:

- Stable canonical symbol identity.
- Correct merging of equivalent analyzer discoveries.
- Correct source locations.
- Correct relationship mapping.
- Correct provenance.
- Correct handling of conflicting or incomplete analyzer information.
- Correct branch/revision association.

### Graph-delta tests

Graph-delta tests should verify:

- Creation.
- Update.
- Deletion.
- Relationship changes.
- Idempotency.
- Incremental updates.
- Rename/move behavior.
- Branch-specific changes.
- Transactional behavior where applicable.

### Graph API tests

The graph API should be tested for:

- Symbol discovery.
- Caller/callee traversal.
- References.
- Implementations.
- Dependencies.
- Dependents.
- Change-impact queries.
- Branch state.
- Branch comparison.
- Deployment state.
- Implementation status.
- Result limits.
- Traversal limits.
- Missing entities.
- Provenance information.

### MCP tests

MCP tests should verify that an agent/client can invoke the semantic tools and receive the expected bounded graph context.

Tests should include scenarios where:

- Functionality exists on `develop`.
- Functionality does not exist on `main`.
- Functionality exists on `main` but is not deployed.
- Functionality is deployed to production.
- Different branches contain different implementations.

The tests should focus on MCP-visible behavior rather than MCP implementation details.

## Out of Scope

The following are initially out of scope:

- Storing complete source-code contents in Neo4j.
- Replacing Git as the source of truth.
- Building a new parser for every supported programming language.
- Reimplementing SCIP.
- Reimplementing Joern.
- Reimplementing Tree-sitter.
- Exposing unrestricted Cypher directly to coding agents.
- Automatically making code changes based solely on graph analysis.
- Treating graph relationships as proof of runtime behavior when only static analysis is available.
- Building a complete enterprise architecture management platform in the initial implementation.
- Requiring ML or vector search for the initial graph.
- Using Python for the core indexing pipeline.
- Requiring every analyzer to run for every repository and every commit.
- Supporting every language with every analyzer from the beginning.
- Assuming `main` always represents production.
- Assuming that code present on a branch has been deployed.
- Building a complete deployment platform.
- Treating natural-language requirement matching as a guaranteed fact without supporting evidence.

## Further Notes

The central architectural principle is:

> **Multiple analyzers, one canonical graph.**

Tree-sitter, SCIP, and Joern should not each create independent graphs that are later merged as separate products. Their outputs should be interpreted as different sources of intelligence about the same underlying code entities.

The second major principle is:

> **Git is the source of truth; Neo4j is the relationship store.**

The graph should tell an agent how things are related, while Git and the working tree remain authoritative for the actual source implementation.

The third major principle is:

> **Incremental graph updates are a first-class requirement.**

The system should be designed around changes and graph deltas rather than assuming that a complete graph rebuild is the normal operation.

The fourth major principle is:

> **Rust is the implementation foundation, not an analyzer-specific implementation detail.**

Rust should own the domain model, indexing pipeline, normalization, graph-delta processing, storage abstraction, graph API, and MCP integration. External analysis engines such as Joern should remain behind adapters.

The fifth principle is:

> **Agents should receive semantic capabilities, not database internals.**

MCP should expose operations such as callers, callees, implementations, dependencies, architecture context, branch comparison, deployment state, and change impact rather than requiring agents to understand the underlying Neo4j schema.

The sixth principle is:

> **Implementation state and deployment state are different facts.**

A feature being present in source code does not mean it is deployed to production.

The graph must therefore preserve the distinction:

```text
Requirement
     ↓
Implementation
     ↓
Branch
     ↓
Commit
     ↓
Release
     ↓
Deployment environment
```

This distinction is particularly important for coding agents operating in organizations with development branches such as `develop` and production-oriented branches such as `main`.

For example:

```text
User request:
"Add support for X."

Graph investigation:

develop
 └── X implementation exists
      └── commit abc123

main
 └── X implementation does not exist

production
 └── X not deployed
```

The agent can then recognize that the requested functionality may already exist in development without incorrectly assuming that it is already available in production.

Another example:

```text
develop
 └── X exists

main
 └── X exists
      └── commit def456

production
 └── deployed commit = abc999
```

In this case, the implementation exists in the production branch but has not necessarily been deployed to production.

The architecture should therefore treat **branch state and deployment state as independent dimensions**.

The final conceptual model is:

```text
                         ┌───────────────┐
                         │  Requirement  │
                         └───────┬───────┘
                                 │
                                 ▼
                         Implementation
                                 │
                  ┌──────────────┴──────────────┐
                  │                             │
                  ▼                             ▼
               develop                         main
                  │                             │
                commit                        commit
                  │                             │
                  └──────────────┬──────────────┘
                                 │
                                 ▼
                              Release
                                 │
                                 ▼
                         Deployment Environment
                                 │
                        ┌────────┴────────┐
                        ▼                 ▼
                    staging          production
```

A future semantic-search layer may complement Neo4j:

```text
                    Coding Agent
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
        Semantic Search          Graph/MCP
              │                     │
              ▼                     ▼
        Conceptually related    Explicit relationships
              │                     │
              └──────────┬──────────┘
                         ▼
                    Git / Source
```

The graph therefore serves as the organization's **structured code-and-architecture relationship layer**, while semantic search, source retrieval, branch comparison, and deployment metadata provide complementary evidence needed for intelligent agent reasoning.