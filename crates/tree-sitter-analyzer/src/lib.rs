use std::collections::HashMap;
use std::path::Path;

use ckg_domain::{
    Analyzer, AnalyzerOutput, CanonicalId, CanonicalIdBuilder, NodeKind, ProcessingStatus,
    Provenance, RawRelation, RawSymbol, RelationKind, SourceLocation, content_hash,
};
use tree_sitter::Node;

mod contracts;
mod resources;

#[cfg(test)]
mod treedump;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    JavaScript,
    TypeScript,
    TypeScriptX,
    Python,
    Go,
    Java,
    C,
    Cpp,
    Kotlin,
    Sql,
    Shell,
    Ruby,
    Php,
    CSharp,
    Scala,
    Dart,
    Lua,
    R,
    Julia,
    Elixir,
    Haskell,
    Ocaml,
    Zig,
    Html,
    Css,
    Yaml,
    Json,
    Markdown,
    Toml,
    Proto,
    Terraform,
    Hcl,
    Powershell,
    Make,
    CMake,
    Swift,
}

impl Lang {
    fn as_str(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::JavaScript => "javascript",
            Lang::TypeScript | Lang::TypeScriptX => "typescript",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::Kotlin => "kotlin",
            Lang::Sql => "sql",
            Lang::Shell => "shell",
            Lang::Ruby => "ruby",
            Lang::Php => "php",
            Lang::CSharp => "csharp",
            Lang::Scala => "scala",
            Lang::Dart => "dart",
            Lang::Lua => "lua",
            Lang::R => "r",
            Lang::Julia => "julia",
            Lang::Elixir => "elixir",
            Lang::Haskell => "haskell",
            Lang::Ocaml => "ocaml",
            Lang::Zig => "zig",
            Lang::Html => "html",
            Lang::Css => "css",
            Lang::Yaml => "yaml",
            Lang::Json => "json",
            Lang::Markdown => "markdown",
            Lang::Toml => "toml",
            Lang::Proto => "proto",
            Lang::Terraform => "terraform",
            Lang::Hcl => "hcl",
            Lang::Powershell => "powershell",
            Lang::Make => "make",
            Lang::CMake => "cmake",
            Lang::Swift => "swift",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        match id {
            "rust" => Some(Lang::Rust),
            "javascript" => Some(Lang::JavaScript),
            "typescript" => Some(Lang::TypeScript),
            "python" => Some(Lang::Python),
            "go" => Some(Lang::Go),
            "java" => Some(Lang::Java),
            "c" => Some(Lang::C),
            "cpp" => Some(Lang::Cpp),
            "kotlin" => Some(Lang::Kotlin),
            "sql" => Some(Lang::Sql),
            "shell" => Some(Lang::Shell),
            "ruby" => Some(Lang::Ruby),
            "php" => Some(Lang::Php),
            "csharp" => Some(Lang::CSharp),
            "scala" => Some(Lang::Scala),
            "dart" => Some(Lang::Dart),
            "lua" => Some(Lang::Lua),
            "r" => Some(Lang::R),
            "julia" => Some(Lang::Julia),
            "elixir" => Some(Lang::Elixir),
            "haskell" => Some(Lang::Haskell),
            "ocaml" => Some(Lang::Ocaml),
            "zig" => Some(Lang::Zig),
            "html" => Some(Lang::Html),
            "css" => Some(Lang::Css),
            "yaml" => Some(Lang::Yaml),
            "json" => Some(Lang::Json),
            "markdown" => Some(Lang::Markdown),
            "toml" => Some(Lang::Toml),
            "proto" => Some(Lang::Proto),
            "terraform" => Some(Lang::Terraform),
            "hcl" => Some(Lang::Hcl),
            "powershell" => Some(Lang::Powershell),
            "make" => Some(Lang::Make),
            "cmake" => Some(Lang::CMake),
            "swift" => Some(Lang::Swift),
            _ => None,
        }
    }

    fn from_language(language: &str) -> Option<Self> {
        if language.eq_ignore_ascii_case("tsx") {
            return Some(Lang::TypeScriptX);
        }
        let id = ckg_langs::detect_language_from_name(language)?;
        Self::from_id(id)
    }

    fn from_path(path: &str) -> Option<Self> {
        if Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tsx"))
        {
            return Some(Lang::TypeScriptX);
        }
        let id = ckg_langs::detect_language_from_path(path)?;
        Self::from_id(id)
    }

    fn resolve(path: &str, language: &str) -> Option<Self> {
        if Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tsx"))
        {
            return Some(Lang::TypeScriptX);
        }
        Self::from_language(language).or_else(|| Self::from_path(path))
    }

    fn tree_sitter_language(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::TypeScriptX => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Lang::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Lang::Shell => tree_sitter_bash::LANGUAGE.into(),
            Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Lang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Lang::Scala => tree_sitter_scala::LANGUAGE.into(),
            Lang::Dart => tree_sitter_dart::LANGUAGE.into(),
            Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
            Lang::R => tree_sitter_r::LANGUAGE.into(),
            Lang::Julia => tree_sitter_julia::LANGUAGE.into(),
            Lang::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Lang::Haskell => tree_sitter_haskell::LANGUAGE.into(),
            Lang::Ocaml => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            Lang::Zig => tree_sitter_zig::LANGUAGE.into(),
            Lang::Html => tree_sitter_html::LANGUAGE.into(),
            Lang::Css => tree_sitter_css::LANGUAGE.into(),
            Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Lang::Json => tree_sitter_json::LANGUAGE.into(),
            Lang::Markdown => tree_sitter_md::LANGUAGE.into(),
            Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Lang::Proto => tree_sitter_proto::LANGUAGE.into(),
            Lang::Terraform | Lang::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Lang::Powershell => tree_sitter_powershell::LANGUAGE.into(),
            Lang::Make => tree_sitter_make::LANGUAGE.into(),
            Lang::CMake => tree_sitter_cmake::LANGUAGE.into(),
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
        }
    }
}

pub fn language_for_path(path: &str) -> Option<&'static str> {
    ckg_langs::detect_language_from_path(path)
}

pub fn has_grammar(language: &str) -> bool {
    ckg_langs::has_grammar(language)
}

pub struct TreeSitterAnalyzer {
    version: String,
}

impl TreeSitterAnalyzer {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn analyze(
        &self,
        repository: &str,
        commit_sha: &str,
        path: &str,
        content: &str,
        language: &str,
    ) -> AnalyzerOutput {
        let lang = Lang::resolve(path, language);

        let Some(lang) = lang else {
            #[cfg(feature = "dynamic-grammars")]
            {
                if let Some(output) =
                    self.analyze_dynamic(repository, commit_sha, path, content, language)
                {
                    return output;
                }
            }
            let mut output = AnalyzerOutput::default();
            let known = ckg_langs::detect_language_from_name(language).is_some()
                || ckg_langs::detect_language_from_path(path).is_some();
            if known {
                output
                    .errors
                    .push(format!("no tree-sitter grammar for language: {language}"));
                output
                    .provenance
                    .push(self.base_provenance(ProcessingStatus::Skipped));
            } else {
                output
                    .errors
                    .push(format!("unsupported language: {language}"));
                output
                    .provenance
                    .push(self.base_provenance(ProcessingStatus::Failed));
            }
            return output;
        };

        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang.tree_sitter_language()).is_err() {
            let mut output = AnalyzerOutput::default();
            output
                .errors
                .push(format!("failed to load grammar for {}", lang.as_str()));
            output
                .provenance
                .push(self.base_provenance(ProcessingStatus::Failed));
            return output;
        }

        let tree = match parser.parse(content, None) {
            Some(tree) => tree,
            None => {
                let mut output = AnalyzerOutput::default();
                output.errors.push("parser returned no tree".to_string());
                output
                    .provenance
                    .push(self.base_provenance(ProcessingStatus::Failed));
                return output;
            }
        };

        let mut errors = Vec::new();
        collect_errors(tree.root_node(), content.as_bytes(), path, &mut errors);
        if errors.is_empty() && tree.root_node().has_error() {
            errors.push(format!("{path}: parse errors detected"));
        }

        let status = if errors.is_empty() {
            ProcessingStatus::Complete
        } else {
            ProcessingStatus::Partial
        };
        let namespace = namespace_for_path(path);
        let mut walker = Walker {
            repository,
            commit_sha,
            path,
            content: content.as_bytes(),
            language: Some(lang),
            language_name: lang.as_str(),
            namespace: &namespace,
            provenance: self.base_provenance(status),
            scope: Vec::new(),
            definitions: HashMap::new(),
            symbols: Vec::new(),
            relations: Vec::new(),
            pending_calls: Vec::new(),
        };
        walker.visit(tree.root_node());
        let (symbols, relations) = walker.finish();

        let contracts = contracts::extract_contracts(
            repository,
            commit_sha,
            path,
            content,
            lang.as_str(),
            &symbols,
        );

        let detail = if errors.is_empty() {
            format!("parsed {} with tree-sitter", lang.as_str())
        } else {
            format!(
                "parsed {} with {} parse error(s)",
                lang.as_str(),
                errors.len()
            )
        };

        AnalyzerOutput {
            symbols,
            relations,
            provenance: vec![self.base_provenance(status).with_detail(detail)],
            errors,
            contracts,
        }
    }

    #[cfg(feature = "dynamic-grammars")]
    fn analyze_dynamic(
        &self,
        repository: &str,
        commit_sha: &str,
        path: &str,
        content: &str,
        language: &str,
    ) -> Option<AnalyzerOutput> {
        let id = ckg_langs::detect_language_from_name(language)
            .or_else(|| ckg_langs::detect_language_from_path(path))?;
        let pack_lang = tree_sitter_language_pack::get_language(id).ok()?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&pack_lang).ok()?;
        let tree = parser.parse(content, None)?;
        let mut errors = Vec::new();
        collect_errors(tree.root_node(), content.as_bytes(), path, &mut errors);
        if errors.is_empty() && tree.root_node().has_error() {
            errors.push(format!("{path}: parse errors detected"));
        }
        let status = if errors.is_empty() {
            ProcessingStatus::Complete
        } else {
            ProcessingStatus::Partial
        };
        let namespace = namespace_for_path(path);
        let mut walker = Walker {
            repository,
            commit_sha,
            path,
            content: content.as_bytes(),
            language: None,
            language_name: id,
            namespace: &namespace,
            provenance: self.base_provenance(status),
            scope: Vec::new(),
            definitions: HashMap::new(),
            symbols: Vec::new(),
            relations: Vec::new(),
            pending_calls: Vec::new(),
        };
        walker.visit(tree.root_node());
        let (symbols, relations) = walker.finish();
        let contracts =
            contracts::extract_contracts(repository, commit_sha, path, content, id, &symbols);
        let detail = format!("parsed {id} with dynamic tree-sitter grammar");
        Some(AnalyzerOutput {
            symbols,
            relations,
            provenance: vec![self.base_provenance(status).with_detail(detail)],
            errors,
            contracts,
        })
    }

    fn base_provenance(&self, status: ProcessingStatus) -> Provenance {
        Provenance::tree_sitter(self.version.as_str()).with_status(status)
    }
}

impl Default for TreeSitterAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for TreeSitterAnalyzer {
    fn name(&self) -> &'static str {
        "tree-sitter"
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn analyze_file(
        &self,
        repository: &str,
        commit_sha: &str,
        path: &str,
        content: &str,
        language: &str,
    ) -> AnalyzerOutput {
        self.analyze(repository, commit_sha, path, content, language)
    }
}

fn namespace_for_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let stem = match p.rfind('.') {
        Some(idx) if !p[idx + 1..].contains('/') => &p[..idx],
        _ => p.as_str(),
    };
    stem.trim_start_matches("./")
        .trim_start_matches('/')
        .replace('/', "::")
}

fn qualified_name(
    language: &str,
    repository: &str,
    namespace: &str,
    containers: &[String],
    symbol: &str,
) -> String {
    let mut parts: Vec<&str> = Vec::new();
    parts.push(language);
    if !repository.is_empty() {
        parts.push(repository);
    }
    if !namespace.is_empty() {
        parts.push(namespace);
    }
    for container in containers {
        if !container.is_empty() {
            parts.push(container);
        }
    }
    parts.push(symbol);
    parts.join("::")
}

fn symbol_id(
    repository: &str,
    namespace: &str,
    containers: &[String],
    symbol: &str,
    language: &str,
) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(repository)
        .namespace(namespace)
        .container(containers.join("::"))
        .symbol(symbol)
        .language(language)
        .build()
}

fn file_id(repository: &str, namespace: &str, path: &str, language: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(repository)
        .namespace(namespace)
        .symbol(path)
        .language(language)
        .build()
}

fn import_id(language: &str, import_path: &str) -> CanonicalId {
    CanonicalId::from_parts(&["import", language, import_path])
}

