fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: roundtrip_debug <file.elm>");
    let source = std::fs::read_to_string(&path).expect("failed to read file");
    let ast = elm_ast_rs::parse(&source).expect("failed to parse");
    let printed = elm_ast_rs::print::print(&ast);

    match elm_ast_rs::parse(&printed) {
        Ok(_) => {
            println!("Round-trip OK for {path}");
        }
        Err(errors) => {
            println!("Round-trip FAILED for {path}");
            for e in &errors {
                println!("  {e}");
            }
            let lines: Vec<&str> = printed.lines().collect();
            for e in &errors {
                let line = e.span.start.line as usize;
                let start = line.saturating_sub(3);
                let end = (line + 2).min(lines.len());
                println!("\n--- context around line {line} ---");
                for (i, ln) in lines[start..end].iter().enumerate() {
                    let line_num = start + i;
                    let marker = if line_num + 1 == line { ">>>" } else { "   " };
                    println!("{marker} {:>4}: {}", line_num + 1, ln);
                }
            }
        }
    }
}
