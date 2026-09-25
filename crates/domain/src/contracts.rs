use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::identity::CanonicalId;
use crate::location::SourceLocation;

/// Communication mechanism used by a cross-repository dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    Rest,
    Grpc,
    JsonRpc,
    WebSocket,
    Sse,
    Kafka,
    Rabbitmq,
    Sqs,
    Pubsub,
}

impl Mechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Mechanism::Rest => "rest",
            Mechanism::Grpc => "grpc",
            Mechanism::JsonRpc => "json_rpc",
            Mechanism::WebSocket => "websocket",
            Mechanism::Sse => "sse",
            Mechanism::Kafka => "kafka",
            Mechanism::Rabbitmq => "rabbitmq",
            Mechanism::Sqs => "sqs",
            Mechanism::Pubsub => "pubsub",
        }
    }

    pub fn is_messaging(self) -> bool {
        matches!(
            self,
            Mechanism::Kafka | Mechanism::Rabbitmq | Mechanism::Sqs | Mechanism::Pubsub
        )
    }

    pub fn from_broker(broker: &str) -> Option<Mechanism> {
        match broker.to_ascii_lowercase().as_str() {
            "kafka" => Some(Mechanism::Kafka),
            "rabbitmq" | "rabbit" | "amqp" => Some(Mechanism::Rabbitmq),
            "sqs" => Some(Mechanism::Sqs),
            "pubsub" | "gcp_pubsub" | "google_pubsub" => Some(Mechanism::Pubsub),
            _ => None,
        }
    }
}

/// Direction of a channel reference relative to the emitting code node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDirection {
    Publish,
    Subscribe,
}

/// Where a contract/evidence record was extracted from (for provenance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Code,
    Config,
    Openapi,
    Asyncapi,
    Proto,
    Helm,
    Kubernetes,
    DockerCompose,
    EnvFile,
    Readme,
    Deployment,
}

/// A contract endpoint provided (exposed) by this repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidedEndpoint {
    /// Source file this record came from; incremental merge key.
    pub file: String,
    pub mechanism: Mechanism,
    /// HTTP verb for rest/json_rpc/ws/sse (e.g. "GET"); empty otherwise.
    #[serde(default)]
    pub http_method: String,
    /// Route/path template as written (rest/ws/sse/json_rpc endpoint).
    #[serde(default)]
    pub route: String,
    /// gRPC `package.Service` or JSON-RPC namespace; empty otherwise.
    #[serde(default)]
    pub service: String,
    /// gRPC/JSON-RPC method name; empty otherwise.
    #[serde(default)]
    pub method: String,
    /// CanonicalId of the handler code node.
    #[serde(default)]
    pub handler_id: CanonicalId,
    #[serde(default)]
    pub framework: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub location: Option<SourceLocation>,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

/// A contract consumed (called/subscribed) by this repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsumedContract {
    /// Source file this record came from; incremental merge key.
    pub file: String,
    pub mechanism: Mechanism,
    /// HTTP verb for rest calls; may be empty when unknown (treated as GET).
    #[serde(default)]
    pub http_method: String,
    /// Called path / ws / sse endpoint template.
    #[serde(default)]
    pub route: String,
    /// gRPC `package.Service` or JSON-RPC namespace.
    #[serde(default)]
    pub service: String,
    /// gRPC/JSON-RPC method name.
    #[serde(default)]
    pub method: String,
    /// CanonicalId of the calling code node.
    #[serde(default)]
    pub caller_id: CanonicalId,
    /// Base URL / host / env var hint naming the target service.
    #[serde(default)]
    pub target_hint: String,
    #[serde(default)]
    pub framework: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub location: Option<SourceLocation>,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

/// A messaging channel reference (publish or subscribe) from a code node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelRef {
    /// Source file this record came from; incremental merge key.
    pub file: String,
    pub direction: ChannelDirection,
    /// Broker: kafka | rabbitmq | sqs | pubsub.
    pub broker: String,
    /// Channel kind: topic | queue | exchange ("" → inferred from broker).
    #[serde(default)]
    pub channel_type: String,
    /// Channel/topic/queue name as written.
    pub channel: String,
    /// RabbitMQ routing key when different from `channel`.
    #[serde(default)]
    pub routing_key: String,
    /// CanonicalId of the publisher/subscriber code node.
    #[serde(default)]
    pub node_id: CanonicalId,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub location: Option<SourceLocation>,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

/// A reference to an external resource (database, bucket, function, dataset,
/// table, external API) made from a code node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceRef {
    /// Source file this record came from; incremental merge key.
    pub file: String,
    /// Resource class: database | bucket | function | dataset | table | api.
    pub resource_type: String,
    /// Provider/technology discriminator (postgres | s3 | lambda | gcs |
    /// bigquery | dynamodb | ...).
    pub mechanism: String,
    /// Intended access: read | write | invoke | connect | query.
    pub access: String,
    /// Target as written: DSN/URL/name/ARN, or `env:NAME` when the target
    /// comes from an environment variable reference.
    pub raw_target: String,
    /// CanonicalId of the referencing code node.
    #[serde(default)]
    pub node_id: CanonicalId,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub location: Option<SourceLocation>,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

