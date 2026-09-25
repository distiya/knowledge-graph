mod tools;

use ckg_domain::CanonicalId;
use ckg_graph_api::{AnalyticsMode, ApiError, GraphApi};
use serde::Serialize;
use serde_json::{Value, json};

pub struct McpServer {
    api: GraphApi,
}

enum ToolFailure {
    UnknownTool,
    Invalid(String),
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_success(value: Value) -> Value {
    let text = value.to_string();
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": value,
        "isError": false
    })
}

fn tool_failure(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

fn to_value<T: Serialize>(value: T) -> Result<Value, ToolFailure> {
    serde_json::to_value(value)
        .map_err(|e| ToolFailure::Invalid(format!("failed to serialize result: {e}")))
}

fn map_api_error(error: ApiError) -> ToolFailure {
    ToolFailure::Invalid(error.to_string())
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolFailure> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolFailure::Invalid(format!("missing or invalid string argument `{key}`")))
}

fn opt_str(args: &Value, key: &str) -> Result<Option<String>, ToolFailure> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| ToolFailure::Invalid(format!("argument `{key}` must be a string"))),
    }
}

fn opt_u32(args: &Value, key: &str) -> Result<Option<u32>, ToolFailure> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| {
                ToolFailure::Invalid(format!("argument `{key}` must be a non-negative integer"))
            }),
    }
}

fn opt_u8(args: &Value, key: &str) -> Result<Option<u8>, ToolFailure> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| {
                ToolFailure::Invalid(format!("argument `{key}` must be an integer in [0, 255]"))
            }),
    }
}

fn opt_string_list(args: &Value, key: &str) -> Result<Vec<String>, ToolFailure> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(v) => {
            let arr = v.as_array().ok_or_else(|| {
                ToolFailure::Invalid(format!("argument `{key}` must be an array of strings"))
            })?;
            let mut out = Vec::new();
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    ToolFailure::Invalid(format!("argument `{key}` must be an array of strings"))
                })?;
                out.push(s.to_string());
            }
            Ok(out)
        }
    }
}

fn req_string_list(args: &Value, key: &str) -> Result<Vec<String>, ToolFailure> {
    if args.get(key).is_none_or(|v| v.is_null()) {
        return Err(ToolFailure::Invalid(format!(
            "missing required argument `{key}`"
        )));
    }
    opt_string_list(args, key).and_then(|list| {
        if list.is_empty() {
            Err(ToolFailure::Invalid(format!(
                "argument `{key}` must be a non-empty array of strings"
            )))
        } else {
            Ok(list)
        }
    })
}

fn req_id(args: &Value, key: &str) -> Result<CanonicalId, ToolFailure> {
    Ok(CanonicalId::from(req_str(args, key)?))
}

impl McpServer {
    pub fn new(api: GraphApi) -> Self {
        Self { api }
    }

