fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: elm_pretty <file.elm>");
    let source = std::fs::read_to_string(&path).expect("failed to read file");
    let ast = elm_ast::parse(&source).expect("failed to parse");
    let printed = elm_ast::pretty_print(&ast);
    print!("{printed}");
}
