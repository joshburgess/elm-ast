/// List all identifiers in an Elm file using the Visit trait.
///
/// Usage: cargo run --example elm_identifiers -- <file.elm>
use elm_ast_rs::node::Spanned;
use elm_ast_rs::visit::{Visit, walk_expr};

struct IdentCollector {
    idents: Vec<(String, u32, u32)>, // (name, line, column)
}

impl Visit for IdentCollector {
    fn visit_expr(&mut self, expr: &Spanned<elm_ast_rs::expr::Expr>) {
        if let elm_ast_rs::expr::Expr::FunctionOrValue { module_name, name } = &expr.value {
            let qualified = if module_name.is_empty() {
                name.clone()
            } else {
                format!("{}.{}", module_name.join("."), name)
            };
            self.idents
                .push((qualified, expr.span.start.line, expr.span.start.column));
        }
        walk_expr(self, expr);
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: elm_identifiers <file.elm>");
        std::process::exit(1);
    });
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Error reading {path}: {e}");
        std::process::exit(1);
    });
    let module = elm_ast_rs::parse(&source).unwrap_or_else(|errors| {
        eprintln!("Parse errors in {path}:");
        for e in &errors {
            eprintln!("  {e}");
        }
        std::process::exit(1);
    });

    let mut collector = IdentCollector { idents: Vec::new() };
    collector.visit_module(&module);

    // Deduplicate and sort.
    let mut unique: Vec<&str> = collector
        .idents
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect();
    unique.sort();
    unique.dedup();

    println!("{} unique identifiers in {path}:", unique.len());
    for name in &unique {
        println!("  {name}");
    }
}