fn location_of(node: Node, repository: &str, commit_sha: &str, path: &str) -> SourceLocation {
    let start = node.start_position();
    let end = node.end_position();
    SourceLocation::new(
        repository,
        commit_sha,
        path,
        start.row as u32 + 1,
        start.column as u32 + 1,
        end.row as u32 + 1,
        end.column as u32 + 1,
    )
}

fn node_hash(node: Node, content: &[u8]) -> String {
    node.utf8_text(content)
        .map(|text| content_hash(text.as_bytes()))
        .unwrap_or_default()
}

fn collect_errors(node: Node, content: &[u8], path: &str, out: &mut Vec<String>) {
    let pos = node.start_position();
    if node.is_error() {
        let snippet = node
            .utf8_text(content)
            .map(|t| t.chars().take(80).collect::<String>())
            .unwrap_or_default();
        out.push(format!(
            "{}:{}:{}: syntax error near {:?}",
            path,
            pos.row + 1,
            pos.column + 1,
            snippet
        ));
    } else if node.is_missing() {
        out.push(format!(
            "{}:{}:{}: missing {}",
            path,
            pos.row + 1,
            pos.column + 1,
            node.kind()
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_errors(child, content, path, out);
    }
}

fn strip_generics(expr: &str) -> String {
    let mut out = String::new();
    let mut depth: i32 = 0;
    for c in expr.chars() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth < 0 {
                    depth = 0;
                }
            }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn base_name(expr: &str) -> String {
    let stripped = strip_generics(expr);
    let trimmed = stripped.trim().trim_end_matches([':', '.', ' ']);
    trimmed
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn strip_string_literal(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn strip_include_path(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>')
        .to_string()
}

fn field_text(node: Node, field: &str, content: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|n| n.utf8_text(content).ok())
        .map(|s| s.to_string())
}

fn first_named_text_of_kinds(node: Node, content: &[u8], kinds: &[&str]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if kinds.contains(&child.kind())
            && let Ok(text) = child.utf8_text(content)
        {
            return Some(text.to_string());
        }
    }
    None
}

fn declarator_name(node: Node, content: &[u8]) -> Option<String> {
    let mut current = node;
    for _ in 0..24 {
        if let Ok(text) = current.utf8_text(content) {
            match current.kind() {
                "identifier"
                | "field_identifier"
                | "type_identifier"
                | "namespace_identifier"
                | "destructor_name"
                | "operator_name"
                | "qualified_identifier"
                | "template_function"
                | "template_method"
                | "dependent_name" => return Some(text.to_string()),
                _ => {}
            }
        }
        if let Some(next) = current.child_by_field_name("declarator") {
            current = next;
            continue;
        }
        if let Some(next) = current.child_by_field_name("name") {
            current = next;
            continue;
        }
        let mut cursor = current.walk();
        if let Some(child) = current.named_children(&mut cursor).next()
            && matches!(
                child.kind(),
                "identifier" | "field_identifier" | "type_identifier" | "qualified_identifier"
            )
        {
            return child.utf8_text(content).ok().map(|s| s.to_string());
        }
        return None;
    }
    None
}

fn is_function_declarator(mut node: Node) -> bool {
    for _ in 0..24 {
        match node.kind() {
            "function_declarator" | "abstract_function_declarator" => return true,
            "identifier" | "field_identifier" | "type_identifier" => return false,
            _ => {}
        }
        match node.child_by_field_name("declarator") {
            Some(next) => node = next,
            None => return false,
        }
    }
    false
}

fn object_reference_name(node: Node, content: &[u8]) -> Option<String> {
    if let Some(name) = field_text(node, "name", content) {
        if let Some(schema) = field_text(node, "schema", content) {
            if let Some(db) = field_text(node, "database", content) {
                return Some(format!("{db}.{schema}.{name}"));
            }
            return Some(format!("{schema}.{name}"));
        }
        return Some(name);
    }
    node.utf8_text(content).ok().map(|s| s.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Type,
    Module,
    Function,
}

struct Scope {
    name: String,
    id: CanonicalId,
    kind: ScopeKind,
}

struct Definition {
    container: String,
    id: CanonicalId,
}

struct PendingCall {
    from: CanonicalId,
    callee: String,
    containers: Vec<String>,
}

struct Walker<'a> {
    repository: &'a str,
    commit_sha: &'a str,
    path: &'a str,
    content: &'a [u8],
    language: Option<Lang>,
    language_name: &'static str,
    namespace: &'a str,
    provenance: Provenance,
    scope: Vec<Scope>,
    definitions: HashMap<String, Vec<Definition>>,
    symbols: Vec<RawSymbol>,
    relations: Vec<RawRelation>,
    pending_calls: Vec<PendingCall>,
}

impl<'a> Walker<'a> {
    fn language_str(&self) -> &'static str {
        self.language_name
    }

    fn container_names(&self) -> Vec<String> {
        self.scope
            .iter()
            .filter(|scope| scope.kind != ScopeKind::Function && !scope.name.is_empty())
            .map(|scope| scope.name.clone())
            .collect()
    }

    fn scope_from(&self) -> CanonicalId {
        self.scope.last().map(|s| s.id.clone()).unwrap_or_else(|| {
            file_id(
                self.repository,
                self.namespace,
                self.path,
                self.language_str(),
            )
        })
    }

    fn finish(self) -> (Vec<RawSymbol>, Vec<RawRelation>) {
        let mut calls: Vec<RawRelation> = Vec::new();
        for pending in &self.pending_calls {
            let name = base_name(&pending.callee);
            if name.is_empty() {
                continue;
            }
            let resolved = self.resolve(&name, &pending.containers);
            let (to, status) = match &resolved {
                Some(id) => (id.clone(), "local"),
                None => (
                    symbol_id(
                        self.repository,
                        self.namespace,
                        &pending.containers,
                        &name,
                        self.language_str(),
                    ),
                    "unresolved",
                ),
            };
            let mut properties = serde_json::Map::new();
            properties.insert(
                "callee".to_string(),
                serde_json::Value::String(pending.callee.clone()),
            );
            properties.insert(
                "resolved".to_string(),
                serde_json::Value::String(status.to_string()),
            );
            calls.push(RawRelation {
                kind: RelationKind::Calls,
                from: pending.from.clone(),
                to,
                properties,
                provenance: self.provenance.clone(),
            });
        }
        let mut relations = self.relations;
        relations.extend(calls);
        (self.symbols, relations)
    }

    fn resolve(&self, name: &str, containers: &[String]) -> Option<CanonicalId> {
        if name.is_empty() {
            return None;
        }
        let definitions = self.definitions.get(name)?;
        let container_key = containers.join("::");
        definitions
            .iter()
            .find(|d| d.container == container_key)
            .or_else(|| definitions.first())
            .map(|d| d.id.clone())
    }

    fn emit_symbol(
        &mut self,
        node: Node,
        kind: NodeKind,
        name: &str,
        containers: &[String],
        extra: Option<(&str, serde_json::Value)>,
    ) -> CanonicalId {
        let id = symbol_id(
            self.repository,
            self.namespace,
            containers,
            name,
            self.language_str(),
        );
        let mut properties = serde_json::Map::new();
        if let Some((key, value)) = extra {
            properties.insert(key.to_string(), value);
        }
        self.symbols.push(RawSymbol {
            canonical_id: id.clone(),
            kind,
            name: name.to_string(),
            qualified_name: qualified_name(
                self.language_str(),
                self.repository,
                self.namespace,
                containers,
                name,
            ),
            language: Some(self.language_str().to_string()),
            location: Some(location_of(
                node,
                self.repository,
                self.commit_sha,
                self.path,
            )),
            content_hash: Some(node_hash(node, self.content)),
            properties,
            provenance: self.provenance.clone(),
        });
        if matches!(kind, NodeKind::Function | NodeKind::Method) {
            self.definitions
                .entry(name.to_string())
                .or_default()
                .push(Definition {
                    container: containers.join("::"),
                    id: id.clone(),
                });
        }
        id
    }

    fn emit_import(&mut self, import_path: &str) {
        if import_path.is_empty() {
            return;
        }
        let from = self.scope_from();
        let mut properties = serde_json::Map::new();
        properties.insert(
            "import_path".to_string(),
            serde_json::Value::String(import_path.to_string()),
        );
        self.relations.push(RawRelation {
            kind: RelationKind::Imports,
            from,
            to: import_id(self.language_str(), import_path),
            properties,
            provenance: self.provenance.clone(),
        });
    }

    fn record_call(&mut self, callee: &str) {
        let containers = self.container_names();
        let from = self.scope_from();
        self.pending_calls.push(PendingCall {
            from,
            callee: callee.to_string(),
            containers,
        });
    }

    fn is_in_type_scope(&self) -> bool {
        self.scope.last().map(|s| s.kind) == Some(ScopeKind::Type)
    }

    fn function_or_method(&self) -> NodeKind {
        if self.is_in_type_scope() {
            NodeKind::Method
        } else {
            NodeKind::Function
        }
    }

    fn push_named_scope(
        &mut self,
        node: Node,
        kind: NodeKind,
        name: String,
        scope_kind: ScopeKind,
    ) -> Option<Scope> {
        let containers = self.container_names();
        let id = self.emit_symbol(node, kind, &name, &containers, None);
        Some(Scope {
            name,
            id,
            kind: scope_kind,
        })
    }

    fn visit(&mut self, node: Node) {
        let kind = node.kind().to_string();
        let mut push: Option<Scope> = None;
        let lang = self.language;

        match (lang, kind.as_str()) {
            (Some(Lang::Rust), "function_item" | "function_signature_item") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Rust), "struct_item" | "enum_item") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Rust), "trait_item") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Rust), "mod_item") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Module, name, ScopeKind::Module);
                }
            }
            (Some(Lang::Rust), "impl_item") => {
                let type_name = field_text(node, "type", self.content)
                    .map(|t| base_name(&t))
                    .unwrap_or_default();
                if !type_name.is_empty() {
                    let containers = self.container_names();
                    let id = self.emit_symbol(
                        node,
                        NodeKind::Class,
                        &type_name,
                        &containers,
                        Some(("impl_block", serde_json::Value::Bool(true))),
                    );
                    push = Some(Scope {
                        name: type_name,
                        id,
                        kind: ScopeKind::Type,
                    });
                } else {
                    push = Some(Scope {
                        name: String::new(),
                        id: self.scope_from(),
                        kind: ScopeKind::Type,
                    });
                }
            }
            (Some(Lang::Rust), "use_declaration") => {
                if let Some(text) = field_text(node, "argument", self.content) {
                    let normalized = text.replace(" as ", " ");
                    self.emit_import(normalized.trim());
                }
            }
            (Some(Lang::Rust), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (Some(Lang::Python), "class_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Python), "function_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Python), "import_statement") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "dotted_name" => {
                            if let Ok(text) = child.utf8_text(self.content) {
                                self.emit_import(text);
                            }
                        }
                        "aliased_import" => {
                            if let Some(text) = field_text(child, "name", self.content) {
                                self.emit_import(&text);
                            }
                        }
                        _ => {}
                    }
                }
            }
            (Some(Lang::Python), "import_from_statement") => {
                let module = field_text(node, "module_name", self.content).unwrap_or_default();
                let mut cursor = node.walk();
                let mut names: Vec<String> = Vec::new();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "dotted_name" => {
                            if let Ok(text) = child.utf8_text(self.content) {
                                names.push(text.to_string());
                            }
                        }
                        "aliased_import" => {
                            if let Some(text) = field_text(child, "name", self.content) {
                                names.push(text);
                            }
                        }
                        "wildcard_import" => {}
                        _ => {}
                    }
                }
                if module.is_empty() {
                    for name in names {
                        self.emit_import(&name);
                    }
                } else if names.is_empty() {
                    self.emit_import(&module);
                } else {
                    for name in names {
                        self.emit_import(&format!("{module}.{name}"));
                    }
                }
            }
            (Some(Lang::Python), "call") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (
                Some(Lang::JavaScript | Lang::TypeScript),
                "function_declaration" | "generator_function_declaration",
            ) => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::JavaScript | Lang::TypeScript), "method_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::JavaScript | Lang::TypeScript), "class_declaration" | "class") => {
                let name = field_text(node, "name", self.content);
                match &name {
                    Some(name) => {
                        push = self.push_named_scope(
                            node,
                            NodeKind::Class,
                            name.clone(),
                            ScopeKind::Type,
                        );
                    }
                    None => {
                        push = Some(Scope {
                            name: String::new(),
                            id: self.scope_from(),
                            kind: ScopeKind::Type,
                        });
                    }
                }
            }
            (Some(Lang::TypeScript), "interface_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::TypeScript), "enum_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::TypeScript), "module" | "internal_module") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Module, name, ScopeKind::Module);
                }
            }
            (Some(Lang::JavaScript | Lang::TypeScript), "import_statement") => {
                if let Some(text) = field_text(node, "source", self.content) {
                    self.emit_import(&strip_string_literal(&text));
                }
            }
            (Some(Lang::JavaScript | Lang::TypeScript), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }
            (Some(Lang::JavaScript | Lang::TypeScript), "variable_declarator") => {
                let is_function_value = field_text(node, "value", self.content).is_some()
                    && node
                        .child_by_field_name("value")
                        .map(|v| matches!(v.kind(), "arrow_function" | "function_expression"))
                        .unwrap_or(false);
                if is_function_value {
                    if let Some(name) = field_text(node, "name", self.content) {
                        let containers = self.container_names();
                        let id =
                            self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }

            (Some(Lang::Go), "source_file") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "package_clause" {
                        continue;
                    }
                    if let Some(pkg) = child
                        .named_child(0)
                        .and_then(|n| n.utf8_text(self.content).ok())
                    {
                        let containers = self.container_names();
                        let id = self.emit_symbol(child, NodeKind::Package, pkg, &containers, None);
                        push = Some(Scope {
                            name: pkg.to_string(),
                            id,
                            kind: ScopeKind::Module,
                        });
                    }
                    break;
                }
            }
            (Some(Lang::Go), "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Go), "method_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let mut containers = self.container_names();
                    if let Some(receiver) =
                        field_text(node, "receiver", self.content).and_then(|r| receiver_type(&r))
                    {
                        containers.push(receiver);
                    }
                    let id = self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Go), "type_spec") => {
                let node_kind = match field_text(node, "type", self.content) {
                    Some(type_text) if type_text.starts_with("interface") => {
                        Some(NodeKind::Interface)
                    }
                    Some(type_text) if type_text.starts_with("struct") => Some(NodeKind::Class),
                    _ => None,
                };
                if let (Some(node_kind), Some(name)) =
                    (node_kind, field_text(node, "name", self.content))
                {
                    push = self.push_named_scope(node, node_kind, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Go), "import_spec") => {
                let path_text = field_text(node, "path", self.content)
                    .map(|t| strip_string_literal(&t))
                    .filter(|t| !t.is_empty());
                if let Some(path_text) = path_text {
                    self.emit_import(&path_text);
                }
            }
            (Some(Lang::Go), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (Some(Lang::Java), "package_declaration") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["identifier", "scoped_identifier"],
                ) {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::Java), "class_declaration" | "enum_declaration" | "record_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Java), "interface_declaration" | "annotation_type_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (
                Some(Lang::Java),
                "method_declaration"
                | "constructor_declaration"
                | "compact_constructor_declaration",
            ) => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Java), "import_declaration") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["identifier", "scoped_identifier"],
                ) {
                    let normalized = text.replace(".static ", "::").replace(" static ", "::");
                    self.emit_import(normalized.trim());
                }
            }
            (Some(Lang::Java), "method_invocation") => {
                let name = field_text(node, "name", self.content);
                let object = field_text(node, "object", self.content);
                let callee = match (object, name) {
                    (Some(obj), Some(name)) => format!("{obj}.{name}"),
                    (None, Some(name)) => name,
                    (Some(obj), None) => obj,
                    (None, None) => String::new(),
                };
                if !callee.is_empty() {
                    self.record_call(&callee);
                }
            }
            (Some(Lang::Java), "object_creation_expression") => {
                if let Some(type_text) = field_text(node, "type", self.content) {
                    self.record_call(&type_text);
                }
            }

            (Some(Lang::C | Lang::Cpp), "function_definition") => {
                if let Some(decl) = node.child_by_field_name("declarator")
                    && let Some(name) = declarator_name(decl, self.content)
                {
                    let name = base_name(&name);
                    if !name.is_empty() {
                        let node_kind = self.function_or_method();
                        let containers = self.container_names();
                        let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (
                Some(Lang::C | Lang::Cpp),
                "struct_specifier" | "union_specifier" | "enum_specifier",
            ) => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::C | Lang::Cpp), "preproc_include") => {
                if let Some(path_text) = field_text(node, "path", self.content) {
                    self.emit_import(&strip_include_path(&path_text));
                }
            }
            (Some(Lang::C | Lang::Cpp), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (Some(Lang::Cpp), "namespace_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Module, name, ScopeKind::Module);
                }
            }
            (Some(Lang::Cpp), "class_specifier") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Cpp), "using_declaration") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["identifier", "qualified_identifier"],
                ) {
                    self.emit_import(&text);
                }
            }
            (Some(Lang::Cpp), "field_declaration") => {
                if self.is_in_type_scope()
                    && let Some(decl) = node.child_by_field_name("declarator")
                    && is_function_declarator(decl)
                    && let Some(name) = declarator_name(decl, self.content)
                {
                    let name = base_name(&name);
                    if !name.is_empty() {
                        let containers = self.container_names();
                        self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                    }
                }
            }

            (Some(Lang::Kotlin), "package_header") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["qualified_identifier", "identifier"],
                ) {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::Kotlin), "class_declaration" | "object_declaration") => {
                let is_interface = {
                    let mut cursor = node.walk();
                    node.children(&mut cursor).any(|c| c.kind() == "interface")
                };
                if let Some(name) = field_text(node, "name", self.content) {
                    let kind = if is_interface {
                        NodeKind::Interface
                    } else {
                        NodeKind::Class
                    };
                    push = self.push_named_scope(node, kind, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Kotlin), "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Kotlin), "import") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["qualified_identifier", "identifier"],
                ) {
                    self.emit_import(&text);
                }
            }
            (Some(Lang::Kotlin), "call_expression") => {
                let mut cursor = node.walk();
                if let Some(callee) = node.named_children(&mut cursor).next()
                    && !matches!(callee.kind(), "value_arguments" | "type_arguments")
                    && let Ok(text) = callee.utf8_text(self.content)
                {
                    self.record_call(text);
                }
            }

            (Some(Lang::Sql), "create_table" | "create_view" | "create_materialized_view") => {
                let mut cursor = node.walk();
                let mut name = None;
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "object_reference" {
                        name = object_reference_name(child, self.content);
                        break;
                    }
                }
                if name.is_none() {
                    name = field_text(node, "name", self.content);
                }
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Sql), "create_function" | "function_declaration") => {
                let mut cursor = node.walk();
                let mut name = None;
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "object_reference" {
                        name = object_reference_name(child, self.content);
                        break;
                    }
                }
                if name.is_none() {
                    name = field_text(node, "name", self.content);
                }
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Sql), "create_query") => {
                if let Some(name) = field_text(node, "name", self.content)
                    && !name.is_empty()
                {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Symbol, &name, &containers, None);
                }
            }

            (Some(Lang::Ruby), "method" | "singleton_method") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Ruby), "class") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Ruby), "module") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Module, name, ScopeKind::Module);
                }
            }
            (Some(Lang::Ruby), "call") => {
                let method_name = field_text(node, "method", self.content);
                let receiver = field_text(node, "receiver", self.content);
                if let Some(method_name) = method_name.as_deref() {
                    if matches!(method_name, "require" | "require_relative" | "load")
                        && receiver.is_none()
                        && let Some(args) = node.child_by_field_name("arguments")
                        && let Some(arg0) = args.named_child(0)
                        && let Ok(raw) = arg0.utf8_text(self.content)
                    {
                        self.emit_import(&strip_string_literal(&raw));
                    } else {
                        let callee = match receiver {
                            Some(recv) if !recv.is_empty() => format!("{recv}.{method_name}"),
                            _ => method_name.to_string(),
                        };
                        self.record_call(&callee);
                    }
                }
            }

            (Some(Lang::Ruby), "identifier") => {
                if let Some(parent) = node.parent()
                    && parent.kind() == "body_statement"
                    && let Ok(text) = node.utf8_text(self.content)
                {
                    let text = text.trim();
                    if !text.is_empty() {
                        self.record_call(text);
                    }
                }
            }

            (Some(Lang::Php), "function_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Php), "method_declaration" | "function_static_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Php), "class_declaration" | "enum_declaration" | "trait_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Php), "interface_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Php), "namespace_definition") => {
                if let Some(text) =
                    first_named_text_of_kinds(node, self.content, &["namespace_name"])
                {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::Php), "namespace_use_declaration") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "namespace_use_clause" | "namespace_use_group" => {
                            if let Ok(text) = child.utf8_text(self.content) {
                                let cleaned = text
                                    .split_whitespace()
                                    .filter(|p| !p.is_empty() && *p != "as" && *p != "use")
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                self.emit_import(&cleaned);
                            }
                        }
                        _ => {}
                    }
                }
            }
            (
                Some(Lang::Php),
                "include_expression"
                | "include_once_expression"
                | "require_expression"
                | "require_once_expression",
            ) => {
                let mut cursor = node.walk();
                if let Some(arg) = node.named_children(&mut cursor).next()
                    && let Ok(text) = arg.utf8_text(self.content)
                {
                    self.emit_import(&strip_string_literal(&text));
                }
            }
            (
                Some(Lang::Php),
                "function_call_expression"
                | "member_call_expression"
                | "scoped_call_expression"
                | "nullsafe_member_call_expression",
            ) => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                } else if let (Some(object), Some(name)) = (
                    field_text(node, "object", self.content),
                    field_text(node, "name", self.content),
                ) {
                    self.record_call(&format!("{object}.{name}"));
                } else if let Some(scope) = field_text(node, "scope", self.content)
                    && let Some(name) = field_text(node, "name", self.content)
                {
                    self.record_call(&format!("{scope}::{name}"));
                }
            }
            (Some(Lang::Php), "object_creation_expression") => {
                if let Some(name) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["name", "qualified_name", "relative_name"],
                ) {
                    self.record_call(&name);
                }
            }

            (
                Some(Lang::CSharp),
                "class_declaration" | "struct_declaration" | "enum_declaration"
                | "record_declaration",
            ) => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::CSharp), "interface_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (
                Some(Lang::CSharp),
                "method_declaration" | "constructor_declaration" | "local_function_statement",
            ) => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::CSharp), "namespace_declaration" | "file_scoped_namespace_declaration") => {
                if let Some(text) =
                    first_named_text_of_kinds(node, self.content, &["identifier", "qualified_name"])
                {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::CSharp), "using_directive") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["identifier"]) {
                    self.emit_import(&text);
                }
            }
            (Some(Lang::CSharp), "invocation_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }
            (Some(Lang::CSharp), "object_creation_expression") => {
                if let Some(type_text) = field_text(node, "type", self.content) {
                    self.record_call(&type_text);
                }
            }

            (Some(Lang::Scala), "class_definition" | "object_definition" | "enum_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Scala), "trait_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Scala), "function_definition" | "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let node_kind = self.function_or_method();
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Scala), "type_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Scala), "package_clause") => {
                if let Some(text) =
                    first_named_text_of_kinds(node, self.content, &["package_identifier"])
                {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::Scala), "import_declaration") => {
                if let Ok(text) = node.utf8_text(self.content) {
                    let path = text
                        .trim()
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .to_string();
                    self.emit_import(&path);
                }
            }
            (Some(Lang::Scala), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (Some(Lang::Shell), "function_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Shell), "command") => {
                if let Some(name) = first_named_text_of_kinds(node, self.content, &["command_name"])
                {
                    if matches!(name.as_str(), "source" | ".")
                        && let Some(arg) = node.child_by_field_name("argument")
                        && let Ok(text) = arg.utf8_text(self.content)
                    {
                        self.emit_import(&strip_string_literal(&text));
                    } else {
                        self.record_call(&name);
                    }
                }
            }

            (Some(Lang::Elixir), "call") => {
                let target = field_text(node, "target", self.content).unwrap_or_default();
                let target_base = base_name(&target);
                match target_base.as_str() {
                    "defmodule" | "defprotocol" | "defimpl" | "defmacro" | "def" | "defp"
                    | "defmacrop" | "defguard" | "defguardp" | "defstruct" | "defexception"
                    | "defoverridable" | "defdelegate" | "defmulti" | "defrecord" => {
                        if let Some(args) = child_of_kind(node, "arguments")
                            && let Some(first) = args.named_child(0)
                        {
                            let name = match first.kind() {
                                "call" => field_text(first, "target", self.content)
                                    .or_else(|| {
                                        first
                                            .named_child(0)
                                            .and_then(|n| n.utf8_text(self.content).ok())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_default(),
                                _ => first
                                    .utf8_text(self.content)
                                    .map(|s| s.to_string())
                                    .unwrap_or_default(),
                            };
                            let is_type = matches!(
                                target_base.as_str(),
                                "defmodule"
                                    | "defprotocol"
                                    | "defimpl"
                                    | "defstruct"
                                    | "defexception"
                            );
                            let name = if is_type {
                                name.trim().to_string()
                            } else {
                                base_name(&name)
                            };
                            if !name.is_empty() {
                                if is_type {
                                    let kind = if target_base == "defprotocol" {
                                        NodeKind::Interface
                                    } else {
                                        NodeKind::Class
                                    };
                                    push = self.push_named_scope(node, kind, name, ScopeKind::Type);
                                } else if matches!(
                                    target_base.as_str(),
                                    "def"
                                        | "defp"
                                        | "defmacro"
                                        | "defmacrop"
                                        | "defguard"
                                        | "defguardp"
                                        | "defdelegate"
                                        | "defmulti"
                                        | "defrecord"
                                ) {
                                    let node_kind = self.function_or_method();
                                    let containers = self.container_names();
                                    let id =
                                        self.emit_symbol(node, node_kind, &name, &containers, None);
                                    push = Some(Scope {
                                        name,
                                        id,
                                        kind: ScopeKind::Function,
                                    });
                                }
                            }
                        }
                    }
                    "import" | "require" | "alias" => {
                        if let Some(args) = child_of_kind(node, "arguments")
                            && let Some(first) = args.named_child(0)
                            && let Ok(text) = first.utf8_text(self.content)
                        {
                            self.emit_import(text.trim());
                        }
                    }
                    _ => {
                        if !target.is_empty() {
                            self.record_call(&target);
                        }
                    }
                }
            }

            (Some(Lang::Dart), "class_declaration" | "enum_declaration" | "mixin_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Dart), "extension_declaration" | "extension_type_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Dart), "function_declaration" | "local_function_declaration") => {
                if let Some(name) = dart_signature_name(node, self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Dart), "method_declaration") => {
                if let Some(name) = dart_signature_name(node, self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Dart), "import_specification") => {
                if let Some(uri) = field_text(node, "uri", self.content) {
                    self.emit_import(&strip_string_literal(&uri));
                }
            }
            (Some(Lang::Dart), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }

            (Some(Lang::Lua), "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let name = match name.as_str() {
                        n if n.starts_with("function ") => {
                            n.trim_start_matches("function ").to_string()
                        }
                        other => other.to_string(),
                    };
                    let name = name
                        .rsplit(|c| c == '.' || c == ':')
                        .next()
                        .unwrap_or(&name)
                        .to_string();
                    if !name.is_empty() {
                        let node_kind = self.function_or_method();
                        let containers = self.container_names();
                        let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (Some(Lang::Lua), "function_call") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    self.record_call(&name);
                }
            }

            (Some(Lang::R), "function_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let name = name.trim().to_string();
                    if !name.is_empty() && !name.eq_ignore_ascii_case("function") {
                        let containers = self.container_names();
                        let id =
                            self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (Some(Lang::R), "binary_operator") => {
                let op = field_text(node, "operator", self.content).unwrap_or_default();
                if matches!(op.as_str(), "<-" | "<<-" | "=") {
                    if let (Some(lhs), Some(rhs)) = (
                        field_text(node, "lhs", self.content),
                        node.child_by_field_name("rhs"),
                    ) {
                        let lhs = lhs.trim().to_string();
                        if rhs.kind() == "function_definition" && !lhs.is_empty() {
                            let containers = self.container_names();
                            let id =
                                self.emit_symbol(node, NodeKind::Function, &lhs, &containers, None);
                            push = Some(Scope {
                                name: lhs,
                                id,
                                kind: ScopeKind::Function,
                            });
                        } else if rhs.kind() == "call" && !lhs.is_empty() {
                            if let Some(fn_text) = field_text(rhs, "function", self.content)
                                && fn_text.trim() == "function"
                            {
                                let containers = self.container_names();
                                let id = self.emit_symbol(
                                    node,
                                    NodeKind::Function,
                                    &lhs,
                                    &containers,
                                    None,
                                );
                                push = Some(Scope {
                                    name: lhs,
                                    id,
                                    kind: ScopeKind::Function,
                                });
                            }
                        }
                    }
                }
            }
            (Some(Lang::R), "call") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    if matches!(
                        callee.as_str(),
                        "library"
                            | "require"
                            | "requireNamespace"
                            | "loadNamespace"
                            | "source"
                            | "attachNamespace"
                    ) && let Some(args) = node.child_by_field_name("arguments")
                        && let Some(first) = args.named_child(0)
                        && let Ok(raw) = first.utf8_text(self.content)
                    {
                        self.emit_import(&strip_string_literal(raw.trim()));
                    } else {
                        self.record_call(&callee);
                    }
                }
            }

            (Some(Lang::Html), "element" | "script_element" | "style_element") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "start_tag" {
                        continue;
                    }
                    let mut attr_cursor = child.walk();
                    for attr in child.named_children(&mut attr_cursor) {
                        if attr.kind() != "attribute" {
                            continue;
                        }
                        let mut ac = attr.walk();
                        let mut attr_name = None;
                        let mut attr_value = None;
                        for part in attr.named_children(&mut ac) {
                            match part.kind() {
                                "attribute_name" => {
                                    attr_name = part.utf8_text(self.content).ok();
                                }
                                "attribute_value" | "quoted_attribute_value" => {
                                    attr_value = part
                                        .utf8_text(self.content)
                                        .ok()
                                        .map(|s| strip_string_literal(&s));
                                }
                                _ => {}
                            }
                        }
                        match (attr_name.as_deref(), attr_value) {
                            (Some("id"), Some(value)) if !value.is_empty() => {
                                let containers = self.container_names();
                                self.emit_symbol(
                                    node,
                                    NodeKind::Variable,
                                    &value,
                                    &containers,
                                    None,
                                );
                            }
                            (Some("src" | "href"), Some(value)) if !value.is_empty() => {
                                self.emit_import(&value);
                            }
                            _ => {}
                        }
                    }
                }
            }

            (Some(Lang::Css), "rule_set") => {
                let mut cursor = node.walk();
                if let Some(selectors) = node.named_children(&mut cursor).next()
                    && selectors.kind() == "selectors"
                {
                    let mut sc = selectors.walk();
                    for sel in selectors.named_children(&mut sc) {
                        match sel.kind() {
                            "class_selector" => {
                                if let Some(text) =
                                    first_named_text_of_kinds(sel, self.content, &["class_name"])
                                {
                                    let name = text.trim_start_matches('.').to_string();
                                    if !name.is_empty() {
                                        let containers = self.container_names();
                                        self.emit_symbol(
                                            node,
                                            NodeKind::Class,
                                            &name,
                                            &containers,
                                            None,
                                        );
                                    }
                                }
                            }
                            "id_selector" => {
                                if let Some(text) =
                                    first_named_text_of_kinds(sel, self.content, &["id_name"])
                                {
                                    let name = text.trim_start_matches('#').to_string();
                                    if !name.is_empty() {
                                        let containers = self.container_names();
                                        self.emit_symbol(
                                            node,
                                            NodeKind::Variable,
                                            &name,
                                            &containers,
                                            None,
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            (Some(Lang::Css), "import_statement") => {
                if let Ok(text) = node.utf8_text(self.content) {
                    let path = text
                        .trim()
                        .trim_start_matches("@import")
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"' || c == ' ')
                        .to_string();
                    self.emit_import(&path);
                }
            }
            (Some(Lang::Css), "call_expression") => {
                if let Some(text) =
                    first_named_text_of_kinds(node, self.content, &["function_name"])
                {
                    self.record_call(&text);
                }
            }

            (Some(Lang::Yaml), "block_mapping_pair" | "flow_pair") => {
                let depth = yaml_depth(node);
                if depth <= 2
                    && let Some(key) = field_text(node, "key", self.content)
                {
                    let name = key
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    if !name.is_empty() {
                        let containers = self.container_names();
                        self.emit_symbol(node, NodeKind::Variable, &name, &containers, None);
                    }
                }
            }

            (Some(Lang::Json), "pair") => {
                if let Some(key) = field_text(node, "key", self.content) {
                    let name = key
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    if !name.is_empty() {
                        let containers = self.container_names();
                        self.emit_symbol(node, NodeKind::Variable, &name, &containers, None);
                    }
                }
            }

            (Some(Lang::Markdown), "atx_heading" | "setext_heading") => {
                if let Some(text) =
                    field_text(node, "heading_content", self.content).or_else(|| {
                        first_named_text_of_kinds(node, self.content, &["paragraph", "inline"])
                    })
                {
                    let name = text.trim().trim_start_matches('#').trim().to_string();
                    if !name.is_empty() {
                        let containers = self.container_names();
                        self.emit_symbol(node, NodeKind::Variable, &name, &containers, None);
                    }
                }
            }

            (Some(Lang::Toml), "table" | "table_array_element") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["bare_key", "quoted_key", "dotted_key"],
                ) {
                    push = self.push_named_scope(node, NodeKind::Class, text, ScopeKind::Type);
                }
            }
            (Some(Lang::Toml), "pair") => {
                if let Some(key) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["bare_key", "quoted_key", "dotted_key"],
                ) {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Variable, &key, &containers, None);
                }
            }

            (Some(Lang::Proto), "message" | "enum") => {
                if let Some(name) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["message_name", "enum_name", "identifier"],
                ) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Proto), "service") => {
                if let Some(name) =
                    first_named_text_of_kinds(node, self.content, &["service_name", "identifier"])
                {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Proto), "rpc") => {
                if let Some(name) =
                    first_named_text_of_kinds(node, self.content, &["rpc_name", "identifier"])
                {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                }
            }
            (Some(Lang::Proto), "package") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["full_ident"]) {
                    push = self.push_named_scope(node, NodeKind::Package, text, ScopeKind::Module);
                }
            }
            (Some(Lang::Proto), "import") => {
                if let Some(text) = field_text(node, "path", self.content) {
                    self.emit_import(&strip_string_literal(&text));
                }
            }

            (Some(Lang::Terraform | Lang::Hcl), "block") => {
                let mut cursor = node.walk();
                let mut parts: Vec<String> = Vec::new();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "identifier" | "string_lit" | "string" => {
                            if let Ok(text) = child.utf8_text(self.content) {
                                parts.push(strip_string_literal(&text));
                            }
                        }
                        "block_start" | "body" | "block_end" => break,
                        _ => {}
                    }
                }
                if let Some(block_type) = parts.first().cloned() {
                    let name = parts[1..].join(".");
                    let (kind, scope_kind) = match block_type.as_str() {
                        "variable" | "output" | "locals" => (NodeKind::Variable, ScopeKind::Module),
                        "resource" | "data" | "provider" | "module" | "terraform" => {
                            (NodeKind::Class, ScopeKind::Type)
                        }
                        _ => (NodeKind::Module, ScopeKind::Module),
                    };
                    let label = if name.is_empty() {
                        block_type
                    } else {
                        format!("{block_type}.{name}")
                    };
                    push = self.push_named_scope(node, kind, label, scope_kind);
                }
            }
            (Some(Lang::Terraform | Lang::Hcl), "attribute") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["identifier"]) {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Variable, &text, &containers, None);
                }
            }
            (Some(Lang::Terraform | Lang::Hcl), "function_call") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["identifier"]) {
                    self.record_call(&text);
                }
            }

            (Some(Lang::Powershell), "function_statement") => {
                if let Some(name) =
                    first_named_text_of_kinds(node, self.content, &["function_name"])
                {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Powershell), "class_statement") => {
                if let Some(name) = first_named_text_of_kinds(node, self.content, &["simple_name"])
                {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Powershell), "enum_statement") => {
                if let Some(name) = first_named_text_of_kinds(node, self.content, &["simple_name"])
                {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Powershell), "class_method_definition") => {
                if let Some(name) =
                    first_named_text_of_kinds(node, self.content, &["simple_name", "function_name"])
                {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
                }
            }
            (Some(Lang::Powershell), "command") => {
                if let Some(name) = first_named_text_of_kinds(node, self.content, &["command_name"])
                {
                    if matches!(
                        name.as_str(),
                        "Import-Module" | "using" | "dot-source" | "."
                    ) {
                        self.emit_import(&name);
                    } else {
                        self.record_call(&name);
                    }
                }
            }

            (Some(Lang::Make), "rule") => {
                let text = child_of_kind(node, "targets")
                    .and_then(|targets| targets.utf8_text(self.content).ok())
                    .map(|t| t.to_string());
                if let Some(text) = text {
                    let name = text.split_whitespace().next().unwrap_or(&text).to_string();
                    if !name.is_empty() {
                        let containers = self.container_names();
                        let id =
                            self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (Some(Lang::Make), "variable_assignment") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Variable, &name, &containers, None);
                }
            }
            (Some(Lang::Make), "include_directive") => {
                if let Some(filenames) = node.child_by_field_name("filenames")
                    && let Ok(text) = filenames.utf8_text(self.content)
                {
                    for part in text.split_whitespace() {
                        self.emit_import(part.trim_matches(|c| c == '\\' || c == ' '));
                    }
                }
            }
            (Some(Lang::Make), "function_call") => {
                if let Some(text) = field_text(node, "function", self.content) {
                    self.record_call(&text);
                }
            }

            (Some(Lang::CMake), "function_def" | "macro_def") => {
                let mut cursor = node.walk();
                let mut name = None;
                for child in node.named_children(&mut cursor) {
                    if matches!(child.kind(), "function_command" | "macro_command")
                        && let Some(inner) = child.named_child(1)
                        && let Some(arg_list) = inner.named_child(0)
                    {
                        if let Ok(text) = arg_list.utf8_text(self.content) {
                            name = text.split_whitespace().next().map(|s| s.to_string());
                        }
                        break;
                    }
                }
                if let Some(name) = name {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::CMake), "normal_command") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["identifier"]) {
                    self.record_call(&text);
                }
            }

            (Some(Lang::Swift), "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let name = name.trim_start_matches("func ").trim().to_string();
                    if !name.is_empty() {
                        let node_kind = self.function_or_method();
                        let containers = self.container_names();
                        let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (Some(Lang::Swift), "class_declaration") => {
                let kind_text =
                    field_text(node, "declaration_kind", self.content).unwrap_or_default();
                let node_kind = match kind_text.as_str() {
                    "enum" | "struct" | "actor" => NodeKind::Class,
                    _ => NodeKind::Class,
                };
                if let Some(name) = field_text(node, "name", self.content) {
                    let name = name.trim_start_matches("class ").trim().to_string();
                    if !name.is_empty() {
                        push = self.push_named_scope(node, node_kind, name, ScopeKind::Type);
                    }
                }
            }
            (Some(Lang::Swift), "protocol_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let name = name.trim_start_matches("protocol ").trim().to_string();
                    if !name.is_empty() {
                        push =
                            self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                    }
                }
            }
            (Some(Lang::Swift), "init_declaration") => {
                let containers = self.container_names();
                let name = "init".to_string();
                self.emit_symbol(node, NodeKind::Method, &name, &containers, None);
            }
            (Some(Lang::Swift), "import_declaration") => {
                if let Some(text) = first_named_text_of_kinds(node, self.content, &["identifier"]) {
                    self.emit_import(&text);
                }
            }
            (Some(Lang::Swift), "call_expression") => {
                if let Some(text) =
                    first_named_text_of_kinds(node, self.content, &["simple_identifier"])
                {
                    self.record_call(&text);
                } else if let Ok(text) = node.utf8_text(self.content) {
                    let head = text.split('(').next().unwrap_or(&text).trim();
                    if !head.is_empty() {
                        self.record_call(head);
                    }
                }
            }

            (Some(Lang::Haskell), "function") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (Some(Lang::Haskell), "class") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Interface, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Haskell), "data_type" | "newtype" | "type_synomym" | "type_family") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Haskell), "import") => {
                if let Some(text) = field_text(node, "module", self.content) {
                    self.emit_import(&text);
                }
            }
            (Some(Lang::Haskell), "module") => {
                if let Some(text) = first_named_text_of_kinds(
                    node,
                    self.content,
                    &["module_id", "module_name", "identifier"],
                ) {
                    push = self.push_named_scope(node, NodeKind::Module, text, ScopeKind::Module);
                }
            }

            (Some(Lang::Ocaml), "value_definition") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "let_binding" {
                        continue;
                    }
                    let name = field_text(child, "pattern", self.content)
                        .or_else(|| {
                            child
                                .named_child(0)
                                .and_then(|n| n.utf8_text(self.content).ok())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default();
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let has_body = child.child_by_field_name("body").is_some();
                    let containers = self.container_names();
                    if has_body {
                        let node_kind = self.function_or_method();
                        let id = self.emit_symbol(child, node_kind, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    } else {
                        self.emit_symbol(child, NodeKind::Variable, &name, &containers, None);
                    }
                }
            }
            (Some(Lang::Ocaml), "type_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                } else if let Some(binding) = node.named_child(0)
                    && let Some(name) = field_text(binding, "name", self.content)
                {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Ocaml), "module_definition") => {
                if let Some(binding) = node.named_child(0)
                    && let Some(name) = binding
                        .named_child(0)
                        .and_then(|n| n.utf8_text(self.content).ok())
                {
                    push = self.push_named_scope(
                        node,
                        NodeKind::Module,
                        name.to_string(),
                        ScopeKind::Module,
                    );
                }
            }
            (Some(Lang::Ocaml), "class_definition") => {
                if let Some(binding) = node.named_child(0)
                    && let Some(name) = binding
                        .named_child(0)
                        .and_then(|n| n.utf8_text(self.content).ok())
                {
                    push = self.push_named_scope(
                        node,
                        NodeKind::Class,
                        name.to_string(),
                        ScopeKind::Type,
                    );
                }
            }
            (Some(Lang::Ocaml), "open_module" | "include_module") => {
                if let Some(text) = field_text(node, "module", self.content) {
                    self.emit_import(&text);
                }
            }

            (Some(Lang::Zig), "function_declaration") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, NodeKind::Function, &name, &containers, None);
                    push = Some(Scope {
                        name,
                        id,
                        kind: ScopeKind::Function,
                    });
                }
            }
            (
                Some(Lang::Zig),
                "struct_declaration"
                | "enum_declaration"
                | "union_declaration"
                | "opaque_declaration"
                | "error_set_declaration",
            ) => {
                if let Some(name) = zig_declaration_name(node, self.content) {
                    push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                }
            }
            (Some(Lang::Zig), "call_expression") => {
                if let Some(callee) = field_text(node, "function", self.content) {
                    self.record_call(&callee);
                }
            }
            (Some(Lang::Zig), "variable_declaration") => {
                if let Some(name) = zig_declaration_name(node, self.content) {
                    let containers = self.container_names();
                    self.emit_symbol(node, NodeKind::Variable, &name, &containers, None);
                }
            }

            (Some(Lang::Julia), "function_definition" | "macro_definition") => {
                if let Some(sig) = node.named_child(0) {
                    let inner = match sig.kind() {
                        "signature" => sig.named_child(0),
                        _ => Some(sig),
                    };
                    let name = match inner {
                        Some(n) if n.kind() == "identifier" => {
                            n.utf8_text(self.content).ok().map(|s| s.to_string())
                        }
                        Some(n) if matches!(n.kind(), "call_expression" | "call") => n
                            .named_child(0)
                            .and_then(|c| c.utf8_text(self.content).ok())
                            .map(|s| s.to_string()),
                        _ => None,
                    };
                    if let Some(name) = name.filter(|s| !s.is_empty()) {
                        let node_kind = self.function_or_method();
                        let containers = self.container_names();
                        let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                        push = Some(Scope {
                            name,
                            id,
                            kind: ScopeKind::Function,
                        });
                    }
                }
            }
            (Some(Lang::Julia), "struct_definition" | "abstract_definition") => {
                if let Some(head) = node.named_child(0) {
                    let name = match head.kind() {
                        "identifier" => head.utf8_text(self.content).ok().map(|s| s.to_string()),
                        "type_head" => head
                            .named_child(0)
                            .and_then(|n| n.utf8_text(self.content).ok())
                            .map(|s| s.to_string()),
                        "call_expression" | "parametrized_type_expression" => head
                            .named_child(0)
                            .and_then(|n| n.utf8_text(self.content).ok())
                            .map(|s| s.to_string()),
                        _ => None,
                    };
                    if let Some(name) = name.filter(|s| !s.is_empty()) {
                        push = self.push_named_scope(node, NodeKind::Class, name, ScopeKind::Type);
                    }
                }
            }
            (Some(Lang::Julia), "module_definition") => {
                if let Some(name) = field_text(node, "name", self.content) {
                    push = self.push_named_scope(node, NodeKind::Module, name, ScopeKind::Module);
                }
            }
            (Some(Lang::Julia), "import_statement" | "using_statement") => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "import_path" | "selected_import" | "identifier" => {
                            if let Ok(text) = child.utf8_text(self.content) {
                                let path =
                                    text.split_whitespace().next().unwrap_or(&text).to_string();
                                if !path.is_empty() {
                                    self.emit_import(&path);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            (Some(Lang::Julia), "call_expression") => {
                if let Some(callee) = node.named_child(0)
                    && let Ok(text) = callee.utf8_text(self.content)
                {
                    self.record_call(text);
                }
            }

            (None, k) => {
                if generic_is_import(k) {
                    if let Some(path) = generic_import_path(node, self.content) {
                        self.emit_import(&path);
                    }
                } else if generic_is_call(k) {
                    if let Some(callee) = generic_callee(node, self.content) {
                        self.record_call(&callee);
                    }
                } else if let Some(node_kind) = generic_symbol_kind(k)
                    && let Some(name) = generic_name(node, self.content)
                {
                    let node_kind = if matches!(node_kind, NodeKind::Function) {
                        self.function_or_method()
                    } else {
                        node_kind
                    };
                    let containers = self.container_names();
                    let id = self.emit_symbol(node, node_kind, &name, &containers, None);
                    let scope_kind = match node_kind {
                        NodeKind::Function | NodeKind::Method => Some(ScopeKind::Function),
                        NodeKind::Module => Some(ScopeKind::Module),
                        NodeKind::Class | NodeKind::Interface => Some(ScopeKind::Type),
                        _ => None,
                    };
                    if let Some(scope_kind) = scope_kind {
                        push = Some(Scope {
                            name,
                            id,
                            kind: scope_kind,
                        });
                    }
                }
            }

            _ => {}
        }

        let pushed = push.is_some();

        if let Some(scope) = push {
            self.scope.push(scope);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child);
        }

        if pushed {
            self.scope.pop();
        }
    }
}

