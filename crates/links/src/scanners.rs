use ckg_domain::{ContractBatch, EvidenceKind, EvidenceRecord};

const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

const ENV_SUFFIXES: &[&str] = &[
    "_URL",
    "_URI",
    "_ENDPOINT",
    "_TOPIC",
    "_QUEUE",
    "_BROKER",
    "_BOOTSTRAP_SERVERS",
    "_HOST",
];

const DEPLOYMENT_DIRS: &[&str] = &["helm/", "charts/", "k8s/", "deploy/", "kubernetes/"];

const README_STOPWORDS: &[&str] = &[
    "topic",
    "topics",
    "queue",
    "queues",
    "endpoint",
    "endpoints",
    "api",
    "apis",
    "kafka",
    "rabbitmq",
    "amqp",
    "sqs",
    "pubsub",
    "broker",
    "brokers",
    "rest",
    "grpc",
    "url",
    "uri",
    "http",
    "https",
    "and",
    "the",
    "for",
    "with",
    "from",
    "this",
    "that",
    "using",
    "used",
    "see",
    "when",
    "what",
    "where",
];

fn ext_of(base: &str) -> &str {
    base.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
}

fn env_ext_ok(ext: &str) -> bool {
    matches!(ext, "yml" | "yaml" | "properties" | "env" | "local" | "")
}

fn is_env_name(base: &str) -> bool {
    (base == ".env"
        || base.starts_with(".env")
        || base.ends_with(".env")
        || base == "application"
        || base.starts_with("application.")
        || base.starts_with("application-")
        || base == "config"
        || base.starts_with("config.")
        || base.starts_with("config-")
        || ext_of(base) == "properties")
        && env_ext_ok(ext_of(base))
}

fn is_compose_name(base: &str) -> bool {
    base.starts_with("docker-compose")
}

fn is_deployment_name(base: &str) -> bool {
    base.starts_with("deployment") || base.starts_with("values") || is_compose_name(base)
}

fn has_deployment_dir(lower_path: &str) -> bool {
    DEPLOYMENT_DIRS.iter().any(|dir| lower_path.contains(dir))
}

fn is_docs_path(lower_path: &str) -> bool {
    lower_path.starts_with("docs/") || lower_path.contains("/docs/")
}

/// Cheap filename/path gate used by the indexer pipeline to decide which file
/// contents are worth reading for evidence scanning.
pub fn is_scannable(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or("");
    if base.is_empty() {
        return false;
    }
    if base.starts_with("readme") {
        return true;
    }
    if is_env_name(base) {
        return true;
    }
    match ext_of(base) {
        "yml" | "yaml" => true,
        "md" | "markdown" => is_docs_path(&lower),
        "json" => base.contains("openapi") || base.contains("swagger") || base.contains("asyncapi"),
        _ => false,
    }
}

fn looks_like_marker(content: &str, marker: &str) -> bool {
    content
        .lines()
        .take(40)
        .any(|line| !line.starts_with(char::is_whitespace) && line.starts_with(marker))
}

fn record(
    file: &str,
    kind: EvidenceKind,
    value: impl Into<String>,
    detail: &str,
    line: u32,
) -> EvidenceRecord {
    EvidenceRecord {
        file: file.to_string(),
        kind,
        value: value.into(),
        detail: detail.to_string(),
        target_repo: String::new(),
        channel: String::new(),
        broker: String::new(),
        line,
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).trim().to_string()
}

fn after_key<'a>(line: &'a str, key: &'a str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    let stripped = trimmed.strip_prefix('-').unwrap_or(trimmed).trim_start();
    let (k, v) = stripped.split_once(':')?;
    if k.trim() == key { Some(v) } else { None }
}

fn is_env_key(raw: &str) -> bool {
    let key = raw.trim().trim_matches(['"', '\'']);
    if key.is_empty() {
        return false;
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return false;
    }
    let upper = key.to_ascii_uppercase();
    ENV_SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
}

fn env_channel(upper_key: &str, value: &str) -> String {
    if (upper_key.ends_with("_TOPIC") || upper_key.ends_with("_QUEUE")) && !value.trim().is_empty()
    {
        value.trim().to_string()
    } else {
        String::new()
    }
}

