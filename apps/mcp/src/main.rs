use std::sync::Arc;

use anyhow::{Context, Result};
use ckg_graph_api::{GraphApi, QueryLimits};
use ckg_mcp_server::McpServer;
use ckg_neo4j_store::{GraphStore, Neo4jStore};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "ckg-mcp",
    version,
    about = "MCP stdio server exposing the enterprise code knowledge graph"
)]
struct Args {
    /// Neo4j bolt/neo4j URI
    #[arg(long)]
    neo4j_uri: Option<String>,
    /// Neo4j username
    #[arg(long)]
    neo4j_user: Option<String>,
    /// Neo4j password
    #[arg(long)]
    neo4j_password: Option<String>,
    /// Neo4j database name
    #[arg(long, default_value = "neo4j")]
    neo4j_database: String,
}

fn env_or(flag: Option<String>, var: &str, default: &str) -> String {
    flag.unwrap_or_else(|| {
        std::env::var(var)
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_string())
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let uri = env_or(args.neo4j_uri, "NEO4J_URI", "bolt://localhost:7687");
    let user = env_or(args.neo4j_user, "NEO4J_USER", "neo4j");
    let password = env_or(args.neo4j_password, "NEO4J_PASSWORD", "testpassword");

    let store = Neo4jStore::connect(&uri, &user, &password, &args.neo4j_database)
        .with_context(|| format!("cannot reach Neo4j at {uri}"))?;
    store
        .health()
        .await
        .with_context(|| format!("Neo4j at {uri} is unreachable"))?;

    let api = GraphApi::new(Arc::new(store), QueryLimits::default());
    let server = McpServer::new(api);
    let result = server.run_stdio().await;
    result
}