fn yaml_depth(node: Node) -> usize {
    let mut depth = 0;
    let mut current = node.parent();
    while let Some(n) = current {
        if matches!(n.kind(), "block_mapping_pair" | "flow_pair") {
            depth += 1;
        }
        current = n.parent();
    }
    depth
}

fn zig_declaration_name(node: Node, content: &[u8]) -> Option<String> {
    if let Some(name) = field_text(node, "name", content) {
        return Some(name);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" => {
                if let Ok(text) = child.utf8_text(content) {
                    return Some(text.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn dart_signature_name(node: Node, content: &[u8]) -> Option<String> {
    let signature = node.child_by_field_name("signature")?;
    if let Some(name) = field_text(signature, "name", content) {
        return Some(name);
    }
    let mut cursor = signature.walk();
    for child in signature.named_children(&mut cursor) {
        if let Some(name) = field_text(child, "name", content) {
            return Some(name);
        }
    }
    None
}

fn generic_symbol_kind(kind: &str) -> Option<NodeKind> {
    if !generic_is_declaration(kind) {
        return None;
    }
    if contains_any(kind, &["interface", "protocol", "trait"]) {
        return Some(NodeKind::Interface);
    }
    if contains_any(kind, &["module", "namespace", "package"]) {
        return Some(NodeKind::Module);
    }
    if contains_any(
        kind,
        &["function", "method", "macro", "subroutine", "define", "fun"],
    ) {
        return Some(NodeKind::Function);
    }
    if contains_any(
        kind,
        &[
            "class", "struct", "enum", "record", "union", "object", "variant", "concept", "entity",
            "type",
        ],
    ) {
        return Some(NodeKind::Class);
    }
    if contains_any(
        kind,
        &["variable", "constant", "field", "property", "binding"],
    ) {
        return Some(NodeKind::Variable);
    }
    None
}

fn generic_is_declaration(kind: &str) -> bool {
    contains_any(
        kind,
        &[
            "declaration",
            "definition",
            "_decl",
            "_item",
            "_def",
            "_decls",
        ],
    ) || matches!(
        kind,
        "method"
            | "class"
            | "function"
            | "def"
            | "macro"
            | "module"
            | "interface"
            | "struct"
            | "enum"
            | "trait"
            | "record"
            | "protocol"
            | "object"
            | "union"
            | "namespace"
            | "package"
            | "type"
            | "variant"
            | "variable"
            | "constant"
            | "subtype"
            | "behavior"
    )
}

fn generic_is_import(kind: &str) -> bool {
    contains_any(
        kind,
        &["import", "include", "require", "using", "load", "from_file"],
    ) || kind.starts_with("use")
}

fn generic_is_call(kind: &str) -> bool {
    contains_any(kind, &["call", "invocation"])
}

fn generic_name(node: Node, content: &[u8]) -> Option<String> {
    if let Some(name) = field_text(node, "name", content) {
        return Some(name);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if !(kind.contains("identifier")
            || kind.contains("_name")
            || kind == "name"
            || kind.contains("symbol")
            || kind == "word"
            || kind.contains("label"))
        {
            continue;
        }
        if let Ok(text) = child.utf8_text(content) {
            let text = text.trim();
            if !text.is_empty() && !text.contains('\n') && text.len() <= 128 {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn generic_import_path(node: Node, content: &[u8]) -> Option<String> {
    for field in ["path", "module", "source", "argument", "target", "name"] {
        if let Some(value) = field_text(node, field, content) {
            return Some(value);
        }
    }
    let child = node.named_child(0)?;
    let text = child.utf8_text(content).ok()?;
    if text.lines().count() > 1 || text.len() > 200 {
        return None;
    }
    Some(text.trim().to_string())
}

fn generic_callee(node: Node, content: &[u8]) -> Option<String> {
    for field in ["function", "callee", "command", "target", "name"] {
        if let Some(value) = field_text(node, field, content) {
            return Some(value);
        }
    }
    let child = node.named_child(0)?;
    let text = child.utf8_text(content).ok()?.trim().to_string();
    if text.is_empty() || text.contains('\n') || text.len() > 200 {
        return None;
    }
    Some(text)
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn receiver_type(receiver: &str) -> Option<String> {
    let inner = receiver
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let token = inner.split_whitespace().last()?;
    let token = token.trim_start_matches(['*', '&']);
    let token = token.split('[').next().unwrap_or(token);
    let name = token.rsplit('.').next().unwrap_or(token);
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
pub(crate) fn contracts_for_test(
    repo: &str,
    sha: &str,
    path: &str,
    source: &str,
    language: &str,
) -> ckg_domain::ContractBatch {
    TreeSitterAnalyzer::new()
        .analyze(repo, sha, path, source, language)
        .contracts
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::ProvenanceSource;

    const REPO: &str = "org/svc";
    const SHA: &str = "abc123";

    fn find_symbol<'a>(output: &'a AnalyzerOutput, name: &str) -> Option<&'a RawSymbol> {
        output.symbols.iter().find(|s| s.name == name)
    }

    fn calls_to<'a>(output: &'a AnalyzerOutput, callee: &str) -> Vec<&'a RawRelation> {
        output
            .relations
            .iter()
            .filter(|r| {
                r.kind == RelationKind::Calls
                    && r.properties.get("callee").and_then(|v| v.as_str()) == Some(callee)
            })
            .collect()
    }

    #[test]
    fn rust_file_extracts_symbols_and_relations() {
        let source = r#"use std::collections::HashMap;

pub struct Config {
    pub name: String,
}

impl Config {
    pub fn build() -> Self {
        let h = HashMap::new();
        helper(h);
        Config { name: String::from("x") }
    }
}

fn helper(map: HashMap<String, String>) {
    let _ = map;
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/lib.rs", source, "rust");

        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let build = find_symbol(&output, "build").expect("build method");
        assert_eq!(build.kind, NodeKind::Method);
        assert_eq!(
            build.qualified_name,
            "rust::org/svc::src::lib::Config::build"
        );
        let loc = build.location.as_ref().expect("location");
        assert_eq!(loc.start_line, 8);
        assert_eq!(loc.end_line, 12);
        assert_eq!(loc.path, "src/lib.rs");
        assert_eq!(loc.commit_sha, SHA);
        assert_eq!(loc.repository, REPO);
        assert!(!build.content_hash.as_ref().unwrap().is_empty());

        let config = find_symbol(&output, "Config").expect("struct");
        assert_eq!(config.kind, NodeKind::Class);
        assert_eq!(config.location.as_ref().unwrap().start_line, 3);
        assert_eq!(config.qualified_name, "rust::org/svc::src::lib::Config");

        let helper = find_symbol(&output, "helper").expect("helper fn");
        assert_eq!(helper.kind, NodeKind::Function);
        assert_eq!(helper.location.as_ref().unwrap().start_line, 15);

        let import = output
            .relations
            .iter()
            .find(|r| r.kind == RelationKind::Imports)
            .expect("import relation");
        assert_eq!(
            import.properties.get("import_path").unwrap().as_str(),
            Some("std::collections::HashMap")
        );
        assert_eq!(
            import.to,
            CanonicalId::from_parts(&["import", "rust", "std::collections::HashMap"])
        );

        let helper_calls = calls_to(&output, "helper");
        assert_eq!(helper_calls.len(), 1);
        let helper_call = helper_calls[0];
        assert_eq!(
            helper_call
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "local"
        );
        assert_eq!(helper_call.to, helper.canonical_id);
        assert_eq!(helper_call.from, build.canonical_id);

        let new_calls = calls_to(&output, "HashMap::new");
        assert_eq!(new_calls.len(), 1);
        assert_eq!(
            new_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "unresolved"
        );

        assert_eq!(output.provenance.len(), 1);
        assert_eq!(output.provenance[0].source, ProvenanceSource::TreeSitter);
        assert_eq!(output.provenance[0].status, ProcessingStatus::Complete);
        assert_eq!(
            output.provenance[0].analyzer_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn rust_parse_errors_yield_partial_status() {
        let source = "fn broken( {\n";
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/broken.rs", source, "rust");
        assert!(!output.errors.is_empty());
        assert_eq!(output.provenance[0].status, ProcessingStatus::Partial);
    }

    #[test]
    fn python_def_extracts_function_class_and_calls() {
        let source = r#"import os

class Engine:
    def run(self):
        start()

def start():
    pass
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "pkg/core.py", source, "python");

        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);
        assert_eq!(engine.location.as_ref().unwrap().start_line, 3);

        let run = find_symbol(&output, "run").expect("method");
        assert_eq!(run.kind, NodeKind::Method);
        assert_eq!(run.location.as_ref().unwrap().start_line, 4);
        assert_eq!(
            run.qualified_name,
            "python::org/svc::pkg::core::Engine::run"
        );

        let start = find_symbol(&output, "start").expect("function");
        assert_eq!(start.kind, NodeKind::Function);
        assert_eq!(start.location.as_ref().unwrap().start_line, 7);

        let import = output
            .relations
            .iter()
            .find(|r| r.kind == RelationKind::Imports)
            .expect("import");
        assert_eq!(
            import.properties.get("import_path").unwrap().as_str(),
            Some("os")
        );

        let start_calls = calls_to(&output, "start");
        assert_eq!(start_calls.len(), 1);
        assert_eq!(
            start_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "local"
        );
        assert_eq!(start_calls[0].to, start.canonical_id);
        assert_eq!(start_calls[0].from, run.canonical_id);
    }

    #[test]
    fn javascript_extracts_function_class_and_calls() {
        let source = r#"import { util } from "./util.js";

export class Runner {
  execute() {
    return util.run();
  }
}

function main() {
  const r = new Runner();
  r.execute();
}

main();
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/index.js", source, "javascript");

        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let runner = find_symbol(&output, "Runner").expect("class");
        assert_eq!(runner.kind, NodeKind::Class);
        assert_eq!(runner.location.as_ref().unwrap().start_line, 3);

        let execute = find_symbol(&output, "execute").expect("method");
        assert_eq!(execute.kind, NodeKind::Method);
        assert_eq!(execute.location.as_ref().unwrap().start_line, 4);

        let main = find_symbol(&output, "main").expect("function");
        assert_eq!(main.kind, NodeKind::Function);
        assert_eq!(main.location.as_ref().unwrap().start_line, 9);

        let import = output
            .relations
            .iter()
            .find(|r| r.kind == RelationKind::Imports)
            .expect("import");
        assert_eq!(
            import.properties.get("import_path").unwrap().as_str(),
            Some("./util.js")
        );

        let main_calls = calls_to(&output, "main");
        assert_eq!(main_calls.len(), 1);
        assert_eq!(
            main_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "local"
        );
        assert_eq!(main_calls[0].to, main.canonical_id);

        let execute_calls = calls_to(&output, "r.execute");
        assert_eq!(execute_calls.len(), 1);
        assert_eq!(
            execute_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "local"
        );
        assert_eq!(execute_calls[0].to, execute.canonical_id);

        let run_calls = calls_to(&output, "util.run");
        assert_eq!(run_calls.len(), 1);
        assert_eq!(
            run_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "unresolved"
        );
    }

    #[test]
    fn typescript_interface_and_function_extracted() {
        let source = r#"interface Store {
  get(): string;
}

export function load(): Store {
  return null as any;
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/store.ts", source, "typescript");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let store = find_symbol(&output, "Store").expect("interface");
        assert_eq!(store.kind, NodeKind::Interface);
        assert_eq!(store.location.as_ref().unwrap().start_line, 1);

        let load = find_symbol(&output, "load").expect("function");
        assert_eq!(load.kind, NodeKind::Function);
        assert_eq!(load.location.as_ref().unwrap().start_line, 5);
    }

    #[test]
    fn go_extracts_package_struct_and_methods() {
        let source = r#"package svc

import "fmt"

type Server struct {
	name string
}

func (s *Server) Start() error {
	fmt.Println(s.name)
	return nil
}

func New() *Server {
	return &Server{}
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "svc/server.go", source, "go");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let pkg = find_symbol(&output, "svc").expect("package");
        assert_eq!(pkg.kind, NodeKind::Package);

        let server = find_symbol(&output, "Server").expect("struct");
        assert_eq!(server.kind, NodeKind::Class);
        assert_eq!(server.location.as_ref().unwrap().start_line, 5);

        let start = find_symbol(&output, "Start").expect("method");
        assert_eq!(start.kind, NodeKind::Method);
        assert_eq!(start.location.as_ref().unwrap().start_line, 9);
        assert_eq!(
            start.qualified_name,
            "go::org/svc::svc::server::svc::Server::Start"
        );

        let new_fn = find_symbol(&output, "New").expect("func");
        assert_eq!(new_fn.kind, NodeKind::Function);
        assert_eq!(new_fn.location.as_ref().unwrap().start_line, 14);

        let import = output
            .relations
            .iter()
            .find(|r| r.kind == RelationKind::Imports)
            .expect("import");
        assert_eq!(
            import.properties.get("import_path").unwrap().as_str(),
            Some("fmt")
        );

        let println_calls = calls_to(&output, "fmt.Println");
        assert_eq!(println_calls.len(), 1);
        assert_eq!(
            println_calls[0]
                .properties
                .get("resolved")
                .unwrap()
                .as_str()
                .unwrap(),
            "unresolved"
        );
    }

    #[test]
    fn language_for_path_maps_extensions() {
        assert_eq!(language_for_path("src/main.rs"), Some("rust"));
        assert_eq!(language_for_path("src/app.js"), Some("javascript"));
        assert_eq!(language_for_path("src/app.jsx"), Some("javascript"));
        assert_eq!(language_for_path("src/app.ts"), Some("typescript"));
        assert_eq!(language_for_path("src/app.tsx"), Some("typescript"));
        assert_eq!(language_for_path("pkg/mod.py"), Some("python"));
        assert_eq!(language_for_path("pkg/mod.pyi"), Some("python"));
        assert_eq!(language_for_path("cmd/main.go"), Some("go"));
        assert_eq!(language_for_path("Main.java"), Some("java"));
        assert_eq!(language_for_path("src/util.c"), Some("c"));
        assert_eq!(language_for_path("src/core.cpp"), Some("cpp"));
        assert_eq!(language_for_path("App.kt"), Some("kotlin"));
        assert_eq!(language_for_path("build.kts"), Some("kotlin"));
        assert_eq!(language_for_path("query.sql"), Some("sql"));
        assert_eq!(language_for_path("README.md"), Some("markdown"));
        assert_eq!(language_for_path("script.rb"), Some("ruby"));
        assert_eq!(language_for_path("app.php"), Some("php"));
        assert_eq!(language_for_path("Program.cs"), Some("csharp"));
        assert_eq!(language_for_path("Main.scala"), Some("scala"));
        assert_eq!(language_for_path("run.sh"), Some("shell"));
        assert_eq!(language_for_path("app.ex"), Some("elixir"));
        assert_eq!(language_for_path("main.dart"), Some("dart"));
        assert_eq!(language_for_path("lib.lua"), Some("lua"));
        assert_eq!(language_for_path("plot.R"), Some("r"));
        assert_eq!(language_for_path("main.jl"), Some("julia"));
        assert_eq!(language_for_path("Main.hs"), Some("haskell"));
        assert_eq!(language_for_path("util.ml"), Some("ocaml"));
        assert_eq!(language_for_path("src/main.zig"), Some("zig"));
        assert_eq!(language_for_path("index.html"), Some("html"));
        assert_eq!(language_for_path("style.css"), Some("css"));
        assert_eq!(language_for_path("config.yml"), Some("yaml"));
        assert_eq!(language_for_path("data.json"), Some("json"));
        assert_eq!(language_for_path("Cargo.toml"), Some("toml"));
        assert_eq!(language_for_path("api.proto"), Some("proto"));
        assert_eq!(language_for_path("main.tf"), Some("terraform"));
        assert_eq!(language_for_path("script.ps1"), Some("powershell"));
        assert_eq!(language_for_path("Makefile"), Some("make"));
        assert_eq!(language_for_path("CMakeLists.txt"), Some("cmake"));
        assert_eq!(language_for_path("main.swift"), Some("swift"));
        assert_eq!(language_for_path("LICENSE"), None);
    }

    #[test]
    fn tsx_path_uses_tsx_grammar() {
        let source = r#"export const App = () => <div className="x">hi</div>;
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/App.tsx", source, "typescript");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);
        assert!(
            output
                .provenance
                .iter()
                .any(|p| p.status == ProcessingStatus::Complete)
        );
    }

    #[test]
    fn detection_only_language_yields_skipped() {
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "script.pl", "print 'hi';", "perl");
        assert_eq!(output.errors.len(), 1);
        assert!(output.errors[0].contains("no tree-sitter grammar"));
        assert!(output.symbols.is_empty());
        assert_eq!(output.provenance[0].status, ProcessingStatus::Skipped);
    }

    #[test]
    fn tier3_with_grammar_uses_static_extraction() {
        assert!(has_grammar("ruby"));
        assert!(has_grammar("yaml"));
        assert!(!has_grammar("perl"));
        assert!(!has_grammar("scss"));
    }

    #[test]
    fn unsupported_language_reports_error() {
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "x.bf", "hello", "brainfuck");
        assert_eq!(output.errors.len(), 1);
        assert!(output.symbols.is_empty());
        assert_eq!(output.provenance[0].status, ProcessingStatus::Failed);
    }

    #[test]
    fn java_extracts_package_class_method_and_calls() {
        let source = r#"package com.acme.core;

import java.util.List;
import static java.lang.Math.max;

public interface Greeter {
    void greet();
}

public class Engine implements Greeter {
    private final List<String> items;

    public Engine(List<String> items) {
        this.items = items;
        validate(items);
    }

    @Override
    public void greet() {
        Helper.run();
        Engine e = new Engine(items);
        max(1, 2);
    }
}

class Helper {
    static void run() {
    }
}

class ValidateHolder {
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Engine.java", source, "java");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let pkg = find_symbol(&output, "com.acme.core").expect("package");
        assert_eq!(pkg.kind, NodeKind::Package);

        let greeter = find_symbol(&output, "Greeter").expect("interface");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let engine_symbols: Vec<_> = output
            .symbols
            .iter()
            .filter(|s| s.name == "Engine")
            .collect();
        assert!(engine_symbols.len() >= 2, "expected class + constructor");

        let greet = find_symbol(&output, "greet").expect("method");
        assert_eq!(greet.kind, NodeKind::Method);

        let run = find_symbol(&output, "run").expect("static method");
        assert_eq!(run.kind, NodeKind::Method);

        let imports: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .collect();
        assert!(imports.len() >= 2, "imports: {:?}", imports);
        let import_paths: Vec<_> = imports
            .iter()
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(import_paths.contains(&"java.util.List"));
        assert!(import_paths.iter().any(|p| p.contains("Math")));

        let run_calls = calls_to(&output, "Helper.run");
        assert_eq!(run_calls.len(), 1);
        assert!(!calls_to(&output, "Engine").is_empty());
        assert_eq!(calls_to(&output, "validate").len(), 1);
    }

    #[test]
    fn c_extracts_function_struct_include_and_calls() {
        let source = r#"#include <stdio.h>
#include "local.h"

struct Point {
    int x;
    int y;
};

union Value {
    int i;
    float f;
};

enum Color {
    RED,
    GREEN
};

int scale(int v) {
    return v * 2;
}

int main(void) {
    struct Point p;
    scale(3);
    printf("hi");
    return 0;
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/main.c", source, "c");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let point = find_symbol(&output, "Point").expect("struct");
        assert_eq!(point.kind, NodeKind::Class);

        let value = find_symbol(&output, "Value").expect("union");
        assert_eq!(value.kind, NodeKind::Class);

        let color = find_symbol(&output, "Color").expect("enum");
        assert_eq!(color.kind, NodeKind::Class);

        let scale = find_symbol(&output, "scale").expect("function");
        assert_eq!(scale.kind, NodeKind::Function);

        let main_fn = find_symbol(&output, "main").expect("main");
        assert_eq!(main_fn.kind, NodeKind::Function);

        let imports: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .collect();
        assert_eq!(imports.len(), 2);
        let paths: Vec<_> = imports
            .iter()
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(paths.contains(&"stdio.h"));
        assert!(paths.contains(&"local.h"));

        assert_eq!(calls_to(&output, "scale").len(), 1);
        assert_eq!(calls_to(&output, "printf").len(), 1);
    }

    #[test]
    fn cpp_extracts_namespace_class_function_and_using() {
        let source = r#"#include <vector>
using std::vector;

namespace acme {

class Widget {
public:
    void render();
    int id;
};

struct Config {
    bool verbose;
};

enum Mode { On, Off };

void Widget::render() {
    build();
}

int build() {
    return 1;
}

}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/widget.cpp", source, "cpp");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let ns = find_symbol(&output, "acme").expect("namespace");
        assert_eq!(ns.kind, NodeKind::Module);

        let widget = find_symbol(&output, "Widget").expect("class");
        assert_eq!(widget.kind, NodeKind::Class);

        let config = find_symbol(&output, "Config").expect("struct");
        assert_eq!(config.kind, NodeKind::Class);

        let mode = find_symbol(&output, "Mode").expect("enum");
        assert_eq!(mode.kind, NodeKind::Class);

        let render = find_symbol(&output, "render").expect("method");
        assert_eq!(render.kind, NodeKind::Method);

        let build = find_symbol(&output, "build").expect("function");
        assert_eq!(build.kind, NodeKind::Function);

        let imports: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .collect();
        assert!(imports.len() >= 2, "imports: {imports:?}");
        let paths: Vec<_> = imports
            .iter()
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(paths.contains(&"vector"));
        assert!(paths.contains(&"std::vector"));

        assert_eq!(calls_to(&output, "build").len(), 1);
    }

    #[test]
    fn kotlin_extracts_package_class_fun_import_and_calls() {
        let source = r#"package com.acme.app

import com.acme.core.Engine
import com.acme.core.Helper

class Runner {
    fun start() {
        Helper.run()
        val e = Engine()
    }
}

interface Greeter {
    fun greet()
}

object Registry {
    fun register() {
    }
}

fun topLevel() {
    Runner().start()
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Runner.kt", source, "kotlin");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let pkg = find_symbol(&output, "com.acme.app").expect("package");
        assert_eq!(pkg.kind, NodeKind::Package);

        let runner = find_symbol(&output, "Runner").expect("class");
        assert_eq!(runner.kind, NodeKind::Class);

        let greeter = find_symbol(&output, "Greeter").expect("interface");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let registry = find_symbol(&output, "Registry").expect("object");
        assert_eq!(registry.kind, NodeKind::Class);

        let start = find_symbol(&output, "start").expect("fun method");
        assert_eq!(start.kind, NodeKind::Method);

        let greet = find_symbol(&output, "greet").expect("interface fun");
        assert_eq!(greet.kind, NodeKind::Method);

        let register = find_symbol(&output, "register").expect("object fun");
        assert_eq!(register.kind, NodeKind::Method);

        let top = find_symbol(&output, "topLevel").expect("top-level fun");
        assert_eq!(top.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(import_paths.contains(&"com.acme.core.Engine"));
        assert!(import_paths.contains(&"com.acme.core.Helper"));

        assert_eq!(calls_to(&output, "Helper.run").len(), 1);
        assert!(!calls_to(&output, "Engine").is_empty());
        assert_eq!(calls_to(&output, "Runner().start").len(), 1);
    }

    #[test]
    fn sql_extracts_create_table_and_function() {
        let source = r#"CREATE TABLE users (
    id INT PRIMARY KEY,
    name TEXT
);

CREATE VIEW active_users AS
    SELECT * FROM users;

CREATE FUNCTION get_count() RETURNS INT AS $$ SELECT 1; $$ LANGUAGE sql;
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "db/schema.sql", source, "sql");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let users = find_symbol(&output, "users").expect("table");
        assert_eq!(users.kind, NodeKind::Class);

        let view = find_symbol(&output, "active_users").expect("view");
        assert_eq!(view.kind, NodeKind::Class);

        let func = find_symbol(&output, "get_count").expect("function");
        assert_eq!(func.kind, NodeKind::Function);
    }

    #[test]
    fn analyzer_trait_surface() {
        let analyzer = TreeSitterAnalyzer::new();
        assert_eq!(Analyzer::name(&analyzer), "tree-sitter");
        assert_eq!(analyzer.version(), env!("CARGO_PKG_VERSION"));
        let output = analyzer.analyze_file(REPO, SHA, "src/lib.rs", "fn main() {}", "rust");
        assert_eq!(output.symbols.len(), 1);
        assert_eq!(output.symbols[0].name, "main");
    }

    #[test]
    fn ruby_extracts_module_class_method_and_calls() {
        let source = r#"require 'json'

module Util
  class Runner
    def run
      helper
      puts "hi"
    end
  end
end

def helper
end
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "lib/app.rb", source, "ruby");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let util = find_symbol(&output, "Util").expect("module");
        assert_eq!(util.kind, NodeKind::Module);

        let runner = find_symbol(&output, "Runner").expect("class");
        assert_eq!(runner.kind, NodeKind::Class);

        let run = find_symbol(&output, "run").expect("method");
        assert_eq!(run.kind, NodeKind::Method);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(import_paths.contains(&"json"), "imports: {import_paths:?}");

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "puts").is_empty());
    }

    #[test]
    fn php_extracts_namespace_class_function_and_calls() {
        let source = r#"<?php
namespace App\Core;

use App\Util\Helper;

interface Greeter {
    public function greet();
}

class Engine implements Greeter {
    public function greet() {
        $this->helper();
        echo "hi";
    }
    private function helper() {
    }
}

function boot() {
    boot();
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Engine.php", source, "php");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let ns = find_symbol(&output, "App\\Core").expect("namespace");
        assert_eq!(ns.kind, NodeKind::Package);

        let greeter = find_symbol(&output, "Greeter").expect("interface");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let greet = find_symbol(&output, "greet").expect("method");
        assert_eq!(greet.kind, NodeKind::Method);

        let boot = find_symbol(&output, "boot").expect("function");
        assert_eq!(boot.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.iter().any(|p| p.contains("Helper")),
            "imports: {import_paths:?}"
        );
        assert!(!calls_to(&output, "boot").is_empty());
    }

    #[test]
    fn csharp_extracts_namespace_class_method_and_calls() {
        let source = r#"using System;

namespace Acme.Core
{
    public interface IGreeter
    {
        void Greet();
    }

    public class Engine : IGreeter
    {
        public Engine()
        {
            Helper.Run();
        }

        public void Greet()
        {
            Console.WriteLine("hi");
        }
    }

    static class Helper
    {
        public static void Run() { }
    }
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Engine.cs", source, "csharp");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let ns = find_symbol(&output, "Acme.Core").expect("namespace");
        assert_eq!(ns.kind, NodeKind::Package);

        let iface = find_symbol(&output, "IGreeter").expect("interface");
        assert_eq!(iface.kind, NodeKind::Interface);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let greet = find_symbol(&output, "Greet").expect("method");
        assert_eq!(greet.kind, NodeKind::Method);

        assert!(!calls_to(&output, "Helper.Run").is_empty());
        assert!(!calls_to(&output, "Console.WriteLine").is_empty());

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"System"),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn scala_extracts_package_class_trait_and_calls() {
        let source = r#"package com.acme.core

import com.acme.util.Helper

class Engine {
  def run(): Unit = {
    Helper.run()
  }
}

trait Greeter {
  def greet(): Unit
}

object Boot {
  def main(args: Array[String]): Unit = {
    val e = new Engine()
    e.run()
  }
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Engine.scala", source, "scala");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let pkg = find_symbol(&output, "com.acme.core").expect("package");
        assert_eq!(pkg.kind, NodeKind::Package);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let greeter = find_symbol(&output, "Greeter").expect("trait");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let run = find_symbol(&output, "run").expect("method");
        assert_eq!(run.kind, NodeKind::Method);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.iter().any(|p| p.contains("Helper")),
            "imports: {import_paths:?}"
        );
        assert!(!calls_to(&output, "Helper.run").is_empty());
    }

    #[test]
    fn shell_extracts_function_and_calls() {
        let source = r#"#!/usr/bin/env bash
set -euo pipefail

greet() {
  echo "hello"
  run_step
}

run_step() {
  true
}

greet
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "scripts/run.sh", source, "shell");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let greet = find_symbol(&output, "greet").expect("function");
        assert_eq!(greet.kind, NodeKind::Function);

        let run_step = find_symbol(&output, "run_step").expect("function");
        assert_eq!(run_step.kind, NodeKind::Function);

        assert!(!calls_to(&output, "echo").is_empty());
        assert!(!calls_to(&output, "run_step").is_empty());
        assert!(!calls_to(&output, "greet").is_empty());
    }

    #[test]
    fn elixir_extracts_module_function_and_calls() {
        let source = r#"defmodule App.Engine do
  alias App.Helper
  import Enum

  def run do
    helper()
    IO.puts("hi")
  end

  defp helper do
    :ok
  end
end
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "lib/engine.ex", source, "elixir");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let engine = find_symbol(&output, "App.Engine").expect("module");
        assert_eq!(engine.kind, NodeKind::Class);

        let run = find_symbol(&output, "run").expect("function");
        assert_eq!(run.kind, NodeKind::Method);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Method);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"App.Helper"),
            "imports: {import_paths:?}"
        );
        assert!(import_paths.contains(&"Enum"), "imports: {import_paths:?}");

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "IO.puts").is_empty());
    }

    #[test]
    fn dart_extracts_class_method_import_and_calls() {
        let source = r#"import 'dart:math';

class Engine {
  void run() {
    helper();
  }

  void helper() {}
}

void main() {
  final e = Engine();
  e.run();
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "lib/main.dart", source, "dart");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let run = find_symbol(&output, "run").expect("method");
        assert_eq!(run.kind, NodeKind::Method);

        let main_fn = find_symbol(&output, "main").expect("function");
        assert_eq!(main_fn.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"dart:math"),
            "imports: {import_paths:?}"
        );

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "e.run").is_empty());
    }

    #[test]
    fn lua_extracts_function_and_calls() {
        let source = r#"local function greet(name)
  print(name)
  helper()
end

function helper()
  return 1
end

greet("world")
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "lib/app.lua", source, "lua");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let greet = find_symbol(&output, "greet").expect("function");
        assert_eq!(greet.kind, NodeKind::Function);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        assert!(!calls_to(&output, "print").is_empty());
        assert!(!calls_to(&output, "helper").is_empty());
    }

    #[test]
    fn r_extracts_function_and_calls() {
        let source = r#"library(stats)

run_analysis <- function(data) {
  helper(data)
}

helper <- function(x) {
  mean(x)
}

run_analysis(1:10)
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "analysis.R", source, "r");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let run = find_symbol(&output, "run_analysis").expect("function");
        assert_eq!(run.kind, NodeKind::Function);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(import_paths.contains(&"stats"), "imports: {import_paths:?}");

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "mean").is_empty());
    }

    #[test]
    fn html_extracts_ids_and_script_imports() {
        let source = r#"<!DOCTYPE html>
<html>
<head>
  <script src="/app.js"></script>
  <link href="/style.css" rel="stylesheet">
</head>
<body>
  <div id="root"></div>
</body>
</html>
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "public/index.html", source, "html");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let root = find_symbol(&output, "root").expect("id");
        assert_eq!(root.kind, NodeKind::Variable);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"/app.js"),
            "imports: {import_paths:?}"
        );
        assert!(
            import_paths.contains(&"/style.css"),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn css_extracts_class_selectors_and_imports() {
        let source = r#"@import "base.css";

.btn {
  color: red;
}

#main {
  display: block;
}

.card .title {
  font-weight: bold;
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "styles/app.css", source, "css");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let btn = find_symbol(&output, "btn").expect("class selector");
        assert_eq!(btn.kind, NodeKind::Class);

        let main = find_symbol(&output, "main").expect("id selector");
        assert_eq!(main.kind, NodeKind::Variable);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"base.css"),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn yaml_extracts_top_level_keys() {
        let source = r#"name: app
version: 1.0
services:
  web:
    image: nginx
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "config.yaml", source, "yaml");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let name = find_symbol(&output, "name").expect("key");
        assert_eq!(name.kind, NodeKind::Variable);

        let version = find_symbol(&output, "version").expect("key");
        assert_eq!(version.kind, NodeKind::Variable);
    }

    #[test]
    fn toml_extracts_tables_and_pairs() {
        let source = r#"[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "demo"
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "Cargo.toml", source, "toml");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let package = find_symbol(&output, "package").expect("table");
        assert_eq!(package.kind, NodeKind::Class);

        let name = find_symbol(&output, "name").expect("pair");
        assert_eq!(name.kind, NodeKind::Variable);
    }

    #[test]
    fn json_extracts_keys() {
        let source = r#"{
  "name": "demo",
  "nested": { "ok": true }
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "data.json", source, "json");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let name = find_symbol(&output, "name").expect("key");
        assert_eq!(name.kind, NodeKind::Variable);
    }

    #[test]
    fn markdown_extracts_headings() {
        let source = r#"# Intro

Some text.

## Details

More text.
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "docs/README.md", source, "markdown");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        assert!(find_symbol(&output, "Intro").is_some());
        assert!(find_symbol(&output, "Details").is_some());
    }

    #[test]
    fn terraform_extracts_resources_variables_and_calls() {
        let source = r#"variable "region" {
  type = string
}

