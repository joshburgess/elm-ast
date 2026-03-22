fn main() {
    let path = std::env::args().nth(1).expect("usage: print_file <file.elm>");
    let source = std::fs::read_to_string(&path).expect("failed to read file");
    let ast = elm_ast_rs::parse(&source).expect("failed to parse");
    let printed = elm_ast_rs::print::print(&ast);
    print!("{printed}");
}