fn env_broker(upper_key: &str) -> String {
    if upper_key.contains("KAFKA") {
        "kafka".to_string()
    } else if upper_key.contains("RABBIT") || upper_key.contains("AMQP") {
        "rabbitmq".to_string()
    } else if upper_key.contains("SQS") {
        "sqs".to_string()
    } else if upper_key.contains("PUBSUB") {
        "pubsub".to_string()
    } else if upper_key.ends_with("_BOOTSTRAP_SERVERS") {
        "kafka".to_string()
    } else {
        String::new()
    }
}

fn find_urls(line: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let http = rest.find("http://");
        let https = rest.find("https://");
        let start = match (http, https) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(start) = start else { break };
        let scheme_len = if rest[start..].starts_with("https://") {
            8
        } else {
            7
        };
        let after = &rest[start + scheme_len..];
        let end_rel = after
            .find(|c: char| c.is_whitespace() || ")]}\"'<>`,".contains(c))
            .unwrap_or(after.len());
        let url = rest[start..start + scheme_len + end_rel].trim_end_matches(['.', ';', ':']);
        if url.len() > scheme_len {
            urls.push(url.to_string());
        }
        rest = &rest[start + scheme_len + end_rel..];
    }
    urls
}

fn find_line_containing(content: &str, needle: &str) -> u32 {
    content
        .lines()
        .position(|line| line.contains(needle))
        .map(|idx| idx as u32 + 1)
        .unwrap_or(0)
}

fn scan_openapi(path: &str, content: &str, ext: &str) -> ContractBatch {
    let mut batch = ContractBatch::default();
    if ext == "json" {
        let Ok(spec) = serde_json::from_str::<serde_json::Value>(content) else {
            return batch;
        };
        let Some(paths) = spec.get("paths").and_then(|p| p.as_object()) else {
            return batch;
        };
        for (route, item) in paths {
            let Some(operations) = item.as_object() else {
                continue;
            };
            for (method, operation) in operations {
                if !HTTP_METHODS.contains(&method.as_str()) || !operation.is_object() {
                    continue;
                }
                let needle = format!("\"{route}\"");
                let line = find_line_containing(content, &needle);
                let detail = content
                    .lines()
                    .nth(line.saturating_sub(1) as usize)
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .unwrap_or(method)
                    .to_string();
                batch.evidence.push(record(
                    path,
                    EvidenceKind::Openapi,
                    format!("{} {}", method.to_ascii_uppercase(), route),
                    &detail,
                    line,
                ));
            }
        }
        return batch;
    }

    let mut in_paths = false;
    let mut paths_indent = 0usize;
    let mut route = String::new();
    let mut route_indent = 0usize;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        let lineno = idx as u32 + 1;
        if !in_paths {
            if trimmed.starts_with("paths:") {
                in_paths = true;
                paths_indent = indent;
            }
            continue;
        }
        if indent <= paths_indent {
            break;
        }
        if trimmed.ends_with(':') {
            let key = unquote(trimmed.trim_end_matches(':'));
            if key.starts_with('/') {
                route = key;
                route_indent = indent;
                continue;
            }
        }
        if route.is_empty() {
            continue;
        }
        if indent <= route_indent {
            route.clear();
            continue;
        }
        let Some(method) = trimmed.strip_suffix(':') else {
            continue;
        };
        let method = method.trim().to_ascii_lowercase();
        if !HTTP_METHODS.contains(&method.as_str()) {
            continue;
        }
        batch.evidence.push(record(
            path,
            EvidenceKind::Openapi,
            format!("{} {route}", method.to_ascii_uppercase()),
            trimmed,
            lineno,
        ));
    }
    batch
}

fn broker_from_block(lower_block: &str) -> &'static str {
    if lower_block.contains("kafka") {
        "kafka"
    } else if lower_block.contains("amqp") || lower_block.contains("rabbit") {
        "rabbitmq"
    } else if lower_block.contains("sqs") || lower_block.contains("sns") {
        "sqs"
    } else if lower_block.contains("pubsub") || lower_block.contains("googleapis") {
        "pubsub"
    } else {
        ""
    }
}

