use ckg_domain::{CanonicalId, CanonicalIdBuilder, ResourceRef};

use crate::contracts::{Extractor, first_quoted, quoted_strings};

/// Connection-string scheme → database mechanism.
const DB_SCHEMES: &[(&str, &str)] = &[
    ("postgres://", "postgres"),
    ("postgresql://", "postgres"),
    ("mysql://", "mysql"),
    ("mariadb://", "mysql"),
    ("mongodb+srv://", "mongodb"),
    ("mongodb://", "mongodb"),
    ("redis://", "redis"),
    ("rediss://", "redis"),
    ("clickhouse://", "clickhouse"),
    ("sqlserver://", "sqlserver"),
    ("mssql://", "sqlserver"),
    ("cockroachdb://", "cockroach"),
    ("amqp://", "rabbitmq"),
    ("amqps://", "rabbitmq"),
    ("sqlite://", "sqlite"),
    ("file:", "sqlite"),
];

impl<'a> Extractor<'a> {
    pub(crate) fn resources(&mut self) {
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            self.literal_resources(lno, line);
            self.env_resources(lno, line);
            match self.language {
                "python" => self.py_resources(lno, line),
                "javascript" | "typescript" => self.js_resources(lno, line),
                "go" => self.go_resources(lno, line),
                "java" | "kotlin" => self.java_resources(lno, line),
                "rust" => self.rust_resources(lno, line),
                "csharp" => self.csharp_resources(lno, line),
                _ => {}
            }
        }
    }

    pub(crate) fn add_resource(
        &mut self,
        line: u32,
        resource_type: &str,
        mechanism: &str,
        access: &str,
        raw_target: &str,
    ) {
        let target = raw_target.trim();
        if target.is_empty() {
            return;
        }
        let node = self
            .enclosing(line)
            .unwrap_or_else(|| self.repository_node_id());
        let access = normalize_access(access);
        let key = format!(
            "r|{resource_type}|{mechanism}|{access}|{target}|{}",
            node.as_str()
        );
        if !self.seen.insert(key) {
            return;
        }
        self.batch.resources.push(ResourceRef {
            file: self.path.to_string(),
            resource_type: resource_type.to_string(),
            mechanism: mechanism.to_string(),
            access,
            raw_target: target.to_string(),
            node_id: node,
            language: self.language.to_string(),
            location: Some(self.loc(line)),
            properties: Default::default(),
        });
    }

    fn repository_node_id(&self) -> CanonicalId {
        CanonicalIdBuilder::new()
            .repository(self.repository)
            .symbol("repo")
            .build()
    }

    fn file_has(&mut self, markers: &[&str]) -> bool {
        markers.iter().any(|marker| {
            if let Some(&hit) = self.file_flags.get(*marker) {
                return hit;
            }
            let hit = self.lower.contains(marker);
            self.file_flags.insert((*marker).to_string(), hit);
            hit
        })
    }

    // ------------------------------------------------------------ literals

    fn literal_resources(&mut self, lno: u32, line: &str) {
        let low_line = line.to_ascii_lowercase();
        for value in quoted_strings(line) {
            self.classify_literal(lno, &low_line, &value);
        }
    }

    fn classify_literal(&mut self, lno: u32, low_line: &str, value: &str) {
        let raw = value.trim();
        if raw.is_empty() {
            return;
        }
        let low = raw.to_ascii_lowercase();

        for (scheme, mechanism) in DB_SCHEMES {
            if low.starts_with(scheme) {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "database", mechanism, &access, raw);
                return;
            }
        }

        if let Some(rest) = low.strip_prefix("jdbc:") {
            let mechanism = if rest.starts_with("postgresql") {
                "postgres"
            } else if rest.starts_with("mysql") || rest.starts_with("mariadb") {
                "mysql"
            } else if rest.starts_with("oracle") {
                "oracle"
            } else if rest.starts_with("sqlserver") || rest.starts_with("jtds") {
                "sqlserver"
            } else {
                "jdbc"
            };
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "database", mechanism, &access, raw);
            return;
        }

        if low.starts_with("s3://") {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "bucket", "s3", &access, raw);
            return;
        }
        if low.starts_with("gs://") {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "bucket", "gcs", &access, raw);
            return;
        }
        if low.starts_with("azblob://") {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "bucket", "azure_blob", &access, raw);
            return;
        }

        if low.starts_with("arn:aws:") {
            self.classify_arn(lno, low_line, &low, raw);
            return;
        }

        if low.starts_with("http://") || low.starts_with("https://") {
            self.classify_http_host(lno, low_line, &low, raw);
            return;
        }

        if low.contains("host=")
            && (low.contains("dbname=") || low.contains("database=") || low.contains("db="))
        {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "database", "sql", &access, raw);
            return;
        }

        if low_line.contains("bigquery")
            && looks_like_dataset_ref(&low)
            && raw.matches('.').count() == 1
        {
            let access = infer_access(low_line, "query");
            self.add_resource(lno, "dataset", "bigquery", &access, raw);
        }
    }

    fn classify_arn(&mut self, lno: u32, low_line: &str, low: &str, raw: &str) {
        let rest = &low["arn:aws:".len()..];
        let parts: Vec<&str> = rest.split(':').collect();
        let Some(service) = parts.first().copied() else {
            return;
        };
        match service {
            "lambda" => {
                let name = parts
                    .iter()
                    .rev()
                    .find(|p| !p.is_empty())
                    .copied()
                    .unwrap_or("");
                if !name.is_empty() {
                    let access = infer_access(low_line, "invoke");
                    self.add_resource(lno, "function", "lambda", &access, raw);
                }
            }
            "s3" => {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "bucket", "s3", &access, raw);
            }
            "dynamodb" => {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "table", "dynamodb", &access, raw);
            }
            _ => {}
        }
    }

    fn classify_http_host(&mut self, lno: u32, low_line: &str, low: &str, raw: &str) {
        let after = &low[low.find("://").map(|i| i + 3).unwrap_or(0)..];
        let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let host = &after[..host_end];
        let path = &after[host_end..];
        let first_seg = path
            .trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");

        if host == "storage.googleapis.com" || host.ends_with(".storage.googleapis.com") {
            if !first_seg.is_empty() {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "bucket", "gcs", &access, raw);
            }
            return;
        }
        if host == "s3.amazonaws.com" {
            if !first_seg.is_empty() {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "bucket", "s3", &access, raw);
            }
            return;
        }
        if host.ends_with(".amazonaws.com") && host.contains(".s3.") {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "bucket", "s3", &access, raw);
            return;
        }
        if let Some(prefix) = host.strip_suffix(".s3.amazonaws.com") {
            let bucket = prefix.split('.').next().unwrap_or("");
            if !bucket.is_empty() {
                let access = infer_access(low_line, "connect");
                self.add_resource(lno, "bucket", "s3", &access, raw);
            }
            return;
        }
        if host.ends_with(".cloudfunctions.net") && !first_seg.is_empty() {
            let access = infer_access(low_line, "invoke");
            self.add_resource(lno, "function", "cloud_function", &access, raw);
            return;
        }
        if host.ends_with(".run.app") && !first_seg.is_empty() {
            let access = infer_access(low_line, "invoke");
            self.add_resource(lno, "function", "cloud_run", &access, raw);
            return;
        }
        if host.contains(".execute-api.") && host.ends_with(".amazonaws.com") {
            let access = infer_access(low_line, "connect");
            self.add_resource(lno, "api", "aws_api_gateway", &access, raw);
        }
    }

    // ---------------------------------------------------------- env refs

    fn env_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();
        let envish = low.contains("environ")
            || low.contains("getenv")
            || low.contains("process.env")
            || low.contains("system.getenv")
            || low.contains("${")
            || low.contains("$(");
        if !envish {
            return;
        }
        for var in extract_env_vars(line) {
            let Some(resource_type) = env_var_kind(&var) else {
                continue;
            };
            let raw = format!("env:{var}");
            self.add_resource(lno, resource_type, "", "connect", &raw);
        }
        for value in quoted_strings(line) {
            let name = value.trim();
            if !is_env_name(name) {
                continue;
            }
            let Some(resource_type) = env_var_kind(name) else {
                continue;
            };
            let raw = format!("env:{name}");
            self.add_resource(lno, resource_type, "", "connect", &raw);
        }
    }

    // ---------------------------------------------------------- python

    fn py_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["boto3", "boto.", "boto(", "amazonaws"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "Bucket")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"]) {
            if let Some(target) = arg_quoted(line, "TableName") {
                let access = infer_access(&low, "connect");
                self.add_resource(lno, "table", "dynamodb", &access, &target);
            } else if low.contains("table(")
                && let Some(target) = arg_quoted(line, "table")
            {
                let access = infer_access(&low, "connect");
                self.add_resource(lno, "table", "dynamodb", &access, &target);
            }
        }

        if let Some(target) =
            arg_quoted(line, "FunctionName").or_else(|| arg_quoted(line, "function_name"))
        {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&["google.cloud.storage", "google-cloud-storage"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- javascript

    fn js_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["s3", "aws-sdk", "aws_sdk"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "Bucket")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"])
            && let Some(target) = arg_quoted(line, "TableName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "table", "dynamodb", &access, &target);
        }

        if let Some(target) = arg_quoted(line, "FunctionName") {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&[
            "@google-cloud/storage",
            "google-cloud-storage",
            "@googleapis",
        ]) && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["@google-cloud/bigquery", "bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- go

    fn go_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["aws-sdk-go", "minio-go", "\"s3\"", "s3."])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "Bucket")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"])
            && let Some(target) = arg_quoted(line, "TableName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "table", "dynamodb", &access, &target);
        }

        if self.file_has(&["aws-lambda-go", "lambda."])
            && let Some(target) = arg_quoted(line, "FunctionName")
        {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&[
            "cloud.google.com/go/storage",
            "google.golang.org/api/storage",
        ]) && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["cloud.google.com/go/bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- java

    fn java_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["s3", "amazonaws"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "BucketName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"])
            && let Some(target) = arg_quoted(line, "TableName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "table", "dynamodb", &access, &target);
        }

        if self.file_has(&["aws-lambda", "lambda."])
            && let Some(target) = arg_quoted(line, "FunctionName")
        {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&["google.cloud.storage"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["google.cloud.bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- rust

    fn rust_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["aws_sdk_s3", "aws-sdk-s3", "rusoto", "s3::"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"])
            && let Some(target) = arg_quoted(line, "table")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "table", "dynamodb", &access, &target);
        }

        if self.file_has(&["aws_lambda", "lambda"])
            && let Some(target) = arg_quoted(line, "function_name")
        {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&["google-cloud-storage", "gcp-storage"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- csharp

    fn csharp_resources(&mut self, lno: u32, line: &str) {
        let low = line.to_ascii_lowercase();

        if self.file_has(&["s3", "awssdk"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "BucketName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "bucket", "s3", &access, &target);
        }

        if self.file_has(&["dynamodb"])
            && let Some(target) = arg_quoted(line, "TableName")
        {
            let access = infer_access(&low, "connect");
            self.add_resource(lno, "table", "dynamodb", &access, &target);
        }

        if self.file_has(&["lambda"])
            && let Some(target) = arg_quoted(line, "FunctionName")
        {
            self.add_resource(lno, "function", "lambda", "invoke", &target);
        }

        if self.file_has(&["google.cloud.storage"])
            && low.contains("bucket")
            && let Some(target) = arg_quoted(line, "bucket")
        {
            self.add_resource(lno, "bucket", "gcs", "connect", &target);
        }

        if self.file_has(&["bigquery"]) {
            self.bigquery_resources(lno, line, &low);
        }
    }

    // ---------------------------------------------------------- bigquery

    fn bigquery_resources(&mut self, lno: u32, line: &str, low: &str) {
        if low.contains("tableid") || low.contains("table_id") {
            let quoted = quoted_strings(line);
            if quoted.len() >= 3 {
                let target = format!("{}.{}.{}", quoted[0], quoted[1], quoted[2]);
                self.add_resource(lno, "table", "bigquery", "query", &target);
                return;
            }
        }
        if low.contains("dataset") {
            let quoted = quoted_strings(line);
            if quoted.len() >= 2 && (low.contains("datasetid") || low.contains("dataset_id")) {
                let target = format!("{}.{}", quoted[0], quoted[1]);
                self.add_resource(lno, "dataset", "bigquery", "query", &target);
                return;
            }
            if let Some(first) = quoted.into_iter().next() {
                self.add_resource(lno, "dataset", "bigquery", "query", &first);
                return;
            }
        }
        if low.contains(".query(") || low.contains(".query (") {
            for value in quoted_strings(line) {
                if let Some(target) = bigquery_table_ref(&value) {
                    self.add_resource(lno, "table", "bigquery", "query", &target);
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- helpers

fn normalize_access(access: &str) -> String {
    let access = access.trim().to_ascii_lowercase();
    match access.as_str() {
        "read" | "write" | "invoke" | "query" | "connect" => access,
        _ => "connect".to_string(),
    }
}

fn infer_access(low_line: &str, default: &str) -> String {
    let flat: String = low_line
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    const WRITE: &[&str] = &[
        "putobject",
        "putitem",
        "putrecord",
        "putrequest",
        "write",
        "insert",
        "update",
        "delete",
        "upload",
        "overwrite",
        "copy",
        "remove",
        "save",
    ];
    const READ: &[&str] = &[
        "getobject",
        "getitem",
        "getrecord",
        "getrequest",
        "select",
        "read",
        "fetch",
        "download",
        "list",
        "scan",
        "load",
        "head",
        "describe",
        "retrieve",
        "exists",
    ];
    if WRITE.iter().any(|m| flat.contains(m)) {
        return "write".to_string();
    }
    if READ.iter().any(|m| flat.contains(m)) {
        return "read".to_string();
    }
    default.to_string()
}

/// Find `key` in `line` case-insensitively, tolerating `_` differences
/// (`TableName` matches `table_name`), then return its quoted argument.
fn arg_quoted(line: &str, key: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for variant in key_variants(key) {
        let needle = variant.to_ascii_lowercase();
        let Some(idx) = lower.find(&needle) else {
            continue;
        };
        let after = &line[idx + needle.len()..];
        let trimmed = after.trim_start().trim_start_matches('_').trim_start();
        if !(trimmed.starts_with(':') || trimmed.starts_with('=') || trimmed.starts_with('(')) {
            continue;
        }
        if let Some(value) = first_quoted(after) {
            return Some(value);
        }
    }
    None
}

fn key_variants(key: &str) -> Vec<String> {
    let mut variants = vec![key.to_string()];
    let flat = key.replace('_', "");
    if flat != key {
        variants.push(flat);
    }
    let mut snake = String::new();
    for (i, c) in key.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    if snake != key {
        variants.push(snake);
    }
    variants
}

fn extract_env_vars(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && (bytes[i + 1] == b'{' || bytes[i + 1] == b'(')
        {
            let close = if bytes[i + 1] == b'{' { b'}' } else { b')' };
            if let Some(end) = line[i + 2..].find(close as char) {
                let name = line[i + 2..i + 2 + end].trim();
                if is_env_name(name) {
                    out.push(name.to_string());
                }
                i += 2 + end + 1;
                continue;
            }
        }
        if bytes[i] == b'$'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let name = &line[i + 1..j];
            if is_env_name(name) {
                out.push(name.to_string());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && name.chars().any(|c| c.is_ascii_alphabetic())
}

fn env_var_kind(name: &str) -> Option<&'static str> {
    let low = name.to_ascii_lowercase();
    if low.contains("bucket") {
        return Some("bucket");
    }
    if low.contains("function") {
        return Some("function");
    }
    if low.contains("url")
        || low.contains("uri")
        || low.contains("dsn")
        || low.contains("database")
        || low.contains("db_")
        || low.ends_with("_db")
        || low.contains("host")
        || low.contains("sql")
        || low.contains("mongo")
        || low.contains("redis")
    {
        return Some("database");
    }
    if low.contains("dataset") {
        return Some("dataset");
    }
    None
}

fn looks_like_dataset_ref(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 2 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && p.chars().any(|c| c.is_ascii_alphabetic())
    })
}

fn bigquery_table_ref(value: &str) -> Option<String> {
    if let Some(open) = value.find('`') {
        let rest = &value[open + 1..];
        let close = rest.find('`')?;
        return as_table_ref(&rest[..close]);
    }
    as_table_ref(value.trim())
}

fn as_table_ref(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let ok = parts.iter().all(|p| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    });
    ok.then(|| parts.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "acme/svc";
    const SHA: &str = "abc123";

    fn resources(language: &str, path: &str, source: &str) -> Vec<ResourceRef> {
        crate::contracts_for_test(REPO, SHA, path, source, language).resources
    }

    #[test]
    fn dsn_literals_extract_database_resources() {
        let py = resources(
            "python",
            "app.py",
            r#"
engine = create_engine("postgresql://orders:pw@db.internal:5432/orders")
redis = redis.from_url("rediss://cache.internal:6380/0")
legacy = "host=db.internal dbname=reports user=reporter"
"#,
        );
        assert!(py.iter().any(|r| r.raw_target.starts_with("postgresql://")
            && r.resource_type == "database"
            && r.mechanism == "postgres"));
        assert!(
            py.iter()
                .any(|r| r.mechanism == "redis" && r.resource_type == "database"),
            "redis dsn: {:?}",
            py
        );
        assert!(
            py.iter().any(|r| r.raw_target.contains("dbname=reports")),
            "keyword dsn: {:?}",
            py
        );
    }

    #[test]
    fn jdbc_and_s3_gs_literals_extract_resources() {
        let java = resources(
            "java",
            "Repo.java",
            r#"
String url = "jdbc:postgresql://db.internal:5432/app";
String obj = "s3://my-bucket/reports/part-001.parquet";
String gs = "gs://team-bucket/data.jsonl";
"#,
        );
        assert!(
            java.iter()
                .any(|r| r.resource_type == "database" && r.raw_target.starts_with("jdbc:"))
        );
        assert!(
            java.iter()
                .any(|r| r.resource_type == "bucket" && r.mechanism == "s3")
        );
        assert!(
            java.iter()
                .any(|r| r.resource_type == "bucket" && r.mechanism == "gcs")
        );
    }

    #[test]
    fn boto3_bucket_and_dynamodb_table_access_inference() {
        let py = resources(
            "python",
            "sync.py",
            r#"
import boto3
s3 = boto3.client("s3")
s3.put_object(Bucket="orders-exports", Key="a.json", Body=b"x")
s3.get_object(Bucket="orders-exports", Key="a.json")
ddb = boto3.resource("dynamodb")
ddb.Table("orders").put_item(Item={"id": "1"})
ddb.Table("orders").get_item(Key={"id": "1"})
"#,
        );
        let writes: Vec<_> = py
            .iter()
            .filter(|r| r.access == "write" && r.mechanism == "s3")
            .collect();
        assert_eq!(writes.len(), 1, "one s3 write: {:?}", py);
        assert_eq!(writes[0].raw_target, "orders-exports");
        let reads = py
            .iter()
            .filter(|r| r.access == "read" && r.mechanism == "s3")
            .count();
        assert_eq!(reads, 1, "one s3 read: {:?}", py);
        let tables: Vec<_> = py
            .iter()
            .filter(|r| r.resource_type == "table" && r.raw_target == "orders")
            .collect();
        assert_eq!(tables.len(), 2, "table deduped by access: {:?}", py);
        assert!(tables.iter().any(|r| r.access == "write"));
        assert!(tables.iter().any(|r| r.access == "read"));
    }

    #[test]
    fn function_name_patterns_extract_invokes() {
        let js = resources(
            "javascript",
            "jobs.js",
            r#"
const { LambdaClient, InvokeCommand } = require("@aws-sdk/client-lambda");
const client = new LambdaClient({});
await client.send(new InvokeCommand({ FunctionName: "billing-worker", Payload: p }));
"#,
        );
        assert!(
            js.iter().any(|r| r.resource_type == "function"
                && r.access == "invoke"
                && r.raw_target == "billing-worker"),
            "js lambda: {:?}",
            js
        );

        let py = resources(
            "python",
            "invoke.py",
            r#"
import boto3
client = boto3.client("lambda")
client.invoke(FunctionName="billing-worker", Payload=b"{}")
"#,
        );
        assert!(
            py.iter()
                .any(|r| r.resource_type == "function" && r.raw_target == "billing-worker"),
            "py lambda: {:?}",
            py
        );
    }

    #[test]
    fn bigquery_datasets_and_query_tables_extract() {
        let java = resources(
            "java",
            "Bq.java",
            r#"
import com.google.cloud.bigquery.BigQuery;
DatasetId id = DatasetId.of("analytics-proj", "orders");
TableId tid = TableId.of("analytics-proj", "orders", "events");
"#,
        );
        assert!(
            java.iter()
                .any(|r| r.resource_type == "dataset" && r.raw_target == "analytics-proj.orders"),
            "java dataset: {:?}",
            java
        );
        assert!(
            java.iter()
                .any(|r| r.resource_type == "table"
                    && r.raw_target == "analytics-proj.orders.events"),
            "java table: {:?}",
            java
        );

        let py = resources(
            "python",
            "bq.py",
            r#"
from google.cloud import bigquery
client = bigquery.Client()
job = client.query("SELECT * FROM `analytics-proj.orders.events` WHERE x = 1")
"#,
        );
        assert!(
            py.iter().any(|r| r.resource_type == "table"
                && r.mechanism == "bigquery"
                && r.access == "query"),
            "py bigquery query table: {:?}",
            py
        );
    }

    #[test]
    fn env_var_references_hint_resource_kinds() {
        let py = resources(
            "python",
            "config.py",
            r#"
import os
url = os.environ["DATABASE_URL"]
bucket = os.environ.get("EXPORT_BUCKET")
fn = os.getenv("WORKER_FUNCTION_NAME")
home = os.environ.get("HOME")
"#,
        );
        assert!(
            py.iter()
                .any(|r| r.raw_target == "env:DATABASE_URL" && r.resource_type == "database"),
            "database env: {:?}",
            py
        );
        assert!(
            py.iter()
                .any(|r| r.raw_target == "env:EXPORT_BUCKET" && r.resource_type == "bucket"),
            "bucket env: {:?}",
            py
        );
        assert!(
            py.iter().any(
                |r| r.raw_target == "env:WORKER_FUNCTION_NAME" && r.resource_type == "function"
            ),
            "function env: {:?}",
            py
        );
        assert!(
            !py.iter().any(|r| r.raw_target.contains("HOME")),
            "HOME is not a resource: {:?}",
            py
        );
    }

    #[test]
    fn cloud_function_and_run_urls_extract_functions() {
        let ts = resources(
            "typescript",
            "client.ts",
            r#"
const res = await fetch("https://us-central1-acme.cloudfunctions.net/billing-fn", { method: "POST" });
const run = await fetch("https://checkout-abc.a.run.app/api/pay");
"#,
        );
        assert!(
            ts.iter()
                .any(|r| r.resource_type == "function" && r.mechanism == "cloud_function"),
            "cloud function url: {:?}",
            ts
        );
        assert!(
            ts.iter()
                .any(|r| r.resource_type == "function" && r.mechanism == "cloud_run"),
            "cloud run url: {:?}",
            ts
        );
    }

    #[test]
    fn arns_classify_by_service() {
        let rs = resources(
            "rust",
            "src/main.rs",
            r#"
let fn_arn = "arn:aws:lambda:us-east-1:123456789012:function:billing-worker";
let tbl = "arn:aws:dynamodb:us-east-1:123456789012:table/orders";
let obj = "s3://arn-bucket/x";
"#,
        );
        assert!(
            rs.iter().any(
                |r| r.resource_type == "function" && r.raw_target.starts_with("arn:aws:lambda")
            ),
            "lambda arn: {:?}",
            rs
        );
        assert!(
            rs.iter().any(|r| r.resource_type == "table"
                && r.raw_target.starts_with("arn:aws:dynamodb")),
            "dynamodb arn: {:?}",
            rs
        );
        assert!(
            rs.iter().any(|r| r.resource_type == "bucket"),
            "s3 url: {:?}",
            rs
        );
    }

    #[test]
    fn module_level_reference_falls_back_to_repository_node() {
        let js = resources(
            "javascript",
            "db.js",
            r#"
const { Pool } = require("pg");
const pool = new Pool({ connectionString: "postgres://app@db.internal:5432/app" });
"#,
        );
        let expected = CanonicalIdBuilder::new()
            .repository(REPO)
            .symbol("repo")
            .build();
        let rec = js
            .iter()
            .find(|r| r.raw_target.starts_with("postgres://"))
            .expect("dsn record");
        assert_eq!(
            rec.node_id, expected,
            "top-level code falls back to repo node"
        );
    }

    #[test]
    fn enclosing_function_attaches_resource_to_symbol() {
        let go = resources(
            "go",
            "store.go",
            r#"
package store

func Export() {
	cfg := &s3.PutObjectInput{Bucket: aws.String("orders-exports"), Key: aws.String("k")}
	_ = cfg
}
"#,
        );
        let rec = go
            .iter()
            .find(|r| r.raw_target == "orders-exports")
            .expect("bucket record");
        assert_ne!(
            rec.node_id,
            CanonicalIdBuilder::new()
                .repository(REPO)
                .symbol("repo")
                .build()
        );
        assert_eq!(rec.access, "write", "PutObject implies write: {rec:?}");
    }

    #[test]
    fn plain_urls_and_env_vars_without_resource_kinds_are_ignored() {
        let js = resources(
            "javascript",
            "app.js",
            r#"
await fetch("https://api.example.com/v1/items", { method: "GET" });
const home = process.env.HOME;
"#,
        );
        assert!(
            js.is_empty(),
            "generic https/HOME must not become resources: {:?}",
            js
        );
    }
}
