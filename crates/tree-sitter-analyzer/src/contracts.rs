use std::collections::{HashMap, HashSet};

use ckg_domain::{
    CanonicalId, ChannelDirection, ChannelRef, ConsumedContract, ContractBatch, Mechanism,
    NodeKind, ProvidedEndpoint, RawSymbol, SourceLocation,
};

const SPRING_ANNOTATIONS: &[(&str, &str)] = &[
    ("@GetMapping(", "GET"),
    ("@PostMapping(", "POST"),
    ("@PutMapping(", "PUT"),
    ("@DeleteMapping(", "DELETE"),
    ("@PatchMapping(", "PATCH"),
];

const NEST_ANNOTATIONS: &[(&str, &str)] = &[
    ("@Get(", "GET"),
    ("@Post(", "POST"),
    ("@Put(", "PUT"),
    ("@Delete(", "DELETE"),
    ("@Patch(", "PATCH"),
];

const CSHARP_ANNOTATIONS: &[(&str, &str)] = &[
    ("[HttpPut(", "PUT"),
    ("[HttpGet(", "GET"),
    ("[HttpPost(", "POST"),
    ("[HttpDelete(", "DELETE"),
    ("[HttpPatch(", "PATCH"),
];

const EXPRESS_RECEIVERS: &[&str] = &["app", "router", "api", "server", "routes", "route"];
const CHI_RECEIVERS: &[&str] = &[
    "r", "router", "mux", "e", "group", "sub", "routes", "route", "api", "app",
];
const GO_CLIENT_RECEIVERS: &[&str] = &[
    "client", "c", "hc", "http", "resty", "req", "s3", "aws", "cl", "s",
];
const HTTP_CLIENT_RECEIVERS: &[&str] = &["axios", "http", "client", "ky"];
const PY_CLIENT_RECEIVERS: &[&str] = &[
    "requests", "session", "sess", "client", "httpx", "aiohttp", "http",
];
const GRPC_RECEIVERS: &[&str] = &["client", "c", "cc", "svc", "conn", "stub"];
const GRPC_COMMON_METHODS: &[&str] = &[
    "get", "post", "put", "delete", "send", "recv", "new", "clone", "ok", "len", "to", "from",
    "eq", "ne", "fmt", "print", "call", "wait", "drop", "iter",
];
const HTTP_VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
const MARKER_TEMPLATES: &[&str] = &["${", "$(", "{{"];

struct FuncSym {
    id: CanonicalId,
    start: u32,
    end: u32,
}

/// Heuristic extraction of service-dependency contracts (routes, RPC
/// endpoints, messaging channel references) from source files.
///
/// Records are keyed by source `file` so the indexer can replace them
/// incrementally; ids mirror the symbol ids emitted by the analyzer walker so
/// the downstream link pass can attach edges to real code nodes.
pub(crate) fn extract_contracts(
    repository: &str,
    commit_sha: &str,
    path: &str,
    content: &str,
    language: &str,
    symbols: &[RawSymbol],
) -> ContractBatch {
    let mut extractor = Extractor::new(repository, commit_sha, path, content, language, symbols);
    extractor.run();
    extractor.batch
}

pub(crate) struct Extractor<'a> {
    pub(crate) repository: &'a str,
    commit_sha: &'a str,
    pub(crate) path: &'a str,
    pub(crate) language: &'a str,
    pub(crate) lower: String,
    pub(crate) lines: Vec<&'a str>,
    funcs: Vec<FuncSym>,
    by_name: HashMap<String, Vec<usize>>,
    pub(crate) batch: ContractBatch,
    pub(crate) seen: HashSet<String>,
    pub(crate) file_flags: HashMap<String, bool>,
}

impl<'a> Extractor<'a> {
    fn new(
        repository: &'a str,
        commit_sha: &'a str,
        path: &'a str,
        content: &'a str,
        language: &'a str,
        symbols: &'a [RawSymbol],
    ) -> Self {
        let mut funcs = Vec::new();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for symbol in symbols {
            if !matches!(symbol.kind, NodeKind::Function | NodeKind::Method) {
                continue;
            }
            let Some(loc) = &symbol.location else {
                continue;
            };
            let idx = funcs.len();
            by_name.entry(symbol.name.clone()).or_default().push(idx);
            funcs.push(FuncSym {
                id: symbol.canonical_id.clone(),
                start: loc.start_line,
                end: loc.end_line,
            });
        }
        Self {
            repository,
            commit_sha,
            path,
            language,
            lower: content.to_ascii_lowercase(),
            lines: content.lines().collect(),
            funcs,
            by_name,
            batch: ContractBatch::default(),
            seen: HashSet::new(),
            file_flags: HashMap::new(),
        }
    }

    pub(crate) fn run(&mut self) {
        if self.language == "proto" {
            self.proto_grpc_provides();
        }
        self.rest_provides();
        self.rest_consumes();
        self.realtime();
        self.messaging();
        self.grpc_consumes();
        self.jsonrpc();
        self.resources();
    }

    pub(crate) fn loc(&self, line: u32) -> SourceLocation {
        SourceLocation::new(
            self.repository,
            self.commit_sha,
            self.path,
            line,
            1,
            line,
            1,
        )
    }

    pub(crate) fn enclosing(&self, line: u32) -> Option<CanonicalId> {
        self.funcs
            .iter()
            .filter(|f| f.start <= line && line <= f.end)
            .max_by_key(|f| f.start)
            .map(|f| f.id.clone())
    }

    pub(crate) fn next_func(&self, line: u32) -> Option<CanonicalId> {
        self.funcs
            .iter()
            .filter(|f| f.start >= line && f.start <= line + 12)
            .min_by_key(|f| f.start)
            .map(|f| f.id.clone())
    }

    fn lookup_name(&self, name: &str) -> Option<CanonicalId> {
        let idx = *self.by_name.get(name)?.first()?;
        Some(self.funcs[idx].id.clone())
    }

    fn range_of(&self, id: &CanonicalId) -> Option<(u32, u32)> {
        self.funcs
            .iter()
            .find(|f| f.id == *id)
            .map(|f| (f.start, f.end))
    }

    fn body_is_sse(&self, handler: &Option<CanonicalId>) -> bool {
        let Some(id) = handler else {
            return false;
        };
        let Some((start, end)) = self.range_of(id) else {
            return false;
        };
        let start = start.max(1) as usize;
        let end = (end as usize).min(self.lines.len());
        if start > end {
            return false;
        }
        self.lines[start - 1..end]
            .iter()
            .any(|line| line.to_ascii_lowercase().contains("text/event-stream"))
    }