fn scan_asyncapi(path: &str, content: &str, ext: &str) -> ContractBatch {
    let mut batch = ContractBatch::default();
    if ext == "json" {
        let Ok(spec) = serde_json::from_str::<serde_json::Value>(content) else {
            return batch;
        };
        let Some(channels) = spec.get("channels").and_then(|c| c.as_object()) else {
            return batch;
        };
        for channel in channels.keys() {
            let needle = format!("\"{channel}\"");
            let line = find_line_containing(content, &needle);
            let mut rec = record(path, EvidenceKind::Asyncapi, channel.clone(), &needle, line);
            rec.channel = channel.clone();
            rec.broker =
                broker_from_block(&channels[channel].to_string().to_ascii_lowercase()).to_string();
            batch.evidence.push(rec);
        }
        return batch;
    }

    let mut in_channels = false;
    let mut channels_indent = 0usize;
    let mut child_indent: Option<usize> = None;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        let lineno = idx as u32 + 1;
        if !in_channels {
            if trimmed.starts_with("channels:") {
                in_channels = true;
                channels_indent = indent;
            }
            continue;
        }
        if indent <= channels_indent {
            break;
        }
        if !trimmed.ends_with(':') {
            continue;
        }
        match child_indent {
            None => {
                child_indent = Some(indent);
                push_channel(&mut batch, path, trimmed, indent, lineno, content, idx);
            }
            Some(child) if indent == child => {
                push_channel(&mut batch, path, trimmed, indent, lineno, content, idx);
            }
            Some(_) => {}
        }
    }
    batch
}

fn push_channel(
    batch: &mut ContractBatch,
    path: &str,
    key_line: &str,
    key_indent: usize,
    lineno: u32,
    content: &str,
    idx: usize,
) {
    let channel = unquote(key_line.trim_end_matches(':'));
    if channel.is_empty() {
        return;
    }
    let mut block = String::new();
    for line in content.lines().skip(idx + 1) {
        if !line.trim().is_empty() && indent_of(line) <= key_indent {
            break;
        }
        block.push_str(line);
        block.push('\n');
    }
    let lower = block.to_ascii_lowercase();
    let broker = broker_from_block(&lower);
    let mut rec = record(
        path,
        EvidenceKind::Asyncapi,
        channel.clone(),
        key_line.trim(),
        lineno,
    );
    rec.channel = channel;
    rec.broker = broker.to_string();
    batch.evidence.push(rec);
}

fn push_env_record(
    batch: &mut ContractBatch,
    path: &str,
    kind: EvidenceKind,
    key: &str,
    value: &str,
    detail: &str,
    lineno: u32,
) {
    if value.trim().is_empty() {
        return;
    }
    let upper = key.to_ascii_uppercase();
    let mut rec = record(path, kind, value.to_string(), detail, lineno);
    rec.channel = env_channel(&upper, value);
    rec.broker = env_broker(&upper);
    batch.evidence.push(rec);
}

