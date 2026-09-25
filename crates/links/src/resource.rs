use ckg_domain::{ContractBatch, RelationKind};
use ckg_repository_config::WorkspaceConfig;

use crate::matcher::{RANK_HIGH, RANK_LOW, RANK_MEDIUM};
use crate::route::normalize_route;

const KNOWN_TYPES: &[&str] = &["database", "bucket", "function", "dataset", "table", "api"];

/// A resource reference resolved to a canonical identity: registry (high) >
/// literal URL/DSN (high) > env-var evidence (medium) > bare name (medium) >
/// unresolved `env:` alias (low).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource {
    pub resource_type: String,
    pub identity: String,
    pub rank: u8,
    pub evidence: Vec<(String, String)>,
}

/// Map an extraction `access` value to the fine-grained dependency edge kind.
pub fn resource_edge_kind(access: &str) -> Option<RelationKind> {
    match access.trim().to_ascii_lowercase().as_str() {
        "read" => Some(RelationKind::ReadsFrom),
        "write" => Some(RelationKind::WritesTo),
        "invoke" => Some(RelationKind::Invokes),
        "connect" => Some(RelationKind::ConnectsTo),
        "query" => Some(RelationKind::Queries),
        _ => None,
    }
}

pub fn resolve_resource(
    workspace: &WorkspaceConfig,
    batch: &ContractBatch,
    resource_type: &str,
    raw_target: &str,
) -> Option<ResolvedResource> {
    let resource_type = resource_type.trim().to_ascii_lowercase();
    if resource_type.is_empty() {
        return None;
    }
    let raw = raw_target.trim();
    if raw.is_empty() {
        return None;
    }

    // 1. Explicit registry entry wins over everything (it may also resolve
    //    template targets the literal scanner cannot).
    if let Some((rtype, identity)) = registry_resolve(workspace, &resource_type, raw) {
        return Some(ResolvedResource {
            resource_type: rtype,
            identity,
            rank: RANK_HIGH,
            evidence: vec![("registry".to_string(), raw.to_string())],
        });
    }

    // Unresolved template targets mint garbage identities; skip them.
    if raw.contains("${") || raw.contains("$(") || raw.contains("{{") {
        return None;
    }

    // 2. Environment-variable alias: evidence first, then a low-confidence
    //    placeholder node keyed by the alias itself.
    if let Some(var) = raw
        .strip_prefix("env:")
        .or_else(|| raw.strip_prefix("ENV:"))
    {
        let var = var.trim();
        if var.is_empty() {
            return None;
        }
        if let Some((identity, file)) = evidence_resolve(batch, &resource_type, var) {
            return Some(ResolvedResource {
                resource_type,
                identity,
                rank: RANK_MEDIUM,
                evidence: vec![
                    ("env".to_string(), var.to_string()),
                    ("evidence".to_string(), file),
                ],
            });
        }
        return Some(ResolvedResource {
            resource_type,
            identity: format!("env:{var}"),
            rank: RANK_LOW,
            evidence: vec![("env".to_string(), var.to_string())],
        });
    }

    // 3. Literal target normalized per resource class.
    let identity = normalize_identity(&resource_type, raw)?;
    let rank = if is_literal_url(raw) {
        RANK_HIGH
    } else {
        RANK_MEDIUM
    };
    let evidence = vec![("resource".to_string(), identity.clone())];
    Some(ResolvedResource {
        resource_type,
        identity,
        rank,
        evidence,
    })
}

/// `links.resources` lookup: exact key first, then case-insensitive. Values
/// may carry a `"<resource_type>:"` prefix that overrides the extracted type.
fn registry_resolve(
    workspace: &WorkspaceConfig,
    resource_type: &str,
    raw: &str,
) -> Option<(String, String)> {
    let map = &workspace.links.resources;
    let value = map.get(raw).or_else(|| {
        let low = raw.to_ascii_lowercase();
        map.iter()
            .find(|(key, _)| key.to_ascii_lowercase() == low)
            .map(|(_, value)| value)
    })?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((head, tail)) = value.split_once(':') {
        let head = head.trim().to_ascii_lowercase();
        let tail = tail.trim();
        if KNOWN_TYPES.contains(&head.as_str()) && !tail.is_empty() {
            let identity = normalize_identity(&head, tail).unwrap_or_else(|| tail.to_string());
            return Some((head, identity));
        }
    }
    let identity = normalize_identity(resource_type, value).unwrap_or_else(|| value.to_string());
    Some((resource_type.to_string(), identity))
}

