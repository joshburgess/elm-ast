/// Parse an Elm file and print it back out (round-trip).
///
/// Usage: cargo run --example elm_roundtrip -- <file.elm>
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: elm_roundtrip <file.elm>");
        std::process::exit(1);
    });
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Error reading {path}: {e}");
        std::process::exit(1);
    });
    let module = elm_ast::parse(&source).unwrap_or_else(|errors| {
        eprintln!("Parse errors in {path}:");
        for e in &errors {
            eprintln!("  {e}");
        }
        std::process::exit(1);
    });
    print!("{}", elm_ast::print(&module));
}
