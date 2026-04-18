use elm_ast::exposing::{ExposedItem, Exposing};
use elm_ast::module_header::ModuleHeader;
fn main() {
    let f = std::env::args().nth(1).expect("usage");
    let src = std::fs::read_to_string(&f).unwrap();
    let ast = elm_ast::parse::parse(&src).unwrap();
    let ModuleHeader::Normal { exposing, .. } = &ast.header.value else {
        return;
    };
    let Exposing::Explicit { items, .. } = &exposing.value else {
        return;
    };
    println!("{} items parsed", items.len());
    for (i, it) in items.iter().enumerate() {
        let n = match &it.value {
            ExposedItem::Function(n) => format!("Function({})", n),
            ExposedItem::TypeOrAlias(n) => format!("TypeOrAlias({})", n),
            ExposedItem::TypeExpose { name, open } => {
                format!("TypeExpose({}, open={})", name, open.is_some())
            }
            ExposedItem::Infix(n) => format!("Infix({})", n),
        };
        println!(
            "{:3}: {} @ line {}..{}",
            i, n, it.span.start.line, it.span.end.line
        );
    }
}
