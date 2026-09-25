use serde_json::{Value, json};

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn string_prop() -> Value {
    json!({"type": "string"})
}

fn depth_prop() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": 255})
}

fn limit_prop() -> Value {
    json!({"type": "integer", "minimum": 1})
}

fn string_array_prop() -> Value {
    json!({"type": "array", "items": {"type": "string"}})
}

fn branch_prop() -> Value {
    json!({"type": "string"})
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn id_tool(name: &'static str, description: &'static str) -> ToolDef {
    ToolDef {
        name,
        description,
        input_schema: schema(
            json!({
                "id": string_prop(),
                "depth": depth_prop(),
                "branch": branch_prop()
            }),
            &["id"],
        ),
    }
}

fn id_limit_tool(name: &'static str, description: &'static str) -> ToolDef {
    ToolDef {
        name,
        description,
        input_schema: schema(
            json!({
                "id": string_prop(),
                "limit": limit_prop(),
                "branch": branch_prop()
            }),
            &["id"],
        ),
    }
}

pub fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "find_symbol",
            description: "Find symbols by exact name, optionally filtered to a repository and branch. Returns bounded results with provenance.",
            input_schema: schema(
                json!({
                    "name": string_prop(),
                    "repo": string_prop(),
                    "branch": branch_prop(),
                    "limit": limit_prop()
                }),
                &["name"],
            ),
        },
        ToolDef {
            name: "get_symbol",
            description: "Retrieve a single symbol node by canonical id, with provenance.",
            input_schema: schema(
                json!({"id": string_prop(), "branch": branch_prop()}),
                &["id"],
            ),
        },
        id_tool(
            "get_callers",
            "Get functions that call the given symbol, traversing CALLS edges inbound up to a bounded depth.",
        ),
        id_tool(
            "get_callees",
            "Get functions called by the given symbol, traversing CALLS edges outbound up to a bounded depth.",
        ),
        id_limit_tool(
            "get_references",
            "Get REFERENCES edges pointing at the given symbol (sites that reference it), within a result limit.",
        ),
        id_limit_tool(
            "get_implementations",
            "Get IMPLEMENTS edges pointing at the given symbol (types implementing it), within a result limit.",
        ),
        id_limit_tool(
            "get_dependencies",
            "Get dependency edges leaving the given symbol: DEPENDS_ON, IMPORTS, service dependencies (CALLS_API, PUBLISH_TO, CONSUME_FROM), and external-resource dependencies (READS_FROM, WRITES_TO, INVOKES, CONNECTS_TO, QUERIES), within a result limit. Service and resource edges carry an optional branch_pairs property used for branch filtering.",
        ),
        id_limit_tool(
            "get_dependents",
            "Get dependency edges arriving at the given symbol (things that depend on it): DEPENDS_ON, IMPORTS, service dependencies (CALLS_API, PUBLISH_TO, CONSUME_FROM), and external-resource dependencies (READS_FROM, WRITES_TO, INVOKES, CONNECTS_TO, QUERIES), within a result limit. Service and resource edges carry an optional branch_pairs property used for branch filtering.",
        ),
        ToolDef {
            name: "get_branch_context",
            description: "Get repository and branch context: the branch head commit and a bounded sample of state contained in that commit.",
            input_schema: schema(
                json!({"repo": string_prop(), "branch": string_prop()}),
                &["repo", "branch"],
            ),
        },
        id_tool(
            "get_change_impact",
            "Union of callers and dependents of a symbol within a bounded depth and result budget, for change-impact analysis. Dependents include DEPENDS_ON, IMPORTS, service dependencies (CALLS_API, PUBLISH_TO, CONSUME_FROM), and external-resource dependencies (READS_FROM, WRITES_TO, INVOKES, CONNECTS_TO, QUERIES).",
        ),
        ToolDef {
            name: "get_code_context",
            description: "Get graph context for canonical symbol ids and/or source paths: matching symbols plus bounded architecture neighborhoods.",
            input_schema: schema(
                json!({
                    "ids": string_array_prop(),
                    "paths": string_array_prop(),
                    "depth": depth_prop(),
                    "branch": branch_prop()
                }),
                &[],
            ),
        },
        ToolDef {
            name: "compare_branch_state",
            description: "Compare bounded symbol-name sets of two branches of one repository: intersection and per-branch differences.",
            input_schema: schema(
                json!({
                    "repo": string_prop(),
                    "branch_a": string_prop(),
                    "branch_b": string_prop()
                }),
                &["repo", "branch_a", "branch_b"],
            ),
        },
        ToolDef {
            name: "get_deployment_state",
            description: "Get deployment facts: commits of a repository deployed to an environment via DEPLOYED_TO relationships, with status and provenance.",
            input_schema: schema(
                json!({"repo": string_prop(), "env": string_prop()}),
                &["repo", "env"],
            ),
        },
        ToolDef {
            name: "get_implementation_status",
            description: "For each requested branch, report factual evidence whether symbols matching a capability query exist under a repository, with per-branch provenance.",
            input_schema: schema(
                json!({
                    "repo": string_prop(),
                    "capability": string_prop(),
                    "branches": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1
                    }
                }),
                &["repo", "capability", "branches"],
            ),
        },
        id_tool(
            "get_architecture_context",
            "Get neighbors of a symbol across all relation kinds within a bounded depth: nodes and edges for architectural context.",
        ),
        ToolDef {
            name: "get_dependency_analytics",
            description: "Run bounded analytics over the dependency graph. Modes: blast_radius (nodes transitively depending on `id`, required for this mode), critical_resources (external resources ranked by dependency importance), bridges (nodes ranked by betweenness centrality), clusters (connected components). Returns the engine used (cypher, apoc, or gds); degraded=true with a note when a preferred engine was unavailable.",
            input_schema: schema(
                json!({
                    "id": string_prop(),
                    "mode": string_prop(),
                    "depth": depth_prop(),
                    "limit": limit_prop(),
                    "branch": branch_prop()
                }),
                &["mode"],
            ),
        },
    ]
}