/// Non-code evidence used to disambiguate cross-repository matching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Source file this record came from; incremental merge key.
    pub file: String,
    pub kind: EvidenceKind,
    /// The URL / channel / host / service name extracted.
    pub value: String,
    /// Raw context (env var assignment, helm key, README sentence...).
    #[serde(default)]
    pub detail: String,
    /// Repository id this evidence points at, when derivable.
    #[serde(default)]
    pub target_repo: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub broker: String,
    #[serde(default)]
    pub line: u32,
}

/// All service-dependency contracts extracted from one repository/branch.
///
/// Persisted per repo+branch by the indexer; consumed by the workspace link
/// pass to build cross-repository edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContractBatch {
    #[serde(default)]
    pub provides: Vec<ProvidedEndpoint>,
    #[serde(default)]
    pub consumes: Vec<ConsumedContract>,
    #[serde(default)]
    pub channels: Vec<ChannelRef>,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRecord>,
}

impl ContractBatch {
    pub fn merge(&mut self, other: ContractBatch) {
        self.provides.extend(other.provides);
        self.consumes.extend(other.consumes);
        self.channels.extend(other.channels);
        self.resources.extend(other.resources);
        self.evidence.extend(other.evidence);
    }

    pub fn is_empty(&self) -> bool {
        self.provides.is_empty()
            && self.consumes.is_empty()
            && self.channels.is_empty()
            && self.resources.is_empty()
            && self.evidence.is_empty()
    }

    /// All source files referenced by this batch.
    pub fn files(&self) -> BTreeSet<String> {
        let mut files = BTreeSet::new();
        files.extend(self.provides.iter().map(|p| p.file.clone()));
        files.extend(self.consumes.iter().map(|c| c.file.clone()));
        files.extend(self.channels.iter().map(|c| c.file.clone()));
        files.extend(self.resources.iter().map(|r| r.file.clone()));
        files.extend(self.evidence.iter().map(|e| e.file.clone()));
        files
    }

    /// Replace all records originating from files present in `incoming`, then
    /// append `incoming`. This is the incremental merge: re-analyzed files
    /// fully supersede their previous records.
    pub fn replace_files(&mut self, incoming: ContractBatch) {
        let touched = incoming.files();
        self.provides.retain(|p| !touched.contains(&p.file));
        self.consumes.retain(|c| !touched.contains(&c.file));
        self.channels.retain(|c| !touched.contains(&c.file));
        self.resources.retain(|r| !touched.contains(&r.file));
        self.evidence.retain(|e| !touched.contains(&e.file));
        self.merge(incoming);
    }

    /// Drop all records originating from the given files (deleted files).
    pub fn remove_files(&mut self, files: &[String]) {
        if files.is_empty() {
            return;
        }
        let removed: BTreeSet<&str> = files.iter().map(String::as_str).collect();
        self.provides.retain(|p| !removed.contains(p.file.as_str()));
        self.consumes.retain(|c| !removed.contains(c.file.as_str()));
        self.channels.retain(|c| !removed.contains(c.file.as_str()));
        self.resources
            .retain(|r| !removed.contains(r.file.as_str()));
        self.evidence.retain(|e| !removed.contains(e.file.as_str()));
    }
}

/// Canonical, repository-independent id of a messaging topic/queue node.
///
/// The same (broker, channel) pair maps to one shared hub node across all
/// repositories and branches.
pub fn topic_id(broker: &str, channel: &str) -> CanonicalId {
    CanonicalId::from_parts(&[
        "topic",
        &broker.trim().to_ascii_lowercase(),
        &channel.trim().to_ascii_lowercase(),
    ])
}

