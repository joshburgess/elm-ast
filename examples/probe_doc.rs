use elm_ast::comment::Comment;
use elm_ast::declaration::Declaration as D;
fn main() {
    let f = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&f).unwrap();
    let ast = elm_ast::parse::parse(&src).unwrap();
    for (i, d) in ast.declarations.iter().enumerate() {
        let doc = match &d.value {
            D::FunctionDeclaration(f) => f.documentation.as_ref().map(|s| s.value.clone()),
            D::AliasDeclaration(a) => a.documentation.as_ref().map(|s| s.value.clone()),
            D::CustomTypeDeclaration(c) => c.documentation.as_ref().map(|s| s.value.clone()),
            _ => None,
        };
        if let Some(t) = doc {
            if t.contains("First, two things") {
                println!("decl {}: doc contains it", i);
                println!("--- full doc text (len={}) ---\n{}\n--- end ---", t.len(), t);
            }
        }
    }
    println!("---module-level comments: {}---", ast.comments.len());
    for c in &ast.comments {
        if let Comment::Doc(t) = &c.value {
            if t.contains("First, two things") {
                println!("Top-level doc (!) with match, len={}", t.len());
            }
        }
    }
}
