use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangTier {
    Tier1,
    Tier2,
    Tier3,
}

#[derive(Debug, Clone, Copy)]
pub struct LangSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    pub tier: LangTier,
}

impl LangSpec {
    pub const fn new(
        id: &'static str,
        aliases: &'static [&'static str],
        extensions: &'static [&'static str],
        filenames: &'static [&'static str],
        tier: LangTier,
    ) -> Self {
        Self {
            id,
            aliases,
            extensions,
            filenames,
            tier,
        }
    }

    pub const fn ext(
        id: &'static str,
        aliases: &'static [&'static str],
        extensions: &'static [&'static str],
        tier: LangTier,
    ) -> Self {
        Self::new(id, aliases, extensions, &[], tier)
    }
}

pub const LANGUAGES: &[LangSpec] = &[
    LangSpec::ext("rust", &["rs", "rust"], &["rs"], LangTier::Tier1),
    LangSpec::ext(
        "javascript",
        &["js", "javascript", "node", "jsx", "mjs", "cjs"],
        &["js", "mjs", "cjs", "jsx"],
        LangTier::Tier1,
    ),
    LangSpec::ext(
        "typescript",
        &["ts", "tsx", "typescript"],
        &["ts", "mts", "cts", "tsx"],
        LangTier::Tier1,
    ),
    LangSpec::ext(
        "python",
        &["py", "pyi", "python", "python3"],
        &["py", "pyi", "pyw"],
        LangTier::Tier1,
    ),
    LangSpec::ext("go", &["go", "golang"], &["go"], LangTier::Tier1),
    LangSpec::ext("java", &["java"], &["java"], LangTier::Tier2),
    LangSpec::ext("c", &["c", "h"], &["c", "h"], LangTier::Tier2),
    LangSpec::ext(
        "cpp",
        &["cpp", "c++", "cxx", "cc", "hpp", "hh", "hxx"],
        &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx"],
        LangTier::Tier2,
    ),
    LangSpec::ext(
        "kotlin",
        &["kotlin", "kt", "kts"],
        &["kt", "kts"],
        LangTier::Tier2,
    ),
    LangSpec::ext("sql", &["sql", "sequel"], &["sql"], LangTier::Tier2),
    LangSpec::ext(
        "shell",
        &["sh", "shell", "bash", "zsh", "fish", "shellscript"],
        &["sh", "bash", "zsh", "fish"],
        LangTier::Tier2,
    ),
    LangSpec::ext("ruby", &["rb", "ruby"], &["rb"], LangTier::Tier2),
    LangSpec::ext(
        "php",
        &["php", "php3", "php4", "php5"],
        &["php", "phtml", "php3", "php4", "php5"],
        LangTier::Tier2,
    ),
    LangSpec::ext("csharp", &["cs", "c#", "csharp"], &["cs"], LangTier::Tier2),
    LangSpec::ext("scala", &["scala", "sc"], &["scala", "sc"], LangTier::Tier2),
    LangSpec::ext("dart", &["dart"], &["dart"], LangTier::Tier2),
    LangSpec::ext("lua", &["lua"], &["lua"], LangTier::Tier2),
    LangSpec::ext("r", &["r", "rscript"], &["r", "rmd"], LangTier::Tier2),
    LangSpec::ext("julia", &["jl", "julia"], &["jl"], LangTier::Tier2),
    LangSpec::ext(
        "perl",
        &["pl", "pm", "perl"],
        &["pl", "pm"],
        LangTier::Tier3,
    ),
    LangSpec::ext(
        "elixir",
        &["ex", "exs", "elixir"],
        &["ex", "exs"],
        LangTier::Tier2,
    ),
    LangSpec::ext("haskell", &["hs", "haskell"], &["hs"], LangTier::Tier2),
    LangSpec::ext(
        "ocaml",
        &["ml", "mli", "ocaml"],
        &["ml", "mli"],
        LangTier::Tier2,
    ),
    LangSpec::ext("zig", &["zig"], &["zig"], LangTier::Tier2),
    LangSpec::ext(
        "html",
        &["html", "htm", "xhtml"],
        &["html", "htm", "xhtml"],
        LangTier::Tier2,
    ),
    LangSpec::ext("css", &["css"], &["css"], LangTier::Tier2),
    LangSpec::ext("scss", &["scss", "sass"], &["scss"], LangTier::Tier3),
    LangSpec::ext("xml", &["xml"], &["xml"], LangTier::Tier3),
    LangSpec::ext("yaml", &["yml", "yaml"], &["yml", "yaml"], LangTier::Tier2),
    LangSpec::ext("json", &["json", "jsonc"], &["json"], LangTier::Tier2),
    LangSpec::ext(
        "markdown",
        &["md", "markdown", "mdown", "mkd"],
        &["md", "markdown", "mdown", "mkd"],
        LangTier::Tier2,
    ),
    LangSpec::ext("toml", &["toml"], &["toml"], LangTier::Tier2),
    LangSpec::ext(
        "proto",
        &["proto", "protobuf", "protocolbuffers"],
        &["proto"],
        LangTier::Tier2,
    ),
    LangSpec::ext(
        "terraform",
        &["tf", "tfvars", "terraform", "hcl"],
        &["tf", "tfvars"],
        LangTier::Tier2,
    ),
    LangSpec::ext("hcl", &["hcl"], &["hcl"], LangTier::Tier2),
    LangSpec::ext(
        "powershell",
        &["ps1", "psm1", "powershell"],
        &["ps1", "psm1"],
        LangTier::Tier2,
    ),
    LangSpec::ext(
        "batch",
        &["bat", "cmd", "batch"],
        &["bat", "cmd"],
        LangTier::Tier3,
    ),
    LangSpec::new(
        "cmake",
        &["cmake"],
        &["cmake"],
        &["CMakeLists.txt"],
        LangTier::Tier2,
    ),
    LangSpec::new(
        "make",
        &["make", "makefile", "gnumakefile"],
        &["mk", "mak", "make"],
        &["Makefile", "makefile", "GNUmakefile"],
        LangTier::Tier2,
    ),
    LangSpec::new(
        "dockerfile",
        &["docker", "dockerfile"],
        &["dockerfile"],
        &["Dockerfile", "dockerfile"],
        LangTier::Tier3,
    ),
    LangSpec::ext("swift", &["swift"], &["swift"], LangTier::Tier2),
    LangSpec::ext(
        "groovy",
        &["groovy", "gradle"],
        &["groovy", "gradle"],
        LangTier::Tier3,
    ),
    LangSpec::ext("vue", &["vue"], &["vue"], LangTier::Tier3),
    LangSpec::ext("svelte", &["svelte"], &["svelte"], LangTier::Tier3),
    LangSpec::ext("racket", &["rkt", "racket"], &["rkt"], LangTier::Tier3),
    LangSpec::ext(
        "clojure",
        &["clj", "cljs", "clojure"],
        &["clj", "cljs"],
        LangTier::Tier3,
    ),
    LangSpec::ext(
        "erlang",
        &["erl", "hrl", "erlang"],
        &["erl", "hrl"],
        LangTier::Tier3,
    ),
    LangSpec::ext(
        "common-lisp",
        &["lisp", "cl", "el"],
        &["lisp", "el"],
        LangTier::Tier3,
    ),
    LangSpec::ext(
        "fortran",
        &["f", "f90", "f77", "fortran"],
        &["f90", "f77", "f"],
        LangTier::Tier3,
    ),
    LangSpec::ext("text", &["txt", "text"], &["txt"], LangTier::Tier3),
];

