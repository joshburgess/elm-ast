fn main() {
    let file = std::env::args().nth(1).unwrap();
    let source = std::fs::read_to_string(&file).unwrap();
    let ast = elm_ast::parse(&source).unwrap();
    print!("{}", elm_ast::pretty_print(&ast));
}