    fn resolve_handler(
        &self,
        line: u32,
        hint: Option<&str>,
        allow_next: bool,
    ) -> Option<CanonicalId> {
        if let Some(name) = hint
            && !name.is_empty()
            && let Some(id) = self.lookup_name(name)
        {
            return Some(id);
        }
        if let Some(id) = self.enclosing(line) {
            return Some(id);
        }
        if allow_next {
            return self.next_func(line);
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn add_provide(
        &mut self,
        line: u32,
        mechanism: Mechanism,
        http_method: &str,
        route: &str,
        service: &str,
        method: &str,
        hint: Option<&str>,
        allow_next: bool,
        framework: &str,
    ) {
        if route.trim().is_empty() && method.trim().is_empty() {
            return;
        }
        if route.contains('$') || route.contains("{{") {
            return;
        }
        let handler = self.resolve_handler(line, hint, allow_next);
        let mut mechanism = mechanism;
        if mechanism == Mechanism::Rest && self.body_is_sse(&handler) {
            mechanism = Mechanism::Sse;
        }
        let mech = mechanism.as_str();
        let handler_str = handler.as_ref().map(|h| h.as_str()).unwrap_or("");
        let key = format!("p|{mech}|{http_method}|{route}|{service}|{method}|{handler_str}");
        if !self.seen.insert(key) {
            return;
        }
        let handler_id = handler.unwrap_or_default();
        self.batch.provides.push(ProvidedEndpoint {
            file: self.path.to_string(),
            mechanism,
            http_method: http_method.to_ascii_uppercase(),
            route: route.to_string(),
            service: service.to_string(),
            method: method.to_string(),
            handler_id,
            framework: framework.to_string(),
            language: self.language.to_string(),
            location: Some(self.loc(line)),
            properties: Default::default(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn add_consume(
        &mut self,
        line: u32,
        mechanism: Mechanism,
        http_method: &str,
        route: &str,
        service: &str,
        method: &str,
        target_hint: &str,
        framework: &str,
    ) {
        let route = strip_template_prefix(route);
        if MARKER_TEMPLATES.iter().any(|m| route.starts_with(m)) {
            return;
        }
        let route_required = matches!(
            mechanism,
            Mechanism::Rest | Mechanism::WebSocket | Mechanism::Sse
        );
        if route_required && route.trim().is_empty() {
            return;
        }
        if route.trim().is_empty() && method.trim().is_empty() && service.trim().is_empty() {
            return;
        }
        let caller = self.enclosing(line).unwrap_or_default();
        let mech = mechanism.as_str();
        let caller_str = caller.as_str();
        let key =
            format!("c|{mech}|{http_method}|{route}|{service}|{method}|{target_hint}|{caller_str}");
        if !self.seen.insert(key) {
            return;
        }
        self.batch.consumes.push(ConsumedContract {
            file: self.path.to_string(),
            mechanism,
            http_method: http_method.to_ascii_uppercase(),
            route: route.to_string(),
            service: service.to_string(),
            method: method.to_string(),
            caller_id: caller,
            target_hint: target_hint.to_string(),
            framework: framework.to_string(),
            language: self.language.to_string(),
            location: Some(self.loc(line)),
            properties: Default::default(),
        });
    }

    fn add_channel(
        &mut self,
        line: u32,
        direction: ChannelDirection,
        broker: &str,
        channel: &str,
        channel_type: &str,
    ) {
        let channel = channel.trim();
        if channel.is_empty() || MARKER_TEMPLATES.iter().any(|m| channel.contains(m)) {
            return;
        }
        let node = self.enclosing(line).unwrap_or_default();
        let key = format!("ch|{broker}|{channel}|{direction:?}|{channel_type}|{node}");
        if !self.seen.insert(key) {
            return;
        }
        self.batch.channels.push(ChannelRef {
            file: self.path.to_string(),
            direction,
            broker: broker.to_string(),
            channel_type: channel_type.to_string(),
            channel: channel.to_string(),
            routing_key: String::new(),
            node_id: node,
            language: self.language.to_string(),
            location: Some(self.loc(line)),
            properties: Default::default(),
        });
    }

    // ---------------------------------------------------------------- REST
    // provides

    fn rest_provides(&mut self) {
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            match self.language {
                "java" | "kotlin" => self.spring_provides(lno, line),
                "python" => self.py_decorator_provides(lno, line),
                "javascript" | "typescript" => {
                    self.nest_provides(lno, line);
                    self.express_provides(lno, line);
                }
                "go" => self.go_provides(lno, line),
                "rust" => self.rust_provides(lno, line),
                "ruby" => self.ruby_provides(lno, line),
                "csharp" => self.csharp_provides(lno, line),
                _ => {}
            }
        }
    }

    fn spring_provides(&mut self, lno: u32, line: &str) {
        for (annotation, method) in SPRING_ANNOTATIONS {
            if let Some(idx) = line.find(annotation) {
                let route = first_quoted(&line[idx + annotation.len()..]).unwrap_or_default();
                if !route.is_empty() {
                    self.add_provide(
                        lno,
                        Mechanism::Rest,
                        method,
                        &route,
                        "",
                        "",
                        None,
                        true,
                        "spring",
                    );
                }
                return;
            }
        }
        if let Some(idx) = line.find("@RequestMapping(") {
            let tail = &line[idx + "@RequestMapping(".len()..];
            let route = match first_quoted(tail) {
                Some(route) => route,
                None => return,
            };
            let method = if line.contains("RequestMethod.POST") {
                "POST"
            } else if line.contains("RequestMethod.PUT") {
                "PUT"
            } else if line.contains("RequestMethod.DELETE") {
                "DELETE"
            } else if line.contains("RequestMethod.PATCH") {
                "PATCH"
            } else if line.contains("RequestMethod.GET") {
                "GET"
            } else {
                ""
            };
            self.add_provide(
                lno,
                Mechanism::Rest,
                method,
                &route,
                "",
                "",
                None,
                true,
                "spring",
            );
        }
    }

    fn py_decorator_provides(&mut self, lno: u32, line: &str) {
        let trimmed = line.trim();
        if !trimmed.starts_with('@') {
            return;
        }
        let Some(paren) = trimmed.find('(') else {
            return;
        };
        let path = &trimmed[1..paren];
        if path.is_empty()
            || !path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return;
        }
        let verb = path.rsplit('.').next().unwrap_or("");
        let verb_lower = verb.to_ascii_lowercase();
        let (mut method, is_route) = match verb_lower.as_str() {
            "get" | "head" => ("GET", false),
            "post" => ("POST", false),
            "put" => ("PUT", false),
            "delete" => ("DELETE", false),
            "patch" => ("PATCH", false),
            "route" => ("GET", true),
            _ => return,
        };
        let tail = &trimmed[paren + 1..];
        let Some(route) = first_quoted(tail) else {
            return;
        };
        if is_route && let Some(mi) = tail.find("methods") {
            let mtail = &tail[mi..];
            method = ["POST", "PUT", "DELETE", "PATCH", "GET"]
                .iter()
                .find(|v| mtail.contains(*v))
                .copied()
                .unwrap_or("GET");
        }
        let framework = path
            .rsplit_once('.')
            .map(|(receiver, _)| receiver)
            .unwrap_or(path)
            .to_string();
        self.add_provide(
            lno,
            Mechanism::Rest,
            method,
            &route,
            "",
            "",
            None,
            true,
            &framework,
        );
    }

    fn nest_provides(&mut self, lno: u32, line: &str) {
        for (annotation, method) in NEST_ANNOTATIONS {
            if let Some(idx) = line.find(annotation) {
                let route = first_quoted(&line[idx + annotation.len()..]).unwrap_or_default();
                if !route.is_empty() {
                    self.add_provide(
                        lno,
                        Mechanism::Rest,
                        method,
                        &route,
                        "",
                        "",
                        None,
                        true,
                        "nestjs",
                    );
                }
                return;
            }
        }
    }

    fn express_provides(&mut self, lno: u32, line: &str) {
        for verb in ["get", "post", "put", "delete", "patch"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !EXPRESS_RECEIVERS.contains(&last.as_str()) {
                continue;
            }
            let Some(route) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if route.is_empty() {
                continue;
            }
            let hint = handler_hint(line, &route);
            self.add_provide(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &route,
                "",
                "",
                hint.as_deref(),
                false,
                "express",
            );
            return;
        }
    }

    fn go_provides(&mut self, lno: u32, line: &str) {
        for verb in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if GO_CLIENT_RECEIVERS.contains(&last.as_str()) {
                continue;
            }
            let Some(route) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if route.is_empty() {
                continue;
            }
            let hint = handler_hint(line, &route);
            self.add_provide(
                lno,
                Mechanism::Rest,
                verb,
                &route,
                "",
                "",
                hint.as_deref(),
                false,
                "gin",
            );
            return;
        }
        for verb in ["Get", "Post", "Put", "Delete", "Patch"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !CHI_RECEIVERS.contains(&last.as_str()) {
                continue;
            }
            let Some(route) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if route.is_empty() {
                continue;
            }
            let hint = handler_hint(line, &route);
            self.add_provide(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &route,
                "",
                "",
                hint.as_deref(),
                false,
                "chi",
            );
            return;
        }
        if let Some(idx) = line.find("HandleFunc(") {
            let before = if idx == 0 { "" } else { &line[..idx] };
            let recv_end = before.trim_end().len();
            let head = &before[..recv_end];
            let recv = last_segment(head).to_ascii_lowercase();
            if recv == "http" || recv == "mux" || recv == "gorilla" || recv == "r" {
                let route = first_quoted(&line[idx + "HandleFunc(".len()..]).unwrap_or_default();
                if !route.is_empty() {
                    let hint = handler_hint(line, &route);
                    self.add_provide(
                        lno,
                        Mechanism::Rest,
                        "",
                        &route,
                        "",
                        "",
                        hint.as_deref(),
                        false,
                        "net/http",
                    );
                }
            }
        }
    }

    fn rust_provides(&mut self, lno: u32, line: &str) {
        for verb in ["get", "post", "put", "delete", "patch"] {
            let pat = format!("[{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            if !line[..idx].ends_with('#') {
                continue;
            }
            let route = first_quoted(&line[idx + pat.len()..]).unwrap_or_default();
            if route.is_empty() {
                continue;
            }
            self.add_provide(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &route,
                "",
                "",
                None,
                true,
                "axum",
            );
            return;
        }
        if let Some(idx) = line.find("#[route(") {
            let tail = &line[idx + "#[route(".len()..];
            let route = first_quoted(tail).unwrap_or_default();
            if !route.is_empty() {
                let method = if line.contains("method = \"POST\"") {
                    "POST"
                } else if line.contains("method = \"PUT\"") {
                    "PUT"
                } else if line.contains("method = \"DELETE\"") {
                    "DELETE"
                } else if line.contains("method = \"GET\"") {
                    "GET"
                } else {
                    ""
                };
                self.add_provide(
                    lno,
                    Mechanism::Rest,
                    method,
                    &route,
                    "",
                    "",
                    None,
                    true,
                    "actix",
                );
            }
            return;
        }
        if let Some(idx) = line.find(".route(") {
            let tail = &line[idx + ".route(".len()..];
            let Some(route) = first_quoted(tail) else {
                return;
            };
            if route.is_empty() {
                return;
            }
            let mut method = "";
            let mut hint = None;
            for verb in ["post", "put", "patch", "delete", "get"] {
                let call = format!("{verb}(");
                if let Some(vi) = tail.find(&call) {
                    method = match verb {
                        "post" => "POST",
                        "put" => "PUT",
                        "patch" => "PATCH",
                        "delete" => "DELETE",
                        _ => "GET",
                    };
                    hint = tail[vi + call.len()..]
                        .trim_start_matches('(')
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect::<String>()
                        .into();
                    if hint.as_deref().is_some_and(|h| h.is_empty()) {
                        hint = None;
                    }
                    break;
                }
            }
            self.add_provide(
                lno,
                Mechanism::Rest,
                method,
                &route,
                "",
                "",
                hint.as_deref(),
                false,
                "axum",
            );
        }
    }

    fn ruby_provides(&mut self, lno: u32, line: &str) {
        let trimmed = line.trim_start();
        for verb in ["get", "post", "put", "delete", "patch"] {
            let Some(rest) = trimmed.strip_prefix(verb) else {
                continue;
            };
            let next = rest
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '`');
            if !next {
                continue;
            }
            let Some(route) = first_quoted(rest) else {
                continue;
            };
            if route.is_empty() {
                continue;
            }
            self.add_provide(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &route,
                "",
                "",
                None,
                false,
                "sinatra",
            );
            return;
        }
    }

    fn csharp_provides(&mut self, lno: u32, line: &str) {
        for (annotation, method) in CSHARP_ANNOTATIONS {
            if let Some(idx) = line.find(annotation) {
                let route = first_quoted(&line[idx + annotation.len()..]).unwrap_or_default();
                if !route.is_empty() {
                    self.add_provide(
                        lno,
                        Mechanism::Rest,
                        method,
                        &route,
                        "",
                        "",
                        None,
                        true,
                        "aspnetcore",
                    );
                }
                return;
            }
        }
    }

    // ------------------------------------------------------------ REST
    // consumes

    fn rest_consumes(&mut self) {
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            match self.language {
                "javascript" | "typescript" => self.js_consumes(lno, line),
                "python" => self.py_consumes(lno, line),
                "go" => self.go_consumes(lno, line),
                "rust" => self.rust_consumes(lno, line),
                "java" | "kotlin" => self.java_consumes(lno, line),
                "ruby" => self.ruby_consumes(lno, line),
                _ => {}
            }
        }
    }

    fn js_consumes(&mut self, lno: u32, line: &str) {
        if let Some(idx) = word_find(line, "fetch(")
            && let Some(url) = first_quoted(&line[idx + "fetch(".len()..])
        {
            let method = line
                .find("method")
                .and_then(|mi| first_quoted(&line[mi + "method".len()..]))
                .filter(|m| HTTP_VERBS.contains(&m.to_ascii_uppercase().as_str()))
                .unwrap_or_else(|| "GET".to_string());
            let hint = target_hint(&url);
            self.add_consume(lno, Mechanism::Rest, &method, &url, "", "", &hint, "fetch");
        }
        for verb in ["get", "post", "put", "delete", "patch"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !HTTP_CLIENT_RECEIVERS.contains(&last.as_str()) {
                continue;
            }
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &url,
                "",
                "",
                &hint,
                &last,
            );
        }
        if line.contains("axios") && line.contains("url:") {
            let Some(url_idx) = line.find("url:") else {
                return;
            };
            let Some(url) = first_quoted(&line[url_idx + 4..]) else {
                return;
            };
            if url.is_empty() {
                return;
            }
            let method = line
                .find("method:")
                .and_then(|mi| first_quoted(&line[mi + 7..]))
                .filter(|m| HTTP_VERBS.contains(&m.to_ascii_uppercase().as_str()))
                .unwrap_or_else(|| "GET".to_string());
            let hint = target_hint(&url);
            self.add_consume(lno, Mechanism::Rest, &method, &url, "", "", &hint, "axios");
        }
    }

