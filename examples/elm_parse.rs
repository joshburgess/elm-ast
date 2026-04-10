/// Parse an Elm file and print the AST as JSON.
///
/// Usage: cargo run --features serde --example elm_parse -- <file.elm>
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: elm_parse <file.elm>");
        std::process::exit(1);
    });
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Error reading {path}: {e}");
        std::process::exit(1);
    });
    match elm_ast_rs::parse(&source) {
        Ok(module) => {
            #[cfg(feature = "serde")]
            {
                let json = serde_json::to_string_pretty(&module).unwrap();
                println!("{json}");
            }
            #[cfg(not(feature = "serde"))]
            {
                println!(
                    "module: {} imports, {} declarations",
                    module.imports.len(),
                    module.declarations.len()
                );
                println!("(enable --features serde for JSON output)");
            }
        }
        Err(errors) => {
            eprintln!("Parse errors in {path}:");
            for e in &errors {
                eprintln!("  {e}");
            }
            std::process::exit(1);
        }
    }
}