pub fn detect_language_from_path(path: &str) -> Option<&'static str> {
    let p = Path::new(path);
    let filename = p.file_name()?.to_str()?;
    for spec in LANGUAGES {
        if spec.filenames.contains(&filename) {
            return Some(spec.id);
        }
    }
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    for spec in LANGUAGES {
        if spec.extensions.contains(&ext.as_str()) {
            return Some(spec.id);
        }
    }
    None
}

pub fn detect_language_from_name(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    for spec in LANGUAGES {
        if spec.id == lower || spec.aliases.contains(&lower.as_str()) {
            return Some(spec.id);
        }
    }
    None
}

pub fn spec_for_id(id: &str) -> Option<&'static LangSpec> {
    LANGUAGES.iter().find(|s| s.id == id)
}

pub fn has_grammar(lang: &str) -> bool {
    let id = detect_language_from_name(lang).unwrap_or(lang);
    spec_for_id(id).is_some_and(|s| s.tier != LangTier::Tier3)
}

pub fn languages() -> &'static [LangSpec] {
    LANGUAGES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_extensions() {
        assert_eq!(detect_language_from_path("src/main.rs"), Some("rust"));
        assert_eq!(detect_language_from_path("app.js"), Some("javascript"));
        assert_eq!(detect_language_from_path("app.jsx"), Some("javascript"));
        assert_eq!(detect_language_from_path("mod.ts"), Some("typescript"));
        assert_eq!(detect_language_from_path("view.tsx"), Some("typescript"));
        assert_eq!(detect_language_from_path("script.py"), Some("python"));
        assert_eq!(detect_language_from_path("script.pyi"), Some("python"));
        assert_eq!(detect_language_from_path("main.go"), Some("go"));
    }

    #[test]
    fn detects_must_have_extensions() {
        assert_eq!(detect_language_from_path("Main.java"), Some("java"));
        assert_eq!(detect_language_from_path("src/util.c"), Some("c"));
        assert_eq!(detect_language_from_path("src/util.h"), Some("c"));
        assert_eq!(detect_language_from_path("src/core.cpp"), Some("cpp"));
        assert_eq!(detect_language_from_path("src/core.cc"), Some("cpp"));
        assert_eq!(detect_language_from_path("App.kt"), Some("kotlin"));
        assert_eq!(detect_language_from_path("build.kts"), Some("kotlin"));
        assert_eq!(detect_language_from_path("query.sql"), Some("sql"));
    }

    #[test]
    fn detects_tier3_extensions() {
        assert_eq!(detect_language_from_path("run.sh"), Some("shell"));
        assert_eq!(detect_language_from_path("app.rb"), Some("ruby"));
        assert_eq!(detect_language_from_path("index.php"), Some("php"));
        assert_eq!(detect_language_from_path("Program.cs"), Some("csharp"));
        assert_eq!(detect_language_from_path("Main.scala"), Some("scala"));
        assert_eq!(detect_language_from_path("main.dart"), Some("dart"));
        assert_eq!(detect_language_from_path("lib.lua"), Some("lua"));
        assert_eq!(detect_language_from_path("plot.R"), Some("r"));
        assert_eq!(detect_language_from_path("main.jl"), Some("julia"));
        assert_eq!(detect_language_from_path("script.pl"), Some("perl"));
        assert_eq!(detect_language_from_path("app.ex"), Some("elixir"));
        assert_eq!(detect_language_from_path("app.exs"), Some("elixir"));
        assert_eq!(detect_language_from_path("Main.hs"), Some("haskell"));
        assert_eq!(detect_language_from_path("util.ml"), Some("ocaml"));
        assert_eq!(detect_language_from_path("src/main.zig"), Some("zig"));
        assert_eq!(detect_language_from_path("index.html"), Some("html"));
        assert_eq!(detect_language_from_path("style.css"), Some("css"));
        assert_eq!(detect_language_from_path("style.scss"), Some("scss"));
        assert_eq!(detect_language_from_path("data.xml"), Some("xml"));
        assert_eq!(detect_language_from_path("config.yml"), Some("yaml"));
        assert_eq!(detect_language_from_path("config.yaml"), Some("yaml"));
        assert_eq!(detect_language_from_path("data.json"), Some("json"));
        assert_eq!(detect_language_from_path("README.md"), Some("markdown"));
        assert_eq!(detect_language_from_path("Cargo.toml"), Some("toml"));
        assert_eq!(detect_language_from_path("api.proto"), Some("proto"));
        assert_eq!(detect_language_from_path("main.tf"), Some("terraform"));
        assert_eq!(detect_language_from_path("net.hcl"), Some("hcl"));
        assert_eq!(detect_language_from_path("script.ps1"), Some("powershell"));
        assert_eq!(detect_language_from_path("run.bat"), Some("batch"));
        assert_eq!(detect_language_from_path("CMakeLists.txt"), Some("cmake"));
        assert_eq!(detect_language_from_path("build.cmake"), Some("cmake"));
        assert_eq!(detect_language_from_path("Makefile"), Some("make"));
        assert_eq!(detect_language_from_path("makefile"), Some("make"));
        assert_eq!(detect_language_from_path("Dockerfile"), Some("dockerfile"));
    }

    #[test]
    fn detects_from_name_aliases() {
        assert_eq!(detect_language_from_name("Rust"), Some("rust"));
        assert_eq!(detect_language_from_name("rs"), Some("rust"));
        assert_eq!(detect_language_from_name("TypeScript"), Some("typescript"));
        assert_eq!(detect_language_from_name("ts"), Some("typescript"));
        assert_eq!(detect_language_from_name("tsx"), Some("typescript"));
        assert_eq!(detect_language_from_name("C++"), Some("cpp"));
        assert_eq!(detect_language_from_name("cpp"), Some("cpp"));
        assert_eq!(detect_language_from_name("Kotlin"), Some("kotlin"));
        assert_eq!(detect_language_from_name("Shell"), Some("shell"));
        assert_eq!(detect_language_from_name("C#"), Some("csharp"));
        assert_eq!(detect_language_from_name("nope"), None);
    }

    #[test]
    fn grammar_and_tiers() {
        assert!(has_grammar("rust"));
        assert!(has_grammar("java"));
        assert!(has_grammar("c"));
        assert!(has_grammar("cpp"));
        assert!(has_grammar("kotlin"));
        assert!(has_grammar("sql"));
        assert!(has_grammar("typescript"));
        assert!(has_grammar("ruby"));
        assert!(has_grammar("markdown"));
        assert!(has_grammar("shell"));
        assert!(has_grammar("yaml"));
        assert!(has_grammar("proto"));
        assert!(!has_grammar("perl"));
        assert!(!has_grammar("scss"));
        assert!(!has_grammar("xml"));
        assert!(!has_grammar("unknown-lang"));
        assert_eq!(spec_for_id("python").map(|s| s.tier), Some(LangTier::Tier1));
        assert_eq!(spec_for_id("java").map(|s| s.tier), Some(LangTier::Tier2));
        assert_eq!(spec_for_id("ruby").map(|s| s.tier), Some(LangTier::Tier2));
        assert_eq!(spec_for_id("perl").map(|s| s.tier), Some(LangTier::Tier3));
    }

    #[test]
    fn unknown_paths_return_none() {
        assert_eq!(detect_language_from_path("LICENSE"), None);
        assert_eq!(detect_language_from_path("lib"), None);
        assert_eq!(detect_language_from_path("archive.xyz"), None);
    }
}
