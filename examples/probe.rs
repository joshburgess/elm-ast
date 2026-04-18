use elm_ast::comment::Comment;
fn main() {
    let f = std::env::args().nth(1).expect("usage: probe <file>");
    let src = std::fs::read_to_string(&f).unwrap();
    let ast = elm_ast::parse::parse(&src).unwrap();
    println!("Imports: {}", ast.imports.len());
    println!("Decls: {}", ast.declarations.len());
    for (i, imp) in ast.imports.iter().enumerate() {
        println!(
            "imp {}: {}..{}",
            i, imp.span.start.offset, imp.span.end.offset
        );
    }
    for (i, d) in ast.declarations.iter().enumerate() {
        println!(
            "decl {}: offset={}..{} line={}..{}",
            i, d.span.start.offset, d.span.end.offset, d.span.start.line, d.span.end.line
        );
    }
    println!("module_comments: {}", ast.comments.len());
    for c in &ast.comments {
        let v = &c.value;
        let s: String = match v {
            Comment::Line(s) => format!("Line: {}", s.trim()),
            Comment::Block(s) => format!("Block: {}", s.chars().take(40).collect::<String>()),
            Comment::Doc(s) => format!("Doc: {}", s.chars().take(40).collect::<String>()),
        };
        println!(
            "  offset={} line={} : {}",
            c.span.start.offset, c.span.start.line, s
        );
    }
}