/// Match `env:NAME` against non-code evidence (`KEY=value` context lines) to
/// recover the concrete target.
fn evidence_resolve(
    batch: &ContractBatch,
    resource_type: &str,
    var: &str,
) -> Option<(String, String)> {
    for record in &batch.evidence {
        let key = env_detail_key(&record.detail);
        if !key.eq_ignore_ascii_case(var) {
            continue;
        }
        let value = record.value.trim();
        if value.is_empty() || value.contains("${") || value.contains("$(") {
            continue;
        }
        let identity = normalize_identity(resource_type, value)?;
        return Some((identity, record.file.clone()));
    }
    None
}

fn env_detail_key(detail: &str) -> &str {
    let detail = detail.trim();
    let cut = detail.find(['=', ':']).unwrap_or(detail.len());
    detail[..cut].trim()
}

fn is_literal_url(raw: &str) -> bool {
    let low = raw.trim().to_ascii_lowercase();
    low.contains("://") || low.starts_with("arn:") || low.starts_with("jdbc:")
}

/// Canonical identity for a resource reference. Credential-free, lowercased,
/// normalized per class so equivalent references converge on one node id.
pub fn normalize_identity(resource_type: &str, raw: &str) -> Option<String> {
    let resource_type = resource_type.trim().to_ascii_lowercase();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let identity = match resource_type.as_str() {
        "bucket" => bucket_identity(raw),
        "database" => database_identity(raw),
        "function" => function_identity(raw),
        "table" => table_identity(raw),
        "api" => api_identity(raw),
        _ => raw.to_ascii_lowercase(),
    };
    let identity = identity.trim().to_string();
    (!identity.is_empty()).then_some(identity)
}

// ---------------------------------------------------------------- buckets

fn bucket_identity(raw: &str) -> String {
    let low = raw.to_ascii_lowercase();
    if let Some(rest) = strip_scheme(&low, &["s3://", "gs://", "azblob://"]) {
        return first_segment(rest);
    }
    if let Some(after) = http_after(&low) {
        let (host, path) = split_host_path(after);
        if let Some(prefix) = host.strip_suffix(".storage.googleapis.com")
            && !prefix.is_empty()
        {
            return prefix.to_string();
        }
        if let Some(idx) = host.find(".s3.")
            && idx > 0
        {
            return host[..idx].to_string();
        }
        if host.starts_with("s3.") || host == "storage.googleapis.com" {
            return first_segment(path);
        }
        let seg = first_segment(path);
        if !seg.is_empty() {
            return seg;
        }
        return host.to_string();
    }
    first_segment(&low)
}

// --------------------------------------------------------------- databases

fn database_identity(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    let s = low.strip_prefix("jdbc:").unwrap_or(&low);
    if let Some(idx) = s.find("://") {
        let scheme = canonical_scheme(&s[..idx]);
        let rest = &s[idx + 3..];
        let (hostport, path) = split_host_path(rest);
        let hostport = match hostport.rfind('@') {
            Some(at) => &hostport[at + 1..],
            None => hostport,
        };
        let (host, port) = split_host_port(hostport);
        if host.is_empty() {
            return String::new();
        }
        let port = port.filter(|p| !is_default_port(&scheme, p));
        let mut out = format!("{scheme}://{host}");
        if let Some(port) = port {
            out.push_str(&format!(":{port}"));
        }
        let db = path.split('?').next().unwrap_or("").trim_end_matches('/');
        if !db.is_empty() && db != "/" {
            out.push_str(db);
        }
        return out;
    }
    if s.contains("host=") {
        return keyword_dsn_identity(s);
    }
    // Bare keyword-ish DSNs (e.g. oracle thin `user/pass@host:port/service`):
    // drop any userinfo prefix rather than leak it into the graph.
    match s.rfind('@') {
        Some(at) => s[at + 1..].trim_start_matches('/').to_string(),
        None => s.to_string(),
    }
}