    fn py_consumes(&mut self, lno: u32, line: &str) {
        for verb in ["get", "post", "put", "delete", "patch"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !PY_CLIENT_RECEIVERS.contains(&last.as_str()) {
                continue;
            }
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &url,
                "",
                "",
                &hint,
                &last,
            );
        }
        if let Some(idx) = word_find(line, "urlopen(")
            && let Some(url) = first_quoted(&line[idx + "urlopen(".len()..])
        {
            let hint = target_hint(&url);
            self.add_consume(lno, Mechanism::Rest, "GET", &url, "", "", &hint, "urllib");
        }
    }

    fn go_consumes(&mut self, lno: u32, line: &str) {
        for (pat, method) in [
            (".Get(", "GET"),
            (".Post(", "POST"),
            (".Head(", "HEAD"),
            (".Do(", ""),
        ] {
            let Some(idx) = line.find(pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !["http", "client", "hc", "c", "resty"].contains(&last.as_str()) {
                continue;
            }
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() || !url.starts_with('/') && !url.contains("://") {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                method,
                &url,
                "",
                "",
                &hint,
                "net/http",
            );
        }
        if let Some(idx) = line.find("NewRequest(") {
            let quoted: Vec<String> = quoted_strings(&line[idx + "NewRequest(".len()..]);
            if quoted.len() >= 2 {
                let method = quoted[0].to_ascii_uppercase();
                if HTTP_VERBS.contains(&method.as_str()) {
                    let hint = target_hint(&quoted[1]);
                    self.add_consume(
                        lno,
                        Mechanism::Rest,
                        &method,
                        &quoted[1],
                        "",
                        "",
                        &hint,
                        "net/http",
                    );
                }
            }
        }
    }

    fn rust_consumes(&mut self, lno: u32, line: &str) {
        if !self.lower.contains("reqwest") {
            return;
        }
        if let Some(idx) = word_find(line, "reqwest::get(")
            && let Some(url) = first_quoted(&line[idx + "reqwest::get(".len()..])
        {
            let hint = target_hint(&url);
            self.add_consume(lno, Mechanism::Rest, "GET", &url, "", "", &hint, "reqwest");
        }
        for verb in ["get", "post"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            let last = last_segment(&receiver).to_ascii_lowercase();
            if !["client", "c", "hc", "http"].contains(&last.as_str()) {
                continue;
            }
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &url,
                "",
                "",
                &hint,
                "reqwest",
            );
        }
    }

    fn java_consumes(&mut self, lno: u32, line: &str) {
        for (pat, method) in [
            (".getForObject(", "GET"),
            (".getForEntity(", "GET"),
            (".postForObject(", "POST"),
            (".postForEntity(", "POST"),
            (".exchange(", ""),
        ] {
            let Some(idx) = line.find(pat) else {
                continue;
            };
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let method = if method.is_empty() {
                line.find("HttpMethod.")
                    .and_then(|mi| {
                        line[mi + "HttpMethod.".len()..]
                            .chars()
                            .take_while(|c| c.is_ascii_alphabetic())
                            .collect::<String>()
                            .to_ascii_uppercase()
                            .into()
                    })
                    .filter(|m| HTTP_VERBS.contains(&m.as_str()))
                    .unwrap_or_default()
            } else {
                method.to_string()
            };
            if !method.is_empty() && !HTTP_VERBS.contains(&method.as_str()) {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                &method,
                &url,
                "",
                "",
                &hint,
                "http-client",
            );
        }
        for (pat, method) in [
            (".get().uri(", "GET"),
            (".post().uri(", "POST"),
            (".put().uri(", "PUT"),
        ] {
            if let Some(idx) = line.find(pat)
                && let Some(url) = first_quoted(&line[idx + pat.len()..])
                && !url.is_empty()
            {
                let hint = target_hint(&url);
                self.add_consume(
                    lno,
                    Mechanism::Rest,
                    method,
                    &url,
                    "",
                    "",
                    &hint,
                    "webclient",
                );
            }
        }
    }

    fn ruby_consumes(&mut self, lno: u32, line: &str) {
        for verb in ["get", "post", "put", "delete", "patch"] {
            let pat = format!(".{verb}(");
            let Some(idx) = line.find(&pat) else {
                continue;
            };
            let Some(receiver) = receiver_before(line, idx) else {
                continue;
            };
            if !(receiver.starts_with("HTTP") || receiver == "RestClient") {
                continue;
            }
            let Some(url) = first_quoted(&line[idx + pat.len()..]) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Rest,
                &verb.to_uppercase(),
                &url,
                "",
                "",
                &hint,
                &receiver,
            );
        }
    }

    // ---------------------------------------------------------- realtime
    // (WebSocket / SSE)

    fn realtime(&mut self) {
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            match self.language {
                "javascript" | "typescript" => self.js_realtime(lno, line),
                "python" => self.py_realtime(lno, line),
                _ => {}
            }
        }
    }

    fn js_realtime(&mut self, lno: u32, line: &str) {
        for (pat, _) in [(".ws(", "websocket"), ("app.ws(", "websocket")] {
            if let Some(idx) = line.find(pat) {
                let Some(receiver) = receiver_before(line, idx) else {
                    continue;
                };
                let last = last_segment(&receiver).to_ascii_lowercase();
                if !["app", "router", "server"].contains(&last.as_str()) {
                    continue;
                }
                if let Some(route) = first_quoted(&line[idx + pat.len()..])
                    && !route.is_empty()
                {
                    self.add_provide(
                        lno,
                        Mechanism::WebSocket,
                        "",
                        &route,
                        "",
                        "",
                        None,
                        true,
                        "express-ws",
                    );
                }
                break;
            }
        }
        if (line.contains("WebSocketServer") || line.contains("WebSocket.Server"))
            && let Some(pi) = line.find("path")
            && let Some(route) = first_quoted(&line[pi + "path".len()..])
            && !route.is_empty()
        {
            self.add_provide(
                lno,
                Mechanism::WebSocket,
                "",
                &route,
                "",
                "",
                None,
                false,
                "ws",
            );
        }
        if let Some(idx) = word_find(line, "new WebSocket(")
            && let Some(url) = first_quoted(&line[idx + "new WebSocket(".len()..])
        {
            let route = url_path(&url);
            let hint = target_hint(&url);
            self.add_consume(lno, Mechanism::WebSocket, "", &route, "", "", &hint, "ws");
        }
        if let Some(idx) = word_find(line, "EventSource(")
            && let Some(url) = first_quoted(&line[idx + "EventSource(".len()..])
        {
            let route = url_path(&url);
            let hint = target_hint(&url);
            self.add_consume(
                lno,
                Mechanism::Sse,
                "GET",
                &route,
                "",
                "",
                &hint,
                "eventsource",
            );
        }
        for pat in ["io.connect(", "io("] {
            if let Some(idx) = word_find(line, pat) {
                let after = &line[idx + pat.len()..];
                if let Some(url) = first_quoted(after)
                    && url.contains("://")
                {
                    let route = url_path(&url);
                    let hint = target_hint(&url);
                    self.add_consume(
                        lno,
                        Mechanism::WebSocket,
                        "",
                        &route,
                        "",
                        "",
                        &hint,
                        "socket.io",
                    );
                }
                break;
            }
        }
    }

    fn py_realtime(&mut self, lno: u32, line: &str) {
        let trimmed = line.trim();
        if trimmed.starts_with('@')
            && trimmed.contains(".websocket(")
            && let Some(idx) = trimmed.find(".websocket(")
            && let Some(route) = first_quoted(&trimmed[idx + ".websocket(".len()..])
            && !route.is_empty()
        {
            self.add_provide(
                lno,
                Mechanism::WebSocket,
                "",
                &route,
                "",
                "",
                None,
                true,
                "fastapi",
            );
        }
        for pat in ["websockets.connect(", "websocket.connect(", ".connect_ws("] {
            if let Some(idx) = line.find(pat)
                && let Some(url) = first_quoted(&line[idx + pat.len()..])
            {
                let route = url_path(&url);
                let hint = target_hint(&url);
                self.add_consume(
                    lno,
                    Mechanism::WebSocket,
                    "",
                    &route,
                    "",
                    "",
                    &hint,
                    "websockets",
                );
            }
        }
    }

    // --------------------------------------------------------- messaging

    fn messaging(&mut self) {
        let kafka = self.lower.contains("kafka");
        let rabbit = self.lower.contains("rabbit") || self.lower.contains("amqp");
        let sqs = self.lower.contains("sqs");
        let pubsub = self.lower.contains("pubsub");
        if !kafka && !rabbit && !sqs && !pubsub {
            return;
        }
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            if kafka {
                self.kafka_channels(lno, line);
            }
            if rabbit {
                self.rabbit_channels(lno, line);
            }
            if sqs {
                self.sqs_channels(lno, line);
            }
            if pubsub {
                self.pubsub_channels(lno, line);
            }
        }
    }

    fn kafka_channels(&mut self, lno: u32, line: &str) {
        match self.language {
            "java" | "kotlin" => {
                for pat in ["kafkaTemplate.send(", "new ProducerRecord<>("] {
                    if let Some(idx) = line.find(pat) {
                        if let Some(channel) = first_quoted(&line[idx + pat.len()..]) {
                            self.add_channel(
                                lno,
                                ChannelDirection::Publish,
                                "kafka",
                                &channel,
                                "topic",
                            );
                        }
                        return;
                    }
                }
                if let Some(idx) = line.find("@KafkaListener(")
                    && let Some(channel) = first_quoted(&line[idx + "@KafkaListener(".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Subscribe, "kafka", &channel, "topic");
                }
            }
            "javascript" | "typescript" => {
                if line.contains("subscribe(") {
                    if let Some(ti) = line.find("topic")
                        && let Some(channel) = first_quoted(&line[ti + "topic".len()..])
                    {
                        self.add_channel(
                            lno,
                            ChannelDirection::Subscribe,
                            "kafka",
                            &channel,
                            "topic",
                        );
                    }
                } else if (line.contains("send(") || line.contains("produce("))
                    && let Some(ti) = line.find("topic")
                    && let Some(channel) = first_quoted(&line[ti + "topic".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Publish, "kafka", &channel, "topic");
                }
            }
            "python" => {
                for pat in ["KafkaConsumer(", "AIOKafkaConsumer(", "AIOKafkaConsumer("] {
                    if let Some(idx) = line.find(pat) {
                        if let Some(channel) = first_quoted(&line[idx + pat.len()..]) {
                            self.add_channel(
                                lno,
                                ChannelDirection::Subscribe,
                                "kafka",
                                &channel,
                                "topic",
                            );
                        }
                        return;
                    }
                }
                if let Some(idx) = line.find(".subscribe(") {
                    if let Some(channel) = first_quoted(&line[idx + ".subscribe(".len()..]) {
                        self.add_channel(
                            lno,
                            ChannelDirection::Subscribe,
                            "kafka",
                            &channel,
                            "topic",
                        );
                    }
                    return;
                }
                if let Some(idx) = line.find(".send(")
                    && let Some(channel) = first_quoted(&line[idx + ".send(".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Publish, "kafka", &channel, "topic");
                }
            }
            "go" => {
                if line.contains("Topic:") && line.contains("Producer") {
                    if let Some(ti) = line.find("Topic:")
                        && let Some(channel) = first_quoted(&line[ti + "Topic:".len()..])
                    {
                        self.add_channel(
                            lno,
                            ChannelDirection::Publish,
                            "kafka",
                            &channel,
                            "topic",
                        );
                    }
                } else if line.contains("Consume(")
                    && let Some(idx) = line.find("Consume(")
                    && let Some(channel) = first_quoted(&line[idx + "Consume(".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Subscribe, "kafka", &channel, "topic");
                }
            }
            "rust" => {
                if let Some(idx) = line.find(".send(")
                    && let Some(channel) = first_quoted(&line[idx + ".send(".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Publish, "kafka", &channel, "topic");
                }
            }
            _ => {}
        }
    }

    fn rabbit_channels(&mut self, lno: u32, line: &str) {
        match self.language {
            "java" | "kotlin" => {
                if let Some(idx) = line.find("convertAndSend(") {
                    let quoted = quoted_strings(&line[idx + "convertAndSend(".len()..]);
                    let channel = quoted.get(1).or_else(|| quoted.first());
                    if let Some(channel) = channel {
                        self.add_channel(
                            lno,
                            ChannelDirection::Publish,
                            "rabbitmq",
                            channel,
                            "queue",
                        );
                    }
                    return;
                }
                for pat in ["@RabbitListener(", "rabbitTemplate.receive("] {
                    if let Some(idx) = line.find(pat) {
                        if let Some(channel) = first_quoted(&line[idx + pat.len()..]) {
                            self.add_channel(
                                lno,
                                ChannelDirection::Subscribe,
                                "rabbitmq",
                                &channel,
                                "queue",
                            );
                        }
                        return;
                    }
                }
            }
            "javascript" | "typescript" => {
                if let Some(idx) = line.find("sendToQueue(") {
                    if let Some(channel) = first_quoted(&line[idx + "sendToQueue(".len()..]) {
                        self.add_channel(
                            lno,
                            ChannelDirection::Publish,
                            "rabbitmq",
                            &channel,
                            "queue",
                        );
                    }
                    return;
                }
                if let Some(idx) = line.find("publish(") {
                    let quoted = quoted_strings(&line[idx + "publish(".len()..]);
                    let channel = quoted.get(1).or_else(|| quoted.first());
                    if let Some(channel) = channel {
                        self.add_channel(
                            lno,
                            ChannelDirection::Publish,
                            "rabbitmq",
                            channel,
                            "queue",
                        );
                    }
                    return;
                }
                if let Some(idx) = line.find("consume(")
                    && let Some(channel) = first_quoted(&line[idx + "consume(".len()..])
                {
                    self.add_channel(
                        lno,
                        ChannelDirection::Subscribe,
                        "rabbitmq",
                        &channel,
                        "queue",
                    );
                }
            }
            "python" => {
                if let Some(idx) = line.find("basic_publish(") {
                    let quoted = quoted_strings(&line[idx + "basic_publish(".len()..]);
                    if quoted.len() >= 2 {
                        self.add_channel(
                            lno,
                            ChannelDirection::Publish,
                            "rabbitmq",
                            &quoted[1],
                            "queue",
                        );
                    }
                    return;
                }
                if let Some(idx) = line.find("basic_consume(") {
                    let tail = &line[idx + "basic_consume(".len()..];
                    let channel = tail
                        .find("queue")
                        .and_then(|qi| first_quoted(&tail[qi..]))
                        .or_else(|| first_quoted(tail));
                    if let Some(channel) = channel {
                        self.add_channel(
                            lno,
                            ChannelDirection::Subscribe,
                            "rabbitmq",
                            &channel,
                            "queue",
                        );
                    }
                }
            }
            "go" => {
                if line.contains("Publish(") {
                    if let Some(idx) = line.find("Publish(") {
                        let quoted = quoted_strings(&line[idx + "Publish(".len()..]);
                        let channel = quoted.get(1).or_else(|| quoted.first());
                        if let Some(channel) = channel {
                            self.add_channel(
                                lno,
                                ChannelDirection::Publish,
                                "rabbitmq",
                                channel,
                                "queue",
                            );
                        }
                    }
                } else if let Some(idx) = line.find("Consume(")
                    && let Some(channel) = first_quoted(&line[idx + "Consume(".len()..])
                {
                    self.add_channel(
                        lno,
                        ChannelDirection::Subscribe,
                        "rabbitmq",
                        &channel,
                        "queue",
                    );
                }
            }
            _ => {}
        }
    }

    fn sqs_channels(&mut self, lno: u32, line: &str) {
        let lower = line.to_ascii_lowercase();
        let is_send = lower.contains("sendmessage") || lower.contains("send_message");
        let is_recv = lower.contains("receivemessage") || lower.contains("receive_message");
        if !is_send && !is_recv {
            return;
        }
        let Some(qi) = lower.find("queueurl") else {
            return;
        };
        let Some(raw) = first_quoted(&line[qi + "queueurl".len()..]) else {
            return;
        };
        let channel = tail_segment(&raw);
        if channel.is_empty() {
            return;
        }
        let direction = if is_send {
            ChannelDirection::Publish
        } else {
            ChannelDirection::Subscribe
        };
        self.add_channel(lno, direction, "sqs", &channel, "queue");
    }

    fn pubsub_channels(&mut self, lno: u32, line: &str) {
        match self.language {
            "python" => {
                for (pat, direction) in [
                    (".topic_path(", ChannelDirection::Publish),
                    (".subscription_path(", ChannelDirection::Subscribe),
                ] {
                    if let Some(idx) = line.find(pat) {
                        let quoted = quoted_strings(&line[idx + pat.len()..]);
                        if let Some(channel) = quoted.last() {
                            let channel = tail_segment(channel);
                            self.add_channel(lno, direction, "pubsub", &channel, "topic");
                            return;
                        }
                    }
                }
                if let Some(idx) = line.find("publisher.publish(")
                    && let Some(url) = first_quoted(&line[idx + "publisher.publish(".len()..])
                    && let Some(topic) = url.split("topics/").nth(1)
                {
                    let channel = tail_segment(topic);
                    self.add_channel(lno, ChannelDirection::Publish, "pubsub", &channel, "topic");
                }
            }
            "javascript" | "typescript" => {
                if let Some(ti) = line.find(".topic(")
                    && line.contains("publish")
                    && let Some(channel) = first_quoted(&line[ti + ".topic(".len()..])
                {
                    self.add_channel(lno, ChannelDirection::Publish, "pubsub", &channel, "topic");
                }
                if let Some(si) = line.find("subscription(")
                    && let Some(channel) = first_quoted(&line[si + "subscription(".len()..])
                {
                    self.add_channel(
                        lno,
                        ChannelDirection::Subscribe,
                        "pubsub",
                        &channel,
                        "topic",
                    );
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------- gRPC

    fn proto_grpc_provides(&mut self) {
        let mut package = String::new();
        let mut service = String::new();
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            let trimmed = line.trim();
            if trimmed.starts_with("package ") {
                package = trimmed
                    .trim_start_matches("package ")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                continue;
            }
            if trimmed.starts_with("service ") && trimmed.contains('{') {
                service = trimmed
                    .trim_start_matches("service ")
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                continue;
            }
            if trimmed == "}" {
                service.clear();
                continue;
            }
            if trimmed.starts_with("rpc ") && !service.is_empty() {
                let rest = trimmed.trim_start_matches("rpc ");
                let method = rest
                    .split(['(', ' ', '\t'])
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches('{');
                if method.is_empty() {
                    continue;
                }
                let full_service = if package.is_empty() {
                    service.clone()
                } else {
                    format!("{package}.{service}")
                };
                self.add_provide(
                    lno,
                    Mechanism::Grpc,
                    "",
                    "",
                    &full_service,
                    method,
                    Some(method),
                    false,
                    "grpc",
                );
            }
        }
    }

    fn grpc_consumes(&mut self) {
        let gate = self.lower.contains("grpc")
            || self.lower.contains("_pb2")
            || self.lower.contains("tonic");
        if !gate {
            return;
        }
        if !matches!(
            self.language,
            "java" | "kotlin" | "python" | "javascript" | "typescript" | "go" | "rust" | "csharp"
        ) {
            return;
        }
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            for (receiver, method, _) in call_sites(line) {
                let recv = receiver.to_ascii_lowercase();
                let is_stub = recv.contains("stub");
                if !is_stub && !GRPC_RECEIVERS.contains(&recv.as_str()) {
                    continue;
                }
                if GRPC_COMMON_METHODS.contains(&method.to_ascii_lowercase().as_str()) {
                    continue;
                }
                self.add_consume(lno, Mechanism::Grpc, "", "", "", &method, "", "grpc");
            }
        }
    }

    // --------------------------------------------------------- JSON-RPC

    fn jsonrpc(&mut self) {
        let gate = self.lower.contains("jsonrpc") || self.lower.contains("json-rpc");
        if !gate {
            return;
        }
        for i in 0..self.lines.len() {
            let line = self.lines[i];
            let lno = i as u32 + 1;
            for pat in [
                "add_method(",
                "addMethod(",
                "create_method(",
                "register_method(",
                "server.method(",
            ] {
                if let Some(idx) = line.find(pat) {
                    if let Some(method) = first_quoted(&line[idx + pat.len()..])
                        && !method.is_empty()
                    {
                        let hint = handler_hint(line, &method);
                        self.add_provide(
                            lno,
                            Mechanism::JsonRpc,
                            "",
                            "",
                            "",
                            &method,
                            hint.as_deref(),
                            false,
                            "jsonrpc",
                        );
                    }
                    break;
                }
            }
            for pat in [".call(", "makeRequest(", "client.request("] {
                if let Some(idx) = line.find(pat) {
                    if let Some(method) = first_quoted(&line[idx + pat.len()..])
                        && !method.is_empty()
                    {
                        self.add_consume(
                            lno,
                            Mechanism::JsonRpc,
                            "",
                            "",
                            "",
                            &method,
                            "",
                            "jsonrpc",
                        );
                    }
                    break;
                }
            }
            if let Some(mi) = line.find("method:")
                && let Some(method) = first_quoted(&line[mi + "method:".len()..])
                && !method.is_empty()
                && !HTTP_VERBS.contains(&method.as_str())
            {
                let route = quoted_strings(line)
                    .into_iter()
                    .find(|q| q.starts_with("http"))
                    .unwrap_or_default();
                self.add_consume(
                    lno,
                    Mechanism::JsonRpc,
                    "",
                    &route,
                    "",
                    &method,
                    "",
                    "jsonrpc",
                );
            }
        }
    }
}

// ------------------------------------------------------------------ helpers

pub(crate) fn quoted_strings(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' || c == '`' {
            let mut j = i + 1;
            let mut value = String::new();
            let mut closed = false;
            while j < chars.len() {
                let cj = chars[j];
                if cj == '\\' && j + 1 < chars.len() {
                    value.push(chars[j + 1]);
                    j += 2;
                    continue;
                }
                if cj == c {
                    closed = true;
                    break;
                }
                value.push(cj);
                j += 1;
            }
            if closed {
                out.push(value);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn first_quoted(s: &str) -> Option<String> {
    quoted_strings(s).into_iter().next()
}

pub(crate) fn word_find(line: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(needle) {
        let idx = from + rel;
        let boundary_ok = idx == 0
            || !line[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if boundary_ok {
            return Some(idx);
        }
        from = idx + 1;
    }
    None
}

/// Receiver path before the `.` at `idx` (original case, may contain dots).
fn receiver_before(line: &str, idx: usize) -> Option<String> {
    let head = line.get(..idx)?;
    let bytes = head.as_bytes();
    let mut start = idx;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    if start == idx {
        return None;
    }
    Some(head[start..idx].to_string())
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['.', ':']).next().unwrap_or(path)
}

/// Handler identifier argument after the quoted route in a registration call.
fn handler_hint(line: &str, route: &str) -> Option<String> {
    let idx = line.find(route)?;
    let after = &line[idx + route.len()..];
    let rest = after.trim_start_matches(|c: char| {
        c == '"' || c == '\'' || c == '`' || c == ',' || c.is_whitespace()
    });
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let name = &rest[..end];
    if name.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(name.to_string())
}