fn scan_deployment(path: &str, content: &str, base: &str) -> ContractBatch {
    let kind = if is_compose_name(base) {
        EvidenceKind::DockerCompose
    } else {
        EvidenceKind::Kubernetes
    };
    let mut batch = ContractBatch::default();
    let lines: Vec<&str> = content.lines().collect();
    let mut ingress_host = String::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let lineno = idx as u32 + 1;

        if let Some(value) = after_key(trimmed, "image") {
            let value = unquote(value);
            if !value.is_empty() && !value.starts_with('$') && !value.starts_with('#') {
                batch
                    .evidence
                    .push(record(path, kind, value, trimmed, lineno));
            }
        }

        for url in find_urls(line) {
            batch
                .evidence
                .push(record(path, kind, url, trimmed, lineno));
        }

        if let Some(name) = after_key(trimmed, "name") {
            let var = unquote(name);
            if is_env_key(&var) {
                let mut value = String::new();
                for lookahead in lines.iter().skip(idx + 1).take(6) {
                    let t = lookahead.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    if after_key(t, "name").is_some() {
                        break;
                    }
                    if let Some(v) = after_key(t, "value") {
                        value = unquote(v);
                        break;
                    }
                }
                push_env_record(&mut batch, path, kind, &var, &value, trimmed, lineno);
            }
        }

        let bare = trimmed.strip_prefix('-').unwrap_or(trimmed).trim();
        if let Some((key, value)) = bare.split_once('=')
            && is_env_key(key)
        {
            push_env_record(
                &mut batch,
                path,
                kind,
                key,
                unquote(value).as_str(),
                trimmed,
                lineno,
            );
        } else if let Some((key, value)) = bare.split_once(':')
            && is_env_key(key)
        {
            push_env_record(
                &mut batch,
                path,
                kind,
                key,
                unquote(value).as_str(),
                trimmed,
                lineno,
            );
        }

        if let Some(topic) = after_key(trimmed, "topic") {
            let value = unquote(topic);
            if !value.is_empty() {
                let mut rec = record(path, kind, value.clone(), trimmed, lineno);
                rec.channel = value;
                rec.broker = "kafka".to_string();
                batch.evidence.push(rec);
            }
        }

        let lower = trimmed.to_ascii_lowercase();
        if (lower.contains("bootstrap.servers") || lower.contains("bootstrap_servers"))
            && let Some((_, value)) = trimmed.split_once(':')
        {
            let value = unquote(value);
            if !value.is_empty() {
                let mut rec = record(path, kind, value, trimmed, lineno);
                rec.broker = "kafka".to_string();
                batch.evidence.push(rec);
            }
        }

        if let Some(host) = after_key(trimmed, "host") {
            let host = unquote(host);
            if !host.is_empty() && !host.contains(' ') && !host.starts_with('$') {
                ingress_host = host;
            }
        } else if let Some(path_value) = after_key(trimmed, "path") {
            let path_value = unquote(path_value);
            if !ingress_host.is_empty() && path_value.starts_with('/') {
                batch.evidence.push(record(
                    path,
                    kind,
                    format!("{ingress_host} {path_value}"),
                    trimmed,
                    lineno,
                ));
            }
        }
    }
    batch
}

fn scan_env(path: &str, content: &str) -> ContractBatch {
    let mut batch = ContractBatch::default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':')) else {
            continue;
        };
        if !is_env_key(key) {
            continue;
        }
        let lineno = idx as u32 + 1;
        push_env_record(
            &mut batch,
            path,
            EvidenceKind::EnvFile,
            key,
            unquote(value).as_str(),
            trimmed,
            lineno,
        );
    }
    batch
}

fn scan_readme(path: &str, content: &str) -> ContractBatch {
    let mut batch = ContractBatch::default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lineno = idx as u32 + 1;
        for url in find_urls(line) {
            batch
                .evidence
                .push(record(path, EvidenceKind::Readme, url, trimmed, lineno));
        }
        for span in backtick_spans(trimmed) {
            let span = span.trim();
            if span.starts_with('/') && !span.contains(' ') {
                batch
                    .evidence
                    .push(record(path, EvidenceKind::Readme, span, trimmed, lineno));
            }
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("topic") || lower.contains("queue") {
            for token in trimmed.split_whitespace() {
                let token = token.trim_matches(|c: char| c.is_ascii_punctuation());
                if topic_shaped(token) {
                    let mut rec = record(path, EvidenceKind::Readme, token, trimmed, lineno);
                    rec.channel = token.to_string();
                    batch.evidence.push(rec);
                }
            }
        }
    }
    batch
}

fn backtick_spans(line: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut start = None;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'`' {
            match start.take() {
                None => start = Some(i + 1),
                Some(s) => {
                    if let Ok(span) = std::str::from_utf8(&bytes[s..i]) {
                        spans.push(span);
                    }
                }
            }
        }
    }
    spans
}

fn topic_shaped(token: &str) -> bool {
    if token.chars().count() < 3 {
        return false;
    }
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return false;
    }
    if !token.contains(['.', '-', '_']) {
        return false;
    }
    !README_STOPWORDS.contains(&token.to_ascii_lowercase().as_str())
}