resource "aws_instance" "web" {
  ami = "abc"
  instance_type = var.type_name
}

output "ip" {
  value = aws_instance.web.public_ip
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "infra/main.tf", source, "terraform");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let var = find_symbol(&output, "variable.region").expect("variable block");
        assert_eq!(var.kind, NodeKind::Variable);

        let res = find_symbol(&output, "resource.aws_instance.web").expect("resource");
        assert_eq!(res.kind, NodeKind::Class);

        let out = find_symbol(&output, "output.ip").expect("output");
        assert_eq!(out.kind, NodeKind::Variable);
    }

    #[test]
    fn proto_extracts_package_message_service_and_imports() {
        let source = r#"syntax = "proto3";

package acme.v1;

import "google/protobuf/any.proto";

message Engine {
  string name = 1;
}

service Greeter {
  rpc SayHello (Engine) returns (Engine);
}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "api.proto", source, "proto");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let pkg = find_symbol(&output, "acme.v1").expect("package");
        assert_eq!(pkg.kind, NodeKind::Package);

        let engine = find_symbol(&output, "Engine").expect("message");
        assert_eq!(engine.kind, NodeKind::Class);

        let greeter = find_symbol(&output, "Greeter").expect("service");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let say = find_symbol(&output, "SayHello").expect("rpc");
        assert_eq!(say.kind, NodeKind::Method);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.iter().any(|p| p.contains("any.proto")),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn powershell_extracts_function_class_and_commands() {
        let source = r#"function Invoke-Thing {
    Write-Output "hi"
    Get-ChildItem
}