/// Strip a `${VAR}` / `$(VAR)` / `{{VAR}}` host-prefix from a consumed URL so
/// `${ORDERS_API}/orders` still matches a literal `/orders` provider route.
fn strip_template_prefix(route: &str) -> &str {
    let (open, close) = if route.starts_with("${") {
        ("${", "}")
    } else if route.starts_with("$(") {
        ("$(", ")")
    } else if route.starts_with("{{") {
        ("{{", "}}")
    } else {
        return route;
    };
    let Some(o) = route.find(open) else {
        return route;
    };
    let rest = &route[o + open.len()..];
    let Some(c) = rest.find(close) else {
        return "";
    };
    &rest[c + close.len()..]
}

fn target_hint(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ws://")
        || url.starts_with("wss://")
    {
        if let Some(idx) = url.find("://") {
            let after = &url[idx + 3..];
            let end = after
                .find(['/', '?', '#'])
                .map(|i| idx + 3 + i)
                .unwrap_or(url.len());
            return url[..end].to_string();
        }
        return String::new();
    }
    if MARKER_TEMPLATES.iter().any(|m| url.starts_with(m)) {
        return url.split('/').next().unwrap_or(url).to_string();
    }
    String::new()
}

fn url_path(url: &str) -> String {
    let url = url.trim();
    let path = match url.find("://") {
        Some(idx) => {
            let after = &url[idx + 3..];
            match after.find('/') {
                Some(i) => &after[i..],
                None => "/",
            }
        }
        None => url,
    };
    path.split(['?', '#']).next().unwrap_or(path).to_string()
}

