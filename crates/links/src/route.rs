const GENERIC_ROUTES: &[&str] = &[
    "/health",
    "/healthz",
    "/ready",
    "/readiness",
    "/live",
    "/metrics",
    "/favicon.ico",
];

fn unify_segment(segment: &str) -> String {
    if segment == "*" {
        return "*".to_string();
    }
    if segment.starts_with('{') && segment.ends_with('}') {
        return "*".to_string();
    }
    if segment.starts_with('<') && segment.ends_with('>') && segment.len() > 2 {
        return "*".to_string();
    }
    if segment.starts_with(':') && segment.len() > 1 {
        return "*".to_string();
    }
    if segment.contains('{') && segment.contains('}') {
        return "*".to_string();
    }
    if segment.contains('*') {
        return "*".to_string();
    }
    segment.to_string()
}

/// Normalize a route/path template for cross-repository comparison.
///
/// Ensures a leading `/`, collapses duplicate separators, drops the query
/// string, strips a trailing slash (except for the root) and unifies
/// placeholder segments (`{x}`, `:x`, `<x>`, `*`) to a single `*` segment.
/// Case is preserved (REST paths are case-sensitive).
pub fn normalize_route(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let no_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let path = match no_query.find("://") {
        Some(idx) => {
            let after_scheme = &no_query[idx + 3..];
            match after_scheme.find('/') {
                Some(i) => &after_scheme[i..],
                None => "/",
            }
        }
        None => no_query,
    };
    let prefixed = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let segments: Vec<String> = prefixed
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(unify_segment)
        .collect();
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// Compare two routes after normalization; a `*` segment matches any single
/// segment on the other side.
pub fn routes_match(left: &str, right: &str) -> bool {
    let a = normalize_route(left);
    let b = normalize_route(right);
    if a == b {
        return true;
    }
    let left_segs: Vec<&str> = a.split('/').filter(|s| !s.is_empty()).collect();
    let right_segs: Vec<&str> = b.split('/').filter(|s| !s.is_empty()).collect();
    if left_segs.len() != right_segs.len() {
        return false;
    }
    left_segs
        .iter()
        .zip(right_segs.iter())
        .all(|(x, y)| x == y || *x == "*" || *y == "*")
}

/// `normalized` must come from [`normalize_route`].
pub fn is_generic_route(normalized: &str) -> bool {
    normalized == "/"
        || GENERIC_ROUTES
            .iter()
            .any(|g| normalized == *g || normalized.ends_with(*g))
}

pub fn is_generic(raw: &str) -> bool {
    is_generic_route(&normalize_route(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_route_table() {
        let cases = [
            ("/api/orders", "/api/orders"),
            ("api/orders", "/api/orders"),
            ("/api//orders/", "/api/orders"),
            ("///a///b///", "/a/b"),
            ("/api/orders?x=1", "/api/orders"),
            ("/api/orders#frag", "/api/orders"),
            ("/api/orders/{id}", "/api/orders/*"),
            ("/api/orders/:id", "/api/orders/*"),
            ("/api/orders/<id>", "/api/orders/*"),
            ("/api/orders/*", "/api/orders/*"),
            ("/api/${id}", "/api/*"),
            ("/api/{id}/details", "/api/*/details"),
            ("/", "/"),
            ("", "/"),
            ("https://host/api/x?y=1", "/api/x"),
            ("https://host", "/"),
            ("\"/quoted/path/\"", "/quoted/path"),
            ("/Api/Orders", "/Api/Orders"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_route(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn routes_match_wildcards_and_lengths() {
        assert!(routes_match("/api/orders/{id}", "/api/orders/:id"));
        assert!(routes_match("/api/orders/{id}", "/api/orders/42"));
        assert!(routes_match("/api/orders/*", "/api/orders/42"));
        assert!(routes_match("/api/orders", "/api/orders/"));
        assert!(!routes_match("/api/orders", "/api/orders/42"));
        assert!(!routes_match("/api/a", "/api/b"));
        assert!(!routes_match("/", "/api"));
        assert!(!routes_match("/api/*/*", "/api/x"));
    }

    #[test]
    fn generic_routes_excluded_by_equality_or_suffix() {
        assert!(is_generic("/health"));
        assert!(is_generic("/healthz"));
        assert!(is_generic("/ready"));
        assert!(is_generic("/readiness"));
        assert!(is_generic("/live"));
        assert!(is_generic("/metrics"));
        assert!(is_generic("/favicon.ico"));
        assert!(is_generic("/"));
        assert!(is_generic("/v1/health"));
        assert!(is_generic("/api/metrics"));
        assert!(!is_generic("/api/orders"));
        assert!(!is_generic("/healthcheck"));
        assert!(!is_generic("/alive"));
    }
}