/// Canonical, repository-independent id of an external resource node.
///
/// The same (resource_type, identity) pair maps to one shared node across all
/// repositories and branches.
pub fn resource_id(resource_type: &str, identity: &str) -> CanonicalId {
    let kind = resource_type.trim().to_ascii_lowercase();
    let ident = identity.trim().to_ascii_lowercase();
    CanonicalId::from_parts(&["resource", &kind, &ident])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provide(file: &str, route: &str) -> ProvidedEndpoint {
        ProvidedEndpoint {
            file: file.to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            handler_id: CanonicalId::from_parts(&["h", route]),
            framework: "test".to_string(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn consume(file: &str, route: &str) -> ConsumedContract {
        ConsumedContract {
            file: file.to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            caller_id: CanonicalId::from_parts(&["c", route]),
            target_hint: String::new(),
            framework: "test".to_string(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    #[test]
    fn topic_id_is_stable_and_normalized() {
        let a = topic_id("Kafka", "Orders.Created");
        let b = topic_id("kafka", "orders.created");
        assert_eq!(a, b);
        assert_ne!(a, topic_id("rabbitmq", "orders.created"));
        assert_ne!(a, topic_id("kafka", "orders.created.v2"));
    }

    #[test]
    fn resource_id_is_stable_and_normalized() {
        let a = resource_id("Bucket", "S3://Orders-Dev");
        let b = resource_id("bucket", "s3://orders-dev");
        assert_eq!(a, b);
        assert_ne!(a, resource_id("database", "s3://orders-dev"));
        assert_ne!(a, resource_id("bucket", "s3://orders-prod"));
    }

    fn resource(file: &str, target: &str) -> ResourceRef {
        ResourceRef {
            file: file.to_string(),
            resource_type: "database".to_string(),
            mechanism: "postgres".to_string(),
            access: "connect".to_string(),
            raw_target: target.to_string(),
            node_id: CanonicalId::from_parts(&["r", target]),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    #[test]
    fn resources_supersede_and_drop_with_touched_files() {
        let mut batch = ContractBatch::default();
        batch.merge(ContractBatch {
            resources: vec![
                resource("a.rs", "postgresql://db1"),
                resource("b.rs", "db2"),
            ],
            ..Default::default()
        });
        assert!(!batch.is_empty());

        batch.replace_files(ContractBatch {
            resources: vec![resource("a.rs", "postgresql://db1-new")],
            ..Default::default()
        });
        assert_eq!(batch.resources.len(), 2);
        assert!(
            batch
                .resources
                .iter()
                .any(|r| r.file == "a.rs" && r.raw_target == "postgresql://db1-new"),
            "re-analyzed file fully supersedes its resource records"
        );

        batch.remove_files(&["a.rs".to_string(), "b.rs".to_string()]);
        assert!(batch.is_empty());
    }

    #[test]
    fn mechanism_roundtrip_and_broker_mapping() {
        assert_eq!(Mechanism::Rest.as_str(), "rest");
        assert_eq!(Mechanism::JsonRpc.as_str(), "json_rpc");
        assert!(Mechanism::Kafka.is_messaging());
        assert!(!Mechanism::Grpc.is_messaging());
        assert_eq!(Mechanism::from_broker("amqp"), Some(Mechanism::Rabbitmq));
        assert_eq!(Mechanism::from_broker("nats"), None);

        let json = serde_json::to_string(&Mechanism::Sqs).unwrap();
        assert_eq!(json, "\"sqs\"");
        let back: Mechanism = serde_json::from_str("\"pubsub\"").unwrap();
        assert_eq!(back, Mechanism::Pubsub);
    }

    #[test]
    fn replace_files_supersedes_only_touched_files() {
        let mut batch = ContractBatch::default();
        batch.merge(ContractBatch {
            provides: vec![provide("a.rs", "/a"), provide("b.rs", "/b")],
            consumes: vec![consume("a.rs", "/ca")],
            ..Default::default()
        });

        let incoming = ContractBatch {
            provides: vec![provide("a.rs", "/a2")],
            consumes: vec![consume("a.rs", "/ca2")],
            ..Default::default()
        };
        batch.replace_files(incoming);

        assert_eq!(batch.provides.len(), 2);
        assert!(
            batch
                .provides
                .iter()
                .any(|p| p.file == "a.rs" && p.route == "/a2")
        );
        assert!(batch.provides.iter().any(|p| p.file == "b.rs"));
        assert_eq!(batch.consumes.len(), 1);
        assert!(
            batch
                .consumes
                .iter()
                .any(|c| c.file == "a.rs" && c.route == "/ca2"),
            "records from re-analyzed files fully supersede prior records"
        );
    }

    #[test]
    fn remove_files_drops_all_kinds_for_paths() {
        let mut batch = ContractBatch {
            provides: vec![provide("gone.rs", "/g")],
            consumes: vec![consume("gone.rs", "/cg")],
            channels: vec![ChannelRef {
                file: "gone.rs".to_string(),
                direction: ChannelDirection::Publish,
                broker: "kafka".to_string(),
                channel_type: "topic".to_string(),
                channel: "t".to_string(),
                routing_key: String::new(),
                node_id: CanonicalId::from_parts(&["n"]),
                language: "rust".to_string(),
                location: None,
                properties: Default::default(),
            }],
            resources: vec![resource("gone.rs", "postgresql://gone")],
            evidence: vec![EvidenceRecord {
                file: "gone.rs".to_string(),
                kind: EvidenceKind::Code,
                value: "v".to_string(),
                detail: String::new(),
                target_repo: String::new(),
                channel: String::new(),
                broker: String::new(),
                line: 1,
            }],
        };
        batch.remove_files(&["gone.rs".to_string()]);
        assert!(batch.is_empty());
    }

    #[test]
    fn files_collects_unique_sources() {
        let mut batch = ContractBatch::default();
        batch.merge(ContractBatch {
            provides: vec![provide("a.rs", "/a"), provide("a.rs", "/a2")],
            consumes: vec![consume("b.rs", "/b")],
            ..Default::default()
        });
        let files: Vec<String> = batch.files().into_iter().collect();
        assert_eq!(files, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }
}
