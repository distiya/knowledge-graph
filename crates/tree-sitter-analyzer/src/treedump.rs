#[cfg(test)]
mod dump3 {
    fn dump(lang: tree_sitter::Language, source: &str) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source, None).unwrap();
        println!("----\n{source}\n====");
        println!("{}", tree.root_node().to_sexp());
    }

    #[test]
    fn dump_all3() {
        dump(
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            r#"package com.x
@RestController
@RequestMapping("/api")
class OrderController {
    @GetMapping("/orders")
    fun list(): List = emptyList()
}
"#,
        );
        dump(
            tree_sitter_java::LANGUAGE.into(),
            r#"class E {
    void m() {
        HttpRequest req = HttpRequest.newBuilder().uri("https://x/y").GET().build();
        wc.get().uri("/a").block();
    }
}
"#,
        );
        dump(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            r#"export class X {
  @Get("a")
  m() {}
}
"#,
        );
    }
}