fn keyword_dsn_identity(s: &str) -> String {
    let mut host = "";
    let mut port = "";
    let mut db = "";
    for token in keyword_tokens(s) {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
            "host" | "hostname" | "server" if !value.is_empty() => host = value,
            "port" if !value.is_empty() => port = value,
            "dbname" | "database" | "db" | "defaultdb" if !value.is_empty() => db = value,
            _ => {}
        }
    }
    if host.is_empty() {
        return s.to_string();
    }
    let mut out = host.to_string();
    if !port.is_empty() {
        out.push_str(&format!(":{port}"));
    }
    if !db.is_empty() {
        out.push('/');
        out.push_str(db);
    }
    out
}

/// Split a keyword DSN on separators, keeping quoted values intact.
fn keyword_tokens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b';' | b'&' if !in_quotes => {
                tokens.push(&s[start..i]);
                start = i + 1;
            }
            c if c.is_ascii_whitespace() && !in_quotes => {
                tokens.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    tokens.push(&s[start..]);
    tokens.retain(|t| !t.trim().is_empty());
    tokens
}

fn canonical_scheme(scheme: &str) -> String {
    match scheme {
        "postgresql" | "pgsql" => "postgres",
        "mariadb" => "mysql",
        "mssql" => "sqlserver",
        "rediss" => "redis",
        "amqps" => "amqp",
        "mongodb+srv" => "mongodb",
        "cockroachdb" => "cockroach",
        other => other,
    }
    .to_string()
}

fn is_default_port(scheme: &str, port: &str) -> bool {
    let expected = match scheme {
        "postgres" => "5432",
        "mysql" => "3306",
        "mongodb" => "27017",
        "redis" => "6379",
        "clickhouse" => "8123",
        "sqlserver" => "1433",
        "amqp" => "5672",
        "cockroach" => "26257",
        _ => return false,
    };
    port == expected
}

// --------------------------------------------------------------- functions

fn function_identity(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    if let Some(idx) = low.find(":function:") {
        let rest = &low[idx + ":function:".len()..];
        let name = rest.split(':').next().unwrap_or("");
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if let Some(after) = http_after(&low) {
        let (host, path) = split_host_path(after);
        if host.is_empty() {
            return low;
        }
        if host.ends_with(".run.app") {
            return host.to_string();
        }
        if host.ends_with(".cloudfunctions.net") {
            let seg = first_segment(path);
            if seg.is_empty() {
                return host.to_string();
            }
            return format!("{host}/{seg}");
        }
        return host.to_string();
    }
    low
}

// ------------------------------------------------------------------ tables

fn table_identity(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    if let Some(idx) = low.find(":table/") {
        let rest = &low[idx + ":table/".len()..];
        let name = rest.split('/').next().unwrap_or("");
        if !name.is_empty() {
            return name.to_string();
        }
    }
    low
}

// -------------------------------------------------------------------- apis

fn api_identity(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    if let Some(after) = http_after(&low) {
        let (host, _) = split_host_path(after);
        if !host.is_empty() {
            return host.to_string();
        }
    }
    if low.starts_with('/') {
        return normalize_route(&low);
    }
    low
}

// ---------------------------------------------------------------- helpers

fn strip_scheme<'a>(low: &'a str, schemes: &[&str]) -> Option<&'a str> {
    schemes.iter().find_map(|s| low.strip_prefix(s))
}

fn http_after(low: &str) -> Option<&str> {
    low.strip_prefix("http://")
        .or_else(|| low.strip_prefix("https://"))
}

fn split_host_path(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?', '#']) {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    }
}

fn split_host_port(hostport: &str) -> (&str, Option<&str>) {
    if let Some(bracket) = hostport.find(']') {
        let host = &hostport[..=bracket];
        let port = hostport[bracket + 1..].strip_prefix(':');
        return (host, port.filter(|p| !p.is_empty()));
    }
    match hostport.rfind(':') {
        Some(idx) => {
            let port = &hostport[idx + 1..];
            let looks_numeric = !port.is_empty() && port.chars().all(|c| c.is_ascii_digit());
            if looks_numeric {
                (&hostport[..idx], Some(port))
            } else {
                (hostport, None)
            }
        }
        None => (hostport, None),
    }
}