    pub fn tool_definitions() -> Vec<Value> {
        tools::tool_definitions()
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema
                })
            })
            .collect()
    }

    fn initialize_result(&self, params: &Value) -> Value {
        let protocol_version = params
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("2024-11-05");
        json!({
            "protocolVersion": protocol_version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": "ckg-mcp-server",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Bounded knowledge-graph queries with provenance. Traversal depth and result counts are limited by server policy; results are factual evidence only. Tools that accept a `branch` argument take an optional branch name: when provided, results are filtered to nodes present on that branch; when omitted, results are the union across branches."
        })
    }

    pub async fn handle_request(&self, request: Value) -> Value {
        let Some(obj) = request.as_object() else {
            return rpc_error(Value::Null, -32600, "invalid request: expected object");
        };

        let id = obj.get("id").cloned();

        let Some(method) = obj.get("method").and_then(|m| m.as_str()) else {
            return rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "invalid request: missing method",
            );
        };

        let Some(id) = id else {
            return Value::Null;
        };

        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));

        match method {
            "initialize" => rpc_result(id, self.initialize_result(&params)),
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, json!({"tools": Self::tool_definitions()})),
            "tools/call" => {
                let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
                    return rpc_error(id, -32602, "tools/call requires params.name");
                };
                let name = name.to_string();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match self.dispatch(&name, &arguments).await {
                    Ok(value) => rpc_result(id, tool_success(value)),
                    Err(ToolFailure::UnknownTool) => {
                        rpc_error(id, -32602, &format!("unknown tool: {name}"))
                    }
                    Err(ToolFailure::Invalid(message)) => rpc_result(id, tool_failure(&message)),
                }
            }
            "notifications/initialized" | "notifications/cancelled" => rpc_result(id, json!({})),
            other => rpc_error(id, -32601, &format!("method not found: {other}")),
        }
    }

    async fn dispatch(&self, name: &str, args: &Value) -> Result<Value, ToolFailure> {
        match name {
            "find_symbol" => {
                let symbol_name = req_str(args, "name")?;
                let repo = opt_str(args, "repo")?;
                let branch = opt_str(args, "branch")?;
                let limit = opt_u32(args, "limit")?.unwrap_or(u32::MAX);
                let result = self
                    .api
                    .find_symbol(symbol_name, repo.as_deref(), limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_symbol" => {
                let id = req_id(args, "id")?;
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_symbol(&id, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_callers" => {
                let id = req_id(args, "id")?;
                let depth = opt_u8(args, "depth")?.unwrap_or(u8::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_callers(&id, depth, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_callees" => {
                let id = req_id(args, "id")?;
                let depth = opt_u8(args, "depth")?.unwrap_or(u8::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_callees(&id, depth, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_references" => {
                let id = req_id(args, "id")?;
                let limit = opt_u32(args, "limit")?.unwrap_or(u32::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_references(&id, limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_implementations" => {
                let id = req_id(args, "id")?;
                let limit = opt_u32(args, "limit")?.unwrap_or(u32::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_implementations(&id, limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_dependencies" => {
                let id = req_id(args, "id")?;
                let limit = opt_u32(args, "limit")?.unwrap_or(u32::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_dependencies(&id, limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_dependents" => {
                let id = req_id(args, "id")?;
                let limit = opt_u32(args, "limit")?.unwrap_or(u32::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_dependents(&id, limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_branch_context" => {
                let repo = req_str(args, "repo")?;
                let branch = req_str(args, "branch")?;
                let result = self
                    .api
                    .get_branch_context(repo, branch)
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_change_impact" => {
                let id = req_id(args, "id")?;
                let depth = opt_u8(args, "depth")?.unwrap_or(u8::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_change_impact(&id, depth, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_code_context" => {
                let ids = opt_string_list(args, "ids")?
                    .into_iter()
                    .map(CanonicalId::from)
                    .collect::<Vec<_>>();
                let paths = opt_string_list(args, "paths")?;
                let depth = opt_u8(args, "depth")?.unwrap_or(u8::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_code_context(&ids, &paths, depth, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "compare_branch_state" => {
                let repo = req_str(args, "repo")?;
                let branch_a = req_str(args, "branch_a")?;
                let branch_b = req_str(args, "branch_b")?;
                let result = self
                    .api
                    .compare_branch_state(repo, branch_a, branch_b)
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_deployment_state" => {
                let repo = req_str(args, "repo")?;
                let env = req_str(args, "env")?;
                let result = self
                    .api
                    .get_deployment_state(repo, env)
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_implementation_status" => {
                let repo = req_str(args, "repo")?;
                let capability = req_str(args, "capability")?;
                let branches = req_string_list(args, "branches")?;
                let result = self
                    .api
                    .get_implementation_status(repo, capability, &branches)
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_architecture_context" => {
                let id = req_id(args, "id")?;
                let depth = opt_u8(args, "depth")?.unwrap_or(u8::MAX);
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_architecture_context(&id, depth, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            "get_dependency_analytics" => {
                let mode_raw = req_str(args, "mode")?;
                let mode: AnalyticsMode =
                    serde_json::from_value(Value::String(mode_raw.to_owned())).map_err(|_| {
                        ToolFailure::Invalid(
                            "invalid `mode` argument: expected blast_radius, critical_resources, bridges, or clusters"
                                .into(),
                        )
                    })?;
                let id = opt_str(args, "id")?.map(CanonicalId::from);
                let depth = opt_u8(args, "depth")?;
                let limit = opt_u32(args, "limit")?;
                let branch = opt_str(args, "branch")?;
                let result = self
                    .api
                    .get_dependency_analytics(id.as_ref(), mode, depth, limit, branch.as_deref())
                    .await
                    .map_err(map_api_error)?;
                to_value(result)
            }
            _ => Err(ToolFailure::UnknownTool),
        }
    }

    pub async fn run_stdio(&self) -> anyhow::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();
        let mut stdout = tokio::io::stdout();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(request) => self.handle_request(request).await,
                Err(e) => rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
            };
            if !response.is_null() {
                let mut text = serde_json::to_string(&response)?;
                text.push('\n');
                stdout.write_all(text.as_bytes()).await?;
                stdout.flush().await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ckg_domain::{
        GraphEdge, GraphNode, NodeKind, Provenance, ProvenanceSource, RelationKind, SourceLocation,
    };
    use ckg_graph_api::{DeltaOp, GraphStore, MockStore};

    fn test_server() -> (McpServer, Arc<MockStore>) {
        let store = Arc::new(MockStore::new());
        let api = GraphApi::with_defaults(store.clone());
        (McpServer::new(api), store)
    }

    async fn seed_symbol(store: &MockStore) {
        let delta = ckg_graph_api::GraphDelta {
            ops: vec![DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-parse"),
                    "parse_config",
                )
                .with_provenance(Provenance::from(ProvenanceSource::TreeSitter)),
            )],
            base_revision: None,
            target_revision: None,
        };
        store.apply_delta(&delta).await.expect("seed");
    }

    async fn seed_branch_graph(store: &MockStore) {
        let repo_id = CanonicalId::from("repo-org-pay");
        let commit_main = CanonicalId::from("commit-sha-main");
        let commit_dev = CanonicalId::from("commit-sha-dev");
        let ops = vec![
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Repository, repo_id.clone(), "org/pay")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-main"),
                "main",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-dev"),
                "develop",
            )),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_main.clone(), "sha-main")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_dev.clone(), "sha-dev")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                repo_id,
                CanonicalId::from("br-main"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                CanonicalId::from("repo-org-pay"),
                CanonicalId::from("br-dev"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-main"),
                commit_main.clone(),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-dev"),
                commit_dev.clone(),
            )),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-legacy"),
                    "legacy_parse",
                )
                .with_location(
                    SourceLocation::new("org/pay", "sha-main", "src/legacy.rs", 1, 0, 10, 0)
                        .with_branch("main"),
                ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-common"),
                    "common_util",
                )
                .with_location(
                    SourceLocation::new("org/pay", "sha-dev", "src/common.rs", 1, 0, 5, 0)
                        .with_branch("develop"),
                ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-parse"),
                    "parse_config",
                )
                .with_location(SourceLocation::new(
                    "org/pay",
                    "sha-dev",
                    "src/parse.rs",
                    1,
                    0,
                    20,
                    0,
                ))
                .with_provenance(Provenance::from(ProvenanceSource::TreeSitter)),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_main.clone(),
                CanonicalId::from("sym-legacy"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_main,
                CanonicalId::from("sym-common"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_dev.clone(),
                CanonicalId::from("sym-common"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_dev,
                CanonicalId::from("sym-parse"),
            )),
        ];
        let delta = ckg_graph_api::GraphDelta {
            ops,
            base_revision: None,
            target_revision: None,
        };
        store.apply_delta(&delta).await.expect("seed");
    }

    async fn seed_service_graph(store: &MockStore) {
        let mut ops = vec![
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Function,
                CanonicalId::from("svc-consumer"),
                "api_consumer",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Function,
                CanonicalId::from("svc-provider"),
                "api_provider",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                CanonicalId::from("svc-topic"),
                "orders.created",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-main"),
                "main",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-dev"),
                "develop",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Commit,
                CanonicalId::from("commit-main"),
                "sha-main",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Commit,
                CanonicalId::from("commit-dev"),
                "sha-dev",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-main"),
                CanonicalId::from("commit-main"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-dev"),
                CanonicalId::from("commit-dev"),
            )),
        ];
        for node_id in ["svc-consumer", "svc-provider", "svc-topic"] {
            for commit in ["commit-main", "commit-dev"] {
                ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
                    RelationKind::ContainsStateOf,
                    CanonicalId::from(commit),
                    CanonicalId::from(node_id),
                )));
            }
        }
        ops.push(DeltaOp::CreateRelationship(
            GraphEdge::new(
                RelationKind::CallsApi,
                CanonicalId::from("svc-consumer"),
                CanonicalId::from("svc-provider"),
            )
            .with_property("branch_pairs", serde_json::json!(["main"])),
        ));
        ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
            RelationKind::PublishTo,
            CanonicalId::from("svc-consumer"),
            CanonicalId::from("svc-topic"),
        )));
        let delta = ckg_graph_api::GraphDelta {
            ops,
            base_revision: None,
            target_revision: None,
        };
        store.apply_delta(&delta).await.expect("seed");
    }

    async fn call_tool(server: &McpServer, id: u64, name: &str, arguments: Value) -> Value {
        server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }))
            .await
    }

    #[tokio::test]
    async fn tools_call_get_branch_context_reports_indexed_branch() {
        let (server, store) = test_server();
        seed_branch_graph(&store).await;

        let response = call_tool(
            &server,
            20,
            "get_branch_context",
            json!({"repo": "org/pay", "branch": "main"}),
        )
        .await;

        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));
        let structured = &result["structuredContent"];
        assert_eq!(structured["data"]["indexed"], json!(true));
        assert_eq!(structured["data"]["branch"]["name"], json!("main"));
        assert_eq!(structured["data"]["commit"]["name"], json!("sha-main"));
        assert_eq!(structured["data"]["repository"]["name"], json!("org/pay"));
        let sample = structured["data"]["state_sample"]
            .as_array()
            .expect("state_sample array");
        assert!(!sample.is_empty(), "state_sample must include branch state");
        assert!(structured["provenance"].as_array().is_some());
    }

    #[tokio::test]
    async fn tools_call_get_branch_context_unknown_branch_not_indexed() {
        let (server, store) = test_server();
        seed_branch_graph(&store).await;

        let response = call_tool(
            &server,
            21,
            "get_branch_context",
            json!({"repo": "org/pay", "branch": "nope"}),
        )
        .await;

        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));
        let structured = &result["structuredContent"];
        assert_eq!(structured["data"]["indexed"], json!(false));
        assert!(
            structured["data"]["branch"].is_null(),
            "unknown branch must be absent"
        );
        assert!(
            structured["data"]["state_sample"]
                .as_array()
                .expect("array")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn tools_call_compare_branch_state_intersection_and_diff() {
        let (server, store) = test_server();
        seed_branch_graph(&store).await;

        let response = call_tool(
            &server,
            22,
            "compare_branch_state",
            json!({"repo": "org/pay", "branch_a": "main", "branch_b": "develop"}),
        )
        .await;

        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));
        let data = &result["structuredContent"]["data"];
        assert_eq!(data["common"], json!(["common_util"]));
        assert_eq!(data["only_in_a"], json!(["legacy_parse"]));
        assert_eq!(data["only_in_b"], json!(["parse_config"]));
        assert_eq!(data["branch_a_indexed"], json!(true));
        assert_eq!(data["branch_b_indexed"], json!(true));
    }

    #[tokio::test]
    async fn tools_call_implementation_status_per_branch_found_and_not_found() {
        let (server, store) = test_server();
        seed_branch_graph(&store).await;

        let response = call_tool(
            &server,
            23,
            "get_implementation_status",
            json!({
                "repo": "org/pay",
                "capability": "parse_config",
                "branches": ["develop", "main", "nope"]
            }),
        )
        .await;

        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));
        let branches = result["structuredContent"]["data"]["branches"]
            .as_array()
            .expect("branches array");
        assert_eq!(branches.len(), 3);

        assert_eq!(branches[0]["branch"], json!("develop"));
        assert_eq!(branches[0]["indexed"], json!(true));
        assert_eq!(branches[0]["found"], json!(true));
        assert_eq!(branches[0]["matches"][0]["id"], json!("sym-parse"));

        assert_eq!(branches[1]["branch"], json!("main"));
        assert_eq!(branches[1]["indexed"], json!(true));
        assert_eq!(branches[1]["found"], json!(false));
        assert!(branches[1]["matches"].as_array().expect("array").is_empty());

        assert_eq!(branches[2]["branch"], json!("nope"));
        assert_eq!(branches[2]["indexed"], json!(false));
        assert_eq!(branches[2]["found"], json!(false));
    }

    #[tokio::test]
    async fn tools_call_find_symbol_filters_by_branch() {
        let (server, store) = test_server();
        seed_branch_graph(&store).await;

        let unfiltered = call_tool(
            &server,
            24,
            "find_symbol",
            json!({"name": "parse_config", "limit": 10}),
        )
        .await;
        assert_eq!(unfiltered["result"]["isError"], json!(false));
        assert_eq!(
            unfiltered["result"]["structuredContent"]["data"]
                .as_array()
                .expect("data")
                .len(),
            1
        );

        let on_main = call_tool(
            &server,
            25,
            "find_symbol",
            json!({"name": "parse_config", "branch": "main", "limit": 10}),
        )
        .await;
        assert_eq!(on_main["result"]["isError"], json!(false));
        assert!(
            on_main["result"]["structuredContent"]["data"]
                .as_array()
                .expect("data")
                .is_empty(),
            "parse_config is not contained in main's commit"
        );

        let on_dev = call_tool(
            &server,
            26,
            "find_symbol",
            json!({"name": "parse_config", "branch": "develop", "limit": 10}),
        )
        .await;
        assert_eq!(on_dev["result"]["isError"], json!(false));
        let data = on_dev["result"]["structuredContent"]["data"]
            .as_array()
            .expect("data");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["id"], json!("sym-parse"));

        let shared_on_main = call_tool(
            &server,
            27,
            "find_symbol",
            json!({"name": "common_util", "branch": "main", "limit": 10}),
        )
        .await;
        assert_eq!(
            shared_on_main["result"]["structuredContent"]["data"]
                .as_array()
                .expect("data")
                .len(),
            1,
            "common_util is contained by main's commit too"
        );
    }

    #[tokio::test]
    async fn initialize_handshake() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test-client", "version": "0.1.0"}
                }
            }))
            .await;

        assert_eq!(response["jsonrpc"], json!("2.0"));
        assert_eq!(response["id"], json!(1));
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], json!("2024-11-05"));
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["serverInfo"]["name"], json!("ckg-mcp-server"));
        assert!(result["serverInfo"]["version"].is_string());
    }

    #[tokio::test]
    async fn tools_list_returns_at_least_ten_tools() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }))
            .await;

        assert_eq!(response["id"], json!(2));
        let tools = response["result"]["tools"].as_array().expect("tools array");
        assert!(tools.len() >= 10, "only {} tools returned", tools.len());
        assert_eq!(tools.len(), 16);

        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for expected in [
            "find_symbol",
            "get_symbol",
            "get_callers",
            "get_callees",
            "get_references",
            "get_implementations",
            "get_dependencies",
            "get_dependents",
            "get_branch_context",
            "get_change_impact",
            "get_code_context",
            "compare_branch_state",
            "get_deployment_state",
            "get_implementation_status",
            "get_architecture_context",
            "get_dependency_analytics",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }

        for tool in tools {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert_eq!(tool["inputSchema"]["type"], json!("object"));
            assert!(tool["inputSchema"]["properties"].is_object());
        }

        let branch_tools = [
            "find_symbol",
            "get_symbol",
            "get_callers",
            "get_callees",
            "get_references",
            "get_implementations",
            "get_dependencies",
            "get_dependents",
            "get_change_impact",
            "get_code_context",
            "get_architecture_context",
            "get_dependency_analytics",
        ];
        for tool in tools {
            let name = tool["name"].as_str().expect("tool name");
            if branch_tools.contains(&name) {
                assert!(
                    tool["inputSchema"]["properties"]["branch"].is_object(),
                    "tool {name} must expose optional branch property"
                );
                let required = tool["inputSchema"]["required"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert!(
                    !required.iter().any(|r| r == "branch"),
                    "branch must be optional on {name}"
                );
            }
        }
    }

    #[tokio::test]
    async fn tools_call_find_symbol_returns_structured_result() {
        let (server, store) = test_server();
        seed_symbol(&store).await;

        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "find_symbol",
                    "arguments": {"name": "parse_config", "limit": 10}
                }
            }))
            .await;

        assert_eq!(response["jsonrpc"], json!("2.0"));
        assert_eq!(response["id"], json!(3));
        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));

        let structured = &result["structuredContent"];
        assert_eq!(structured["data"][0]["name"], json!("parse_config"));
        assert_eq!(
            structured["data"][0]["id"],
            json!(CanonicalId::from("sym-parse"))
        );
        assert!(structured["provenance"].as_array().is_some());
        assert_eq!(structured["provenance"][0]["source"], json!("TREE_SITTER"));

        let text = result["content"][0]["text"].as_str().expect("text content");
        let parsed: Value = serde_json::from_str(text).expect("content is json");
        assert_eq!(parsed["data"][0]["name"], json!("parse_config"));
    }

    #[tokio::test]
    async fn tools_call_get_callers_works_through_mock() {
        let (server, store) = test_server();
        let delta = ckg_graph_api::GraphDelta {
            ops: vec![
                DeltaOp::CreateNode(GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("n-caller"),
                    "caller_fn",
                )),
                DeltaOp::CreateNode(GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("n-callee"),
                    "callee_fn",
                )),
                DeltaOp::CreateRelationship(GraphEdge::new(
                    RelationKind::Calls,
                    CanonicalId::from("n-caller"),
                    CanonicalId::from("n-callee"),
                )),
            ],
            base_revision: None,
            target_revision: None,
        };
        store.apply_delta(&delta).await.expect("seed");

        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "get_callers",
                    "arguments": {"id": CanonicalId::from("n-callee"), "depth": 2}
                }
            }))
            .await;

        let result = &response["result"];
        assert_eq!(result["isError"], json!(false));
        let structured = &result["structuredContent"];
        assert_eq!(structured["data"].as_array().expect("callers").len(), 1);
        assert_eq!(
            structured["data"][0]["edge"]["from"],
            json!(CanonicalId::from("n-caller"))
        );
        assert_eq!(structured["data"][0]["depth"], json!(1));
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "bogus/method"
            }))
            .await;
        assert_eq!(response["error"]["code"], json!(-32601));
        assert_eq!(response["id"], json!(9));
    }

    #[tokio::test]
    async fn unknown_tool_returns_invalid_params_error() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "tools/call",
                "params": {"name": "does_not_exist", "arguments": {}}
            }))
            .await;
        assert_eq!(response["error"]["code"], json!(-32602));
        assert!(
            response["error"]["message"]
                .as_str()
                .expect("message")
                .contains("does_not_exist")
        );
    }

    #[tokio::test]
    async fn store_failure_returns_tool_error_result() {
        let (server, store) = test_server();
        store.set_fail(true);
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {"name": "find_symbol", "arguments": {"name": "x"}}
            }))
            .await;
        assert_eq!(response["id"], json!(11));
        let result = &response["result"];
        assert_eq!(result["isError"], json!(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("mock store failure")
        );
    }

    #[tokio::test]
    async fn missing_tool_argument_returns_tool_error() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {"name": "find_symbol", "arguments": {}}
            }))
            .await;
        let result = &response["result"];
        assert_eq!(result["isError"], json!(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("name")
        );
    }

    #[tokio::test]
    async fn notification_without_id_returns_null() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .await;
        assert!(response.is_null());
    }

    #[tokio::test]
    async fn non_object_request_returns_invalid_request() {
        let (server, _store) = test_server();
        let response = server.handle_request(json!("just a string")).await;
        assert_eq!(response["error"]["code"], json!(-32600));
        assert!(response["id"].is_null());
    }

    #[tokio::test]
    async fn ping_returns_empty_result() {
        let (server, _store) = test_server();
        let response = server
            .handle_request(json!({"jsonrpc": "2.0", "id": 13, "method": "ping"}))
            .await;
        assert_eq!(response["result"], json!({}));
    }

    #[tokio::test]
    async fn tools_call_get_dependencies_returns_service_edges_and_filters_by_branch_pairs() {
        let (server, store) = test_server();
        seed_service_graph(&store).await;

        let unfiltered = call_tool(
            &server,
            30,
            "get_dependencies",
            json!({"id": CanonicalId::from("svc-consumer"), "limit": 10}),
        )
        .await;
        assert_eq!(unfiltered["result"]["isError"], json!(false));
        let data = unfiltered["result"]["structuredContent"]["data"]
            .as_array()
            .expect("data");
        assert_eq!(data.len(), 2);
        let kinds: Vec<&str> = data.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(
            kinds.contains(&"CALLS_API"),
            "service edge must be returned"
        );
        assert!(kinds.contains(&"PUBLISH_TO"));

        let on_main = call_tool(
            &server,
            31,
            "get_dependencies",
            json!({"id": CanonicalId::from("svc-consumer"), "limit": 10, "branch": "main"}),
        )
        .await;
        assert_eq!(on_main["result"]["isError"], json!(false));
        assert_eq!(
            on_main["result"]["structuredContent"]["data"]
                .as_array()
                .expect("data")
                .len(),
            2,
            "branch_pairs contains main"
        );

        let on_dev = call_tool(
            &server,
            32,
            "get_dependencies",
            json!({
                "id": CanonicalId::from("svc-consumer"),
                "limit": 10,
                "branch": "develop"
            }),
        )
        .await;
        assert_eq!(on_dev["result"]["isError"], json!(false));
        let dev_data = on_dev["result"]["structuredContent"]["data"]
            .as_array()
            .expect("data");
        assert_eq!(
            dev_data.len(),
            1,
            "CALLS_API branch_pairs excludes develop while endpoints pass"
        );
        assert_eq!(dev_data[0]["kind"], json!("PUBLISH_TO"));
    }

    #[tokio::test]
    async fn tools_call_get_architecture_context_includes_service_edges() {
        let (server, store) = test_server();
        seed_service_graph(&store).await;

        let response = call_tool(
            &server,
            33,
            "get_architecture_context",
            json!({"id": CanonicalId::from("svc-consumer"), "depth": 1}),
        )
        .await;
        assert_eq!(response["result"]["isError"], json!(false));
        let edges = response["result"]["structuredContent"]["data"]["edges"]
            .as_array()
            .expect("edges");
        let kinds: Vec<&str> = edges.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(kinds.contains(&"CALLS_API"));
        assert!(kinds.contains(&"PUBLISH_TO"));
        let nodes = response["result"]["structuredContent"]["data"]["nodes"]
            .as_array()
            .expect("nodes");
        let node_ids: Vec<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
        assert!(node_ids.contains(&"svc-provider"));
        assert!(node_ids.contains(&"svc-topic"));
    }

    #[tokio::test]
    async fn tools_call_get_dependency_analytics_runs_modes() {
        let (server, store) = test_server();
        let ops = vec![
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Function,
                CanonicalId::from("an-app"),
                "app_fn",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Resource,
                CanonicalId::from("an-db"),
                "orders-db",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ConnectsTo,
                CanonicalId::from("an-app"),
                CanonicalId::from("an-db"),
            )),
        ];
        store
            .apply_delta(&ckg_graph_api::GraphDelta {
                ops,
                base_revision: None,
                target_revision: None,
            })
            .await
            .expect("seed");

        let blast = call_tool(
            &server,
            40,
            "get_dependency_analytics",
            json!({
                "id": CanonicalId::from("an-db"),
                "mode": "blast_radius",
                "depth": 2
            }),
        )
        .await;
        assert_eq!(blast["result"]["isError"], json!(false));
        let data = &blast["result"]["structuredContent"]["data"];
        assert_eq!(data["mode"], json!("blast_radius"));
        assert_eq!(data["engine"], json!("cypher"));
        assert_eq!(data["degraded"], json!(true));
        let rows = data["rows"].as_array().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!("an-app"));
        assert_eq!(rows[0]["distance"], json!(1));

        let resources = call_tool(
            &server,
            41,
            "get_dependency_analytics",
            json!({"mode": "critical_resources"}),
        )
        .await;
        assert_eq!(resources["result"]["isError"], json!(false));
        let rows = resources["result"]["structuredContent"]["data"]["rows"]
            .as_array()
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!("an-db"));

        let missing_id = call_tool(
            &server,
            42,
            "get_dependency_analytics",
            json!({"mode": "blast_radius"}),
        )
        .await;
        assert_eq!(missing_id["result"]["isError"], json!(true));

        let bad_mode = call_tool(
            &server,
            43,
            "get_dependency_analytics",
            json!({"mode": "nonsense"}),
        )
        .await;
        assert_eq!(bad_mode["result"]["isError"], json!(true));
    }
}