class Engine {
    [void] Run() {
        Write-Output "run"
    }
}

Invoke-Thing
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "scripts/app.ps1", source, "powershell");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let f = find_symbol(&output, "Invoke-Thing").expect("function");
        assert_eq!(f.kind, NodeKind::Function);

        let cls = find_symbol(&output, "Engine").expect("class");
        assert_eq!(cls.kind, NodeKind::Class);

        assert!(!calls_to(&output, "Write-Output").is_empty());
    }

    #[test]
    fn make_extracts_rules_variables_and_includes() {
        let source = r#"include common.mk

CFLAGS = -O2

all: build

build:
	gcc -o app main.c
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "Makefile", source, "make");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let all = find_symbol(&output, "all").expect("rule");
        assert_eq!(all.kind, NodeKind::Function);

        let cflags = find_symbol(&output, "CFLAGS").expect("variable");
        assert_eq!(cflags.kind, NodeKind::Variable);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"common.mk"),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn cmake_extracts_function_and_commands() {
        let source = r#"function(my_func arg)
  message(STATUS "hi ${arg}")
endfunction()

add_executable(app main.c)
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "CMakeLists.txt", source, "cmake");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let f = find_symbol(&output, "my_func").expect("function");
        assert_eq!(f.kind, NodeKind::Function);

        assert!(!calls_to(&output, "message").is_empty());
        assert!(!calls_to(&output, "add_executable").is_empty());
    }

    #[test]
    fn haskell_extracts_function_class_and_imports() {
        let source = r#"module Main where

import Data.List

data Engine = Engine

class Greeter a where
  greet :: a -> IO ()

run :: Engine -> IO ()
run e = helper e

helper :: Engine -> IO ()
helper _ = putStrLn "hi"
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/Main.hs", source, "haskell");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let run = find_symbol(&output, "run").expect("function");
        assert_eq!(run.kind, NodeKind::Function);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        let engine = find_symbol(&output, "Engine").expect("data type");
        assert_eq!(engine.kind, NodeKind::Class);

        let greeter = find_symbol(&output, "Greeter").expect("class");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.iter().any(|p| p.contains("Data.List")),
            "imports: {import_paths:?}"
        );
    }

    #[test]
    fn julia_extracts_function_struct_module_and_calls() {
        let source = r#"module App

using LinearAlgebra

struct Engine
    name::String
end

function run(e::Engine)
    helper()
    println(e.name)
end

function helper()
    nothing
end

end
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/App.jl", source, "julia");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let module_app = find_symbol(&output, "App").expect("module");
        assert_eq!(module_app.kind, NodeKind::Module);

        let engine = find_symbol(&output, "Engine").expect("struct");
        assert_eq!(engine.kind, NodeKind::Class);

        let run = find_symbol(&output, "run").expect("function");
        assert_eq!(run.kind, NodeKind::Function);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"LinearAlgebra"),
            "imports: {import_paths:?}"
        );

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "println").is_empty());
    }

    #[test]
    fn zig_extracts_function_struct_and_calls() {
        let source = r#"const std = @import("std");

pub const Engine = struct {
    name: []const u8,
};

pub fn run(e: Engine) void {
    helper();
    std.debug.print("{s}\n", .{e.name});
}

fn helper() void {}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/main.zig", source, "zig");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let run = find_symbol(&output, "run").expect("function");
        assert_eq!(run.kind, NodeKind::Function);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "std.debug.print").is_empty());
    }

    #[test]
    fn ocaml_extracts_value_type_module_and_imports() {
        let source = r#"module Engine = struct
  type t = { name : string }

  let run e =
    helper e;
    print_endline e.name

  let helper _ = ()
end

open List
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "src/engine.ml", source, "ocaml");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let engine = find_symbol(&output, "Engine").expect("module");
        assert_eq!(engine.kind, NodeKind::Module);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(import_paths.contains(&"List"), "imports: {import_paths:?}");
    }

    #[test]
    fn swift_extracts_class_function_import_and_calls() {
        let source = r#"import Foundation

protocol Greeter {
    func greet()
}

class Engine {
    init() {}
    func run() {
        helper()
        print("hi")
    }
}

func helper() {}
"#;
        let analyzer = TreeSitterAnalyzer::new();
        let output = analyzer.analyze(REPO, SHA, "Sources/App.swift", source, "swift");
        assert!(output.errors.is_empty(), "errors: {:?}", output.errors);

        let engine = find_symbol(&output, "Engine").expect("class");
        assert_eq!(engine.kind, NodeKind::Class);

        let greeter = find_symbol(&output, "Greeter").expect("protocol");
        assert_eq!(greeter.kind, NodeKind::Interface);

        let run = find_symbol(&output, "run").expect("method");
        assert_eq!(run.kind, NodeKind::Method);

        let helper = find_symbol(&output, "helper").expect("function");
        assert_eq!(helper.kind, NodeKind::Function);

        let init = find_symbol(&output, "init").expect("init");
        assert_eq!(init.kind, NodeKind::Method);

        let import_paths: Vec<_> = output
            .relations
            .iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .filter_map(|r| r.properties.get("import_path").and_then(|v| v.as_str()))
            .collect();
        assert!(
            import_paths.contains(&"Foundation"),
            "imports: {import_paths:?}"
        );

        assert!(!calls_to(&output, "helper").is_empty());
        assert!(!calls_to(&output, "print").is_empty());
    }

    #[cfg(feature = "dynamic-grammars")]
    #[test]
    fn tmp_dynamic_probe() {
        let cases: [(&str, &str, &str); 3] = [
            (
                "styles.scss",
                "scss",
                ".button {\n  color: red;\n}\n\n@mixin big {\n  font-size: 2em;\n}\n",
            ),
            (
                "build.gradle",
                "groovy",
                "def helper() {\n  println 'hi'\n}\n\nclass Engine {\n  def run() {\n    helper()\n  }\n}\n",
            ),
            (
                "src/app.clj",
                "clojure",
                "(ns app.core)\n\n(defn helper [x]\n  x)\n\n(defn run []\n  (helper 1))\n",
            ),
        ];
        for (path, lang, src) in cases {
            if let Some(l) = Lang::resolve(path, lang) {
                let _ = l;
            }
            let pl = tree_sitter_language_pack::get_language(lang).expect("grammar");
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&pl).expect("set");
            let tree = parser.parse(src, None).expect("tree");
            println!("== {lang} SEXP ==\n{}", tree.root_node().to_sexp());
            let out = TreeSitterAnalyzer::new().analyze(REPO, SHA, path, src, lang);
            println!("== {lang} == errors={:?}", out.errors);
            for p in &out.provenance {
                println!("{lang} PROV {:?} {:?}", p.status, p.detail);
            }
            for s in &out.symbols {
                println!("{lang} SYM {} {:?}", s.name, s.kind);
            }
            for r in &out.relations {
                println!("{lang} REL {:?} {:?}", r.kind, r.properties);
            }
        }
    }
}