fn first_segment(path: &str) -> String {
    path.trim_start_matches('/')
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{EvidenceKind, EvidenceRecord};

    fn ws() -> WorkspaceConfig {
        WorkspaceConfig::default()
    }

    fn batch() -> ContractBatch {
        ContractBatch::default()
    }

    #[test]
    fn bucket_identities_converge_across_forms() {
        for raw in [
            "s3://orders-exports/2024/a.json",
            "gs://orders-exports/x",
            "https://storage.googleapis.com/orders-exports/key",
            "https://s3.amazonaws.com/orders-exports/key",
            "https://orders-exports.s3.us-east-1.amazonaws.com/key",
            "orders-exports",
        ] {
            assert_eq!(
                normalize_identity("bucket", raw).as_deref(),
                Some("orders-exports"),
                "raw {raw:?}"
            );
        }
        assert_eq!(
            normalize_identity("bucket", "s3://other-bucket/x").as_deref(),
            Some("other-bucket")
        );
    }

    #[test]
    fn database_identity_strips_credentials_and_default_ports() {
        assert_eq!(
            normalize_identity(
                "database",
                "postgres://app:s3cret@db.internal:5432/orders?sslmode=require"
            )
            .as_deref(),
            Some("postgres://db.internal/orders")
        );
        assert_eq!(
            normalize_identity("database", "jdbc:postgresql://db.internal/orders").as_deref(),
            Some("postgres://db.internal/orders"),
            "jdbc merges with the direct scheme"
        );
        assert_eq!(
            normalize_identity("database", "postgres://db.internal:6543/app").as_deref(),
            Some("postgres://db.internal:6543/app"),
            "non-default port kept"
        );
        assert_eq!(
            normalize_identity("database", "host=db.internal dbname=app port=5432").as_deref(),
            Some("db.internal:5432/app")
        );
        assert_eq!(
            normalize_identity("database", "user/pass@db.internal:1521/svc").as_deref(),
            Some("db.internal:1521/svc"),
            "oracle thin userinfo dropped"
        );
    }

    #[test]
    fn function_and_table_identities() {
        assert_eq!(
            normalize_identity(
                "function",
                "arn:aws:lambda:us-east-1:123456789012:function:billing-worker:live"
            )
            .as_deref(),
            Some("billing-worker")
        );
        assert_eq!(
            normalize_identity(
                "function",
                "https://us-central1-proj.cloudfunctions.net/billing-worker"
            )
            .as_deref(),
            Some("us-central1-proj.cloudfunctions.net/billing-worker")
        );
        assert_eq!(
            normalize_identity("function", "https://billing.a.run.app/v1").as_deref(),
            Some("billing.a.run.app")
        );
        assert_eq!(
            normalize_identity(
                "table",
                "arn:aws:dynamodb:us-east-1:123456789012:table/Orders/index/cursor"
            )
            .as_deref(),
            Some("orders")
        );
        assert_eq!(
            normalize_identity("table", "Orders").as_deref(),
            Some("orders")
        );
    }

    #[test]
    fn api_identity_is_host_or_normalized_route() {
        assert_eq!(
            normalize_identity(
                "api",
                "https://abc123.execute-api.us-east-1.amazonaws.com/prod/orders"
            )
            .as_deref(),
            Some("abc123.execute-api.us-east-1.amazonaws.com")
        );
        assert_eq!(
            normalize_identity("api", "/orders/{id}").as_deref(),
            Some("/orders/*")
        );
        assert!(normalize_identity("bucket", "  ").is_none());
    }

    #[test]
    fn literal_urls_rank_high_and_bare_names_medium() {
        let resolved =
            resolve_resource(&ws(), &batch(), "bucket", "s3://orders-exports/2024/a.json")
                .expect("resolved");
        assert_eq!(resolved.identity, "orders-exports");
        assert_eq!(resolved.rank, RANK_HIGH);
        assert_eq!(
            resolved.evidence,
            vec![("resource".to_string(), "orders-exports".to_string())]
        );

        let resolved =
            resolve_resource(&ws(), &batch(), "bucket", "orders-exports").expect("resolved");
        assert_eq!(resolved.rank, RANK_MEDIUM);
    }

    #[test]
    fn registry_entry_overrides_identity_with_type_prefix() {
        let mut ws = ws();
        ws.links.resources.insert(
            "s3://orders-dev/x".to_string(),
            "bucket:orders-prod".to_string(),
        );
        let resolved =
            resolve_resource(&ws, &batch(), "bucket", "s3://orders-dev/x").expect("resolved");
        assert_eq!(resolved.resource_type, "bucket");
        assert_eq!(resolved.identity, "orders-prod");
        assert_eq!(resolved.rank, RANK_HIGH);
        assert_eq!(
            resolved.evidence,
            vec![("registry".to_string(), "s3://orders-dev/x".to_string())]
        );
    }

    #[test]
    fn registry_matches_case_insensitively_and_resolves_templates() {
        let mut ws = ws();
        ws.links
            .resources
            .insert("S3://Orders-Dev/x".to_string(), "orders-prod".to_string());
        let resolved =
            resolve_resource(&ws, &batch(), "bucket", "s3://orders-dev/x").expect("resolved");
        assert_eq!(resolved.identity, "orders-prod");

        ws.links.resources.clear();
        ws.links
            .resources
            .insert("s3://${BUCKET}/x".to_string(), "orders-prod".to_string());
        let resolved = resolve_resource(&ws, &batch(), "bucket", "s3://${BUCKET}/x")
            .expect("registry resolves templates");
        assert_eq!(resolved.identity, "orders-prod");
    }

    #[test]
    fn template_targets_without_registry_are_skipped() {
        assert!(resolve_resource(&ws(), &batch(), "bucket", "s3://${BUCKET}/x").is_none());
        assert!(resolve_resource(&ws(), &batch(), "database", "postgres://${HOST}/db").is_none());
    }

    #[test]
    fn env_alias_resolves_via_evidence_then_falls_back_to_low() {
        let mut b = batch();
        b.evidence.push(EvidenceRecord {
            file: ".env".to_string(),
            kind: EvidenceKind::EnvFile,
            value: "postgres://app:s3cret@db.internal:5432/orders".to_string(),
            detail: "ORDERS_DSN=postgres://app:s3cret@db.internal:5432/orders".to_string(),
            target_repo: String::new(),
            channel: String::new(),
            broker: String::new(),
            line: 3,
        });
        let resolved = resolve_resource(&ws(), &b, "database", "env:ORDERS_DSN").expect("resolved");
        assert_eq!(resolved.identity, "postgres://db.internal/orders");
        assert_eq!(resolved.rank, RANK_MEDIUM);
        assert!(!resolved.identity.contains("s3cret"));
        assert_eq!(
            resolved.evidence,
            vec![
                ("env".to_string(), "ORDERS_DSN".to_string()),
                ("evidence".to_string(), ".env".to_string()),
            ]
        );

        let resolved =
            resolve_resource(&ws(), &b, "database", "env:MISSING_DSN").expect("fallback");
        assert_eq!(resolved.identity, "env:MISSING_DSN");
        assert_eq!(resolved.rank, RANK_LOW);
        assert_eq!(
            resolved.evidence,
            vec![("env".to_string(), "MISSING_DSN".to_string())]
        );
    }

    #[test]
    fn registry_resolves_env_aliases_before_evidence() {
        let mut ws = ws();
        ws.links
            .resources
            .insert("env:ORDERS_DSN".to_string(), "orders-db".to_string());
        let resolved =
            resolve_resource(&ws, &batch(), "database", "env:ORDERS_DSN").expect("resolved");
        assert_eq!(resolved.identity, "orders-db");
        assert_eq!(resolved.rank, RANK_HIGH);
    }

    #[test]
    fn access_maps_to_fine_grained_kinds() {
        assert_eq!(resource_edge_kind("read"), Some(RelationKind::ReadsFrom));
        assert_eq!(resource_edge_kind("write"), Some(RelationKind::WritesTo));
        assert_eq!(resource_edge_kind("invoke"), Some(RelationKind::Invokes));
        assert_eq!(
            resource_edge_kind("connect"),
            Some(RelationKind::ConnectsTo)
        );
        assert_eq!(resource_edge_kind("query"), Some(RelationKind::Queries));
        assert_eq!(resource_edge_kind("frobnicate"), None);
    }

    #[test]
    fn keyword_dsn_parses_host_port_db() {
        assert_eq!(
            keyword_dsn_identity("host=db.internal port=6543 dbname=app"),
            "db.internal:6543/app"
        );
        assert_eq!(
            keyword_dsn_identity("host=db.internal dbname=app"),
            "db.internal/app"
        );
        assert_eq!(
            keyword_dsn_identity("host=\"db.internal\" dbname=\"my db\""),
            "db.internal/my db"
        );
    }
}
