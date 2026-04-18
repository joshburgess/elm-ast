use elm_ast::parse::parse;
fn main() {
    let f = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&f).unwrap();
    let ast = parse(&src).unwrap();
    println!("{:#?}", ast.declarations);
}