pub(crate) fn tail_segment(value: &str) -> String {
    let value = value.trim_end_matches('/');
    value.rsplit('/').next().unwrap_or(value).to_string()
}

/// `<recv>.<method>(` call sites: (receiver last segment, method, paren index).
fn call_sites(line: &str) -> Vec<(String, String, usize)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.' {
            let mut start = i;
            while start > 0
                && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_')
            {
                start -= 1;
            }
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if start < i && end > i + 1 {
                let mut paren = end;
                while paren < bytes.len() && (bytes[paren] == b' ' || bytes[paren] == b'\t') {
                    paren += 1;
                }
                if paren < bytes.len()
                    && bytes[paren] == b'('
                    && let (Ok(recv), Ok(method)) = (
                        std::str::from_utf8(&bytes[start..i]),
                        std::str::from_utf8(&bytes[i + 1..end]),
                    )
                {
                    out.push((recv.to_string(), method.to_string(), paren));
                }
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TreeSitterAnalyzer;

    const REPO: &str = "acme/svc";
    const SHA: &str = "abc123";

    fn contracts(language: &str, path: &str, source: &str) -> ContractBatch {
        TreeSitterAnalyzer::new()
            .analyze(REPO, SHA, path, source, language)
            .contracts
    }

    fn output(language: &str, path: &str, source: &str) -> crate::AnalyzerOutput {
        TreeSitterAnalyzer::new().analyze(REPO, SHA, path, source, language)
    }

    #[test]
    fn spring_get_mapping_extracts_provider_with_handler_id() {
        let out = output(
            "java",
            "src/OrderController.java",
            r#"
package com.acme;
public class OrderController {
    @GetMapping("/orders/{id}")
    public Order getOrder(String id) { return null; }
}
"#,
        );
        let provide = out
            .contracts
            .provides
            .iter()
            .find(|p| p.route == "/orders/{id}")
            .expect("provide");
        assert_eq!(provide.mechanism, Mechanism::Rest);
        assert_eq!(provide.http_method, "GET");
        assert_eq!(provide.framework, "spring");
        let handler = out
            .symbols
            .iter()
            .find(|s| s.name == "getOrder")
            .expect("symbol");
        assert_eq!(provide.handler_id, handler.canonical_id);
    }

    #[test]
    fn spring_request_mapping_reads_request_method() {
        let batch = contracts(
            "java",
            "src/OrderController.java",
            r#"
public class OrderController {
    @RequestMapping(value = "/orders", method = RequestMethod.POST)
    public Order create() { return null; }
}
"#,
        );
        let provide = batch
            .provides
            .iter()
            .find(|p| p.route == "/orders")
            .expect("provide");
        assert_eq!(provide.http_method, "POST");
    }

    #[test]
    fn express_route_resolves_named_handler() {
        let out = output(
            "javascript",
            "src/routes.js",
            r#"
const express = require("express");
function listUsers(req, res) { res.end(); }
const app = express();
app.get("/users", listUsers);
"#,
        );
        let provide = out
            .contracts
            .provides
            .iter()
            .find(|p| p.route == "/users")
            .expect("provide");
        assert_eq!(provide.http_method, "GET");
        assert_eq!(provide.framework, "express");
        let handler = out
            .symbols
            .iter()
            .find(|s| s.name == "listUsers")
            .expect("symbol");
        assert_eq!(provide.handler_id, handler.canonical_id);
    }

    #[test]
    fn fastapi_decorator_extracts_provider() {
        let batch = contracts(
            "python",
            "src/main.py",
            r#"
from fastapi import FastAPI
app = FastAPI()

@app.get("/items")
def list_items():
    return []
"#,
        );
        let provide = batch
            .provides
            .iter()
            .find(|p| p.route == "/items")
            .expect("provide");
        assert_eq!(provide.http_method, "GET");
        assert_eq!(provide.framework, "app");
        assert!(!provide.handler_id.as_str().is_empty());
    }

    #[test]
    fn gin_uppercase_route_extracts_provider() {
        let batch = contracts(
            "go",
            "main.go",
            r#"
package main
func ping(c *gin.Context) {}
func main() {
    r := gin.Default()
    r.GET("/ping", ping)
}
"#,
        );
        let provide = batch
            .provides
            .iter()
            .find(|p| p.route == "/ping")
            .expect("provide");
        assert_eq!(provide.http_method, "GET");
        assert_eq!(provide.framework, "gin");
    }

    #[test]
    fn axum_attribute_and_route_extract_providers() {
        let batch = contracts(
            "rust",
            "src/main.rs",
            r#"
async fn health() -> &'static str { "ok" }
#[get("/healthz")]
async fn healthz() -> &'static str { "ok" }
fn routes() {
    let app = Router::new().route("/orders/{id}", get(get_order));
}
"#,
        );
        assert!(
            batch
                .provides
                .iter()
                .any(|p| p.route == "/healthz" && p.http_method == "GET"),
            "attribute route: {:?}",
            batch.provides
        );
        assert!(
            batch
                .provides
                .iter()
                .any(|p| p.route == "/orders/{id}" && p.http_method == "GET"),
            "route() call: {:?}",
            batch.provides
        );
    }

    #[test]
    fn sinatra_verb_extracts_provider() {
        let batch = contracts(
            "ruby",
            "app.rb",
            r#"
get "/ping" do
  "pong"
end
post "/orders" do
  ""
end
"#,
        );
        assert!(
            batch
                .provides
                .iter()
                .any(|p| p.route == "/ping" && p.http_method == "GET")
        );
        assert!(
            batch
                .provides
                .iter()
                .any(|p| p.route == "/orders" && p.http_method == "POST")
        );
    }

    #[test]
    fn fetch_with_method_and_absolute_url_extracts_consume_with_hint() {
        let out = output(
            "javascript",
            "src/client.js",
            r#"
async function createOrder() {
  await fetch("https://api.orders.internal/v1/orders", { method: "POST", body: "{}" });
}
"#,
        );
        let consume = out
            .contracts
            .consumes
            .iter()
            .find(|c| c.mechanism == Mechanism::Rest)
            .expect("consume");
        assert_eq!(consume.http_method, "POST");
        assert_eq!(consume.route, "https://api.orders.internal/v1/orders");
        assert_eq!(consume.target_hint, "https://api.orders.internal");
        let caller = out
            .symbols
            .iter()
            .find(|s| s.name == "createOrder")
            .expect("symbol");
        assert_eq!(consume.caller_id, caller.canonical_id);
    }

    #[test]
    fn axios_relative_url_has_empty_hint() {
        let batch = contracts(
            "javascript",
            "src/client.js",
            r#"
const axios = require("axios");
async function run() {
  await axios.post("/v1/orders", { id: 1 });
}
"#,
        );
        let consume = batch
            .consumes
            .iter()
            .find(|c| c.route.contains("/v1/orders"))
            .expect("consume");
        assert_eq!(consume.http_method, "POST");
        assert_eq!(consume.target_hint, "");
        assert_eq!(consume.framework, "axios");
    }

    #[test]
    fn requests_get_extracts_python_consume() {
        let batch = contracts(
            "python",
            "src/client.py",
            r#"
import requests
def load():
    return requests.get("https://api.users.internal/users")
"#,
        );
        let consume = batch
            .consumes
            .iter()
            .find(|c| c.route.contains("/users"))
            .expect("consume");
        assert_eq!(consume.http_method, "GET");
        assert_eq!(consume.target_hint, "https://api.users.internal");
    }

    #[test]
    fn proto_service_rpc_extracts_grpc_provider() {
        let out = output(
            "proto",
            "api/greeter.proto",
            r#"
syntax = "proto3";
package acme.v1;
service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply);
}
"#,
        );
        let provide = out
            .contracts
            .provides
            .iter()
            .find(|p| p.mechanism == Mechanism::Grpc)
            .expect("provide");
        assert_eq!(provide.service, "acme.v1.Greeter");
        assert_eq!(provide.method, "SayHello");
        let handler = out
            .symbols
            .iter()
            .find(|s| s.name == "SayHello")
            .expect("rpc symbol");
        assert_eq!(provide.handler_id, handler.canonical_id);
    }

    #[test]
    fn java_stub_call_extracts_grpc_consume() {
        let out = output(
            "java",
            "src/GreeterClient.java",
            r#"
import io.grpc.ManagedChannel;
public class GreeterClient {
    public void hello(ManagedChannel channel) {
        GreeterBlockingStub stub = GreeterGrpc.newBlockingStub(channel);
        stub.SayHello("bob");
    }
}
"#,
        );
        let consume = out
            .contracts
            .consumes
            .iter()
            .find(|c| c.mechanism == Mechanism::Grpc)
            .expect("consume");
        assert_eq!(consume.method, "SayHello");
        assert!(!consume.caller_id.as_str().is_empty());
    }

    #[test]
    fn kafka_produce_and_listen_extract_channels() {
        let batch = contracts(
            "java",
            "src/OrderEvents.java",
            r#"
@Service
public class OrderEvents {
    @Autowired KafkaTemplate<String, String> kafkaTemplate;
    void created() { kafkaTemplate.send("orders.created", "1"); }
    @KafkaListener(topics = "orders.created")
    void onCreated(String msg) {}
}
"#,
        );
        assert!(
            batch.channels.iter().any(|c| {
                c.direction == ChannelDirection::Publish
                    && c.channel == "orders.created"
                    && c.broker == "kafka"
            }),
            "publish: {:?}",
            batch.channels
        );
        assert!(
            batch.channels.iter().any(|c| {
                c.direction == ChannelDirection::Subscribe
                    && c.channel == "orders.created"
                    && c.broker == "kafka"
            }),
            "subscribe: {:?}",
            batch.channels
        );
        assert!(
            batch
                .channels
                .iter()
                .all(|c| !c.node_id.as_str().is_empty())
        );
    }

    #[test]
    fn rabbitmq_send_to_queue_and_consume_extract_channels() {
        let batch = contracts(
            "javascript",
            "src/bus.js",
            r#"
const amqp = require("amqplib");
async function work(ch) {
  ch.sendToQueue("billing.jobs", Buffer.from("x"));
  ch.consume("billing.jobs", () => {});
}
"#,
        );
        assert!(batch.channels.iter().any(|c| {
            c.direction == ChannelDirection::Publish
                && c.channel == "billing.jobs"
                && c.broker == "rabbitmq"
        }));
        assert!(batch.channels.iter().any(|c| {
            c.direction == ChannelDirection::Subscribe && c.channel == "billing.jobs"
        }));
    }

    #[test]
    fn sqs_send_and_receive_extract_channels_from_queue_url() {
        let batch = contracts(
            "python",
            "src/queue.py",
            r#"
import boto3
def run(sqs):
    sqs.send_message(QueueUrl="https://sqs.us-east-1.amazonaws.com/123/orders-queue", MessageBody="x")
    sqs.receive_message(QueueUrl="https://sqs.us-east-1.amazonaws.com/123/orders-queue")
"#,
        );
        let publish = batch
            .channels
            .iter()
            .find(|c| c.direction == ChannelDirection::Publish)
            .expect("publish");
        assert_eq!(publish.channel, "orders-queue");
        assert_eq!(publish.broker, "sqs");
        assert!(
            batch
                .channels
                .iter()
                .any(|c| c.direction == ChannelDirection::Subscribe && c.channel == "orders-queue")
        );
    }

    #[test]
    fn websocket_and_sse_clients_extract_consumes() {
        let batch = contracts(
            "javascript",
            "src/live.js",
            r#"
function connect() {
  const ws = new WebSocket("ws://localhost:8080/stream");
  const es = new EventSource("/events");
}
"#,
        );
        let ws = batch
            .consumes
            .iter()
            .find(|c| c.mechanism == Mechanism::WebSocket)
            .expect("ws consume");
        assert_eq!(ws.route, "/stream");
        let sse = batch
            .consumes
            .iter()
            .find(|c| c.mechanism == Mechanism::Sse)
            .expect("sse consume");
        assert_eq!(sse.route, "/events");
        assert_eq!(sse.http_method, "GET");
    }

    #[test]
    fn express_ws_and_fastapi_websocket_extract_provides() {
        let js = contracts(
            "javascript",
            "src/ws.js",
            r#"
const app = require("express")();
app.ws("/chat", (ws) => {});
"#,
        );
        assert!(
            js.provides
                .iter()
                .any(|p| { p.mechanism == Mechanism::WebSocket && p.route == "/chat" })
        );

        let py = contracts(
            "python",
            "src/ws.py",
            r#"
@app.websocket("/ws")
async def ws_endpoint(websocket):
    await websocket.accept()
"#,
        );
        assert!(
            py.provides
                .iter()
                .any(|p| { p.mechanism == Mechanism::WebSocket && p.route == "/ws" })
        );
    }

    #[test]
    fn sse_producer_upgrades_rest_provider() {
        let batch = contracts(
            "java",
            "src/StreamController.java",
            r#"
public class StreamController {
    @GetMapping("/stream")
    public SseEmitter stream() {
        headers.set("Content-Type", "text/event-stream");
        return new SseEmitter();
    }
}
"#,
        );
        let provide = batch
            .provides
            .iter()
            .find(|p| p.route == "/stream")
            .expect("provide");
        assert_eq!(provide.mechanism, Mechanism::Sse);
    }

    #[test]
    fn template_channels_and_unrelated_files_are_skipped() {
        let batch = contracts(
            "python",
            "src/bus.py",
            r#"
from kafka import KafkaProducer
def pub(p):
    p.send("${TOPIC_PREFIX}.orders")
def other():
    return 1
"#,
        );
        assert!(batch.channels.is_empty(), "channels: {:?}", batch.channels);
        assert!(batch.provides.is_empty());
        assert!(batch.consumes.is_empty());

        let plain = contracts("rust", "src/lib.rs", "fn main() { println!(\"hi\"); }");
        assert!(plain.is_empty());
    }

    #[test]
    fn records_carry_file_key_and_location() {
        let batch = contracts(
            "python",
            "src/main.py",
            r#"
@app.get("/ping")
def ping():
    return "pong"
"#,
        );
        let provide = batch.provides.first().expect("provide");
        assert_eq!(provide.file, "src/main.py");
        let loc = provide.location.as_ref().expect("location");
        assert_eq!(loc.path, "src/main.py");
        assert_eq!(loc.start_line, 2);
        assert_eq!(loc.repository, REPO);
    }
}