/// Pattern-gated evidence scanner: dispatches on filename/path first and only
/// then inspects content. Records share the source `file` key used by
/// [`ckg_domain::ContractBatch::replace_files`].
pub fn scan_file(path: &str, content: &str) -> ContractBatch {
    let norm = path.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or("").to_string();
    let ext = ext_of(&base).to_string();
    let yamlish = matches!(ext.as_str(), "yml" | "yaml");

    if yamlish || ext == "json" {
        if base.starts_with("openapi")
            || base.contains("swagger")
            || looks_like_marker(content, "openapi:")
            || looks_like_marker(content, "swagger:")
        {
            return scan_openapi(path, content, &ext);
        }
        if yamlish && looks_like_marker(content, "asyncapi:") {
            return scan_asyncapi(path, content, &ext);
        }
        if ext == "json" && content.contains("\"asyncapi\":") {
            return scan_asyncapi(path, content, &ext);
        }
    }
    if yamlish && (has_deployment_dir(&lower) || is_deployment_name(&base)) {
        return scan_deployment(path, content, &base);
    }
    if is_env_name(&base) {
        return scan_env(path, content);
    }
    if base.starts_with("readme") || is_docs_path(&lower) {
        return scan_readme(path, content);
    }
    ContractBatch::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_yaml_emits_method_path_evidence() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Orders
paths:
  /api/orders:
    get:
      summary: list
    post:
      summary: create
  /api/orders/{id}:
    get:
      summary: fetch
components: {}
"#;
        let batch = scan_file("openapi/orders.yaml", spec);
        assert_eq!(
            batch.provides.len() + batch.consumes.len() + batch.channels.len(),
            0
        );
        let values: Vec<&str> = batch.evidence.iter().map(|e| e.value.as_str()).collect();
        assert!(values.contains(&"GET /api/orders"), "got {values:?}");
        assert!(values.contains(&"POST /api/orders"), "got {values:?}");
        assert!(values.contains(&"GET /api/orders/{id}"), "got {values:?}");
        assert!(
            batch
                .evidence
                .iter()
                .all(|e| e.kind == EvidenceKind::Openapi)
        );
        assert!(
            batch
                .evidence
                .iter()
                .all(|e| e.file == "openapi/orders.yaml")
        );
        let get = batch
            .evidence
            .iter()
            .find(|e| e.value == "GET /api/orders")
            .expect("get evidence");
        assert_eq!(get.line, 7);
        assert_eq!(get.detail, "get:");
    }

    #[test]
    fn openapi_json_emits_evidence() {
        let spec = r#"{
  "openapi": "3.0.0",
  "paths": {
    "/api/orders": { "get": { "summary": "list" } }
  }
}"#;
        let batch = scan_file("openapi.json", spec);
        assert_eq!(batch.evidence.len(), 1);
        assert_eq!(batch.evidence[0].value, "GET /api/orders");
        assert_eq!(batch.evidence[0].kind, EvidenceKind::Openapi);
        assert!(batch.evidence[0].line > 0);
    }

    #[test]
    fn asyncapi_channels_emit_channel_and_broker() {
        let spec = r#"
asyncapi: 2.6.0
channels:
  orders.created:
    bindings:
      kafka:
        topic: orders.created
  payments/queue:
    bindings:
      amqp:
        queue: payments
"#;
        let batch = scan_file("asyncapi.yaml", spec);
        assert_eq!(batch.evidence.len(), 2, "got {:?}", batch.evidence);
        assert!(
            batch
                .evidence
                .iter()
                .all(|e| e.kind == EvidenceKind::Asyncapi)
        );
        let orders = &batch.evidence[0];
        assert_eq!(orders.channel, "orders.created");
        assert_eq!(orders.broker, "kafka");
        assert_eq!(orders.line, 4);
        let payments = &batch.evidence[1];
        assert_eq!(payments.channel, "payments/queue");
        assert_eq!(payments.broker, "rabbitmq");
    }

    #[test]
    fn env_file_url_and_topic_records() {
        let env = "# service endpoints\nORDERS_BASE_URL=https://api.orders.internal/v1\nPAYMENTS_TOPIC=payments.settled\nEMPTY=\n";
        let batch = scan_file(".env", env);
        assert_eq!(batch.evidence.len(), 2, "got {:?}", batch.evidence);
        let url = &batch.evidence[0];
        assert_eq!(url.kind, EvidenceKind::EnvFile);
        assert_eq!(url.value, "https://api.orders.internal/v1");
        assert_eq!(url.detail, "ORDERS_BASE_URL=https://api.orders.internal/v1");
        assert_eq!(url.line, 2);
        let topic = &batch.evidence[1];
        assert_eq!(topic.channel, "payments.settled");
        assert_eq!(topic.value, "payments.settled");
    }

    #[test]
    fn compose_file_emits_topic_channel() {
        let compose = r#"
services:
  producer:
    image: confluentinc/cp-kafka:7.5
    environment:
      KAFKA_TOPIC: orders.created
      KAFKA_BOOTSTRAP_SERVERS: kafka:9092
"#;
        let batch = scan_file("docker-compose.yml", compose);
        assert_eq!(batch.evidence[0].kind, EvidenceKind::DockerCompose);
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "confluentinc/cp-kafka:7.5")
        );
        let topic = batch
            .evidence
            .iter()
            .find(|e| e.channel == "orders.created")
            .expect("topic evidence");
        assert_eq!(topic.broker, "kafka");
        assert_eq!(topic.line, 6);
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "kafka:9092" && e.broker == "kafka")
        );
    }

    #[test]
    fn k8s_env_and_ingress_evidence() {
        let manifest = r#"
apiVersion: networking.k8s.io/v1
kind: Ingress
spec:
  rules:
    - host: api.orders.internal
      http:
        paths:
          - path: /api
spec:
  template:
    spec:
      containers:
        - name: app
          image: acme/orders:1.2.3
          env:
            - name: ORDERS_TOPIC
              value: orders.created
"#;
        let batch = scan_file("k8s/orders.yaml", manifest);
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "acme/orders:1.2.3")
        );
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "api.orders.internal /api"),
            "ingress host+path: {:?}",
            batch.evidence
        );
        let env = batch
            .evidence
            .iter()
            .find(|e| e.channel == "orders.created")
            .expect("env topic");
        assert_eq!(env.detail, "- name: ORDERS_TOPIC");
        assert_eq!(env.line, 17);
        assert!(
            batch
                .evidence
                .iter()
                .all(|e| e.kind == EvidenceKind::Kubernetes)
        );
    }

    #[test]
    fn readme_emits_urls_paths_and_channels() {
        let readme = "# Billing\n\nCall https://api.billing.internal/v1/invoices to list invoices.\nRoutes live under `/api/invoices/{id}`.\nEvents: ORDER_CREATED is published to topic `billing.created`.\n";
        let batch = scan_file("README.md", readme);
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "https://api.billing.internal/v1/invoices")
        );
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.value == "/api/invoices/{id}")
        );
        assert!(
            batch
                .evidence
                .iter()
                .any(|e| e.channel == "billing.created"),
            "got {:?}",
            batch.evidence
        );
        assert!(
            batch
                .evidence
                .iter()
                .all(|e| e.kind == EvidenceKind::Readme)
        );
        assert!(batch.evidence.iter().all(|e| e.line >= 1));
    }

    #[test]
    fn gates_dispatch_to_expected_scanner() {
        assert!(scan_file("src/main.rs", "fn main() {}").is_empty());
        assert!(scan_file("docs/guide.md", "no signals here").is_empty());
        assert!(scan_file("chart/Chart.yaml", "name: x\nversion: 1\n").is_empty());
        assert!(
            scan_file("services/deployment.yaml", "kind: Deployment\n")
                .evidence
                .is_empty()
        );
        assert!(
            scan_file("openapi.yaml", "openapi: 3.0.0\npaths: {}\n")
                .evidence
                .is_empty()
        );
        assert!(scan_file("spec.json", "{\"info\": {}}").is_empty());
    }

    #[test]
    fn scannable_gate_matches_dispatch() {
        assert!(is_scannable("README.md"));
        assert!(is_scannable("docs/runbook.md"));
        assert!(is_scannable("deploy/k8s/app.yaml"));
        assert!(is_scannable(".env"));
        assert!(is_scannable("config/app.env"));
        assert!(is_scannable("application-prod.yml"));
        assert!(is_scannable("openapi.json"));
        assert!(!is_scannable("package.json"));
        assert!(!is_scannable("src/main.rs"));
        assert!(!is_scannable("CHANGELOG.md"));
        assert!(!is_scannable("config.md"));
    }
}
