use std::io::Write;

fn main() {
    let file = std::env::args()
        .nth(1)
        .expect("Usage: diff_pretty <file.elm>");
    let elm_format = std::env::var("ELM_FORMAT").unwrap_or_else(|_| "elm-format".to_string());

    let source = std::fs::read_to_string(&file).unwrap();
    let ast = elm_ast::parse(&source).unwrap();
    let pretty = elm_ast::pretty_print(&ast);

    let mut child = std::process::Command::new(&elm_format)
        .args(["--stdin", "--elm-version=0.19"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(pretty.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let fmt = String::from_utf8(output.stdout).unwrap();

    if pretty == fmt {
        println!("MATCH!");
        return;
    }

    let p_lines: Vec<&str> = pretty.lines().collect();
    let f_lines: Vec<&str> = fmt.lines().collect();
    let max = p_lines.len().max(f_lines.len());
    let mut diff_count = 0;
    for i in 0..max {
        let p = p_lines.get(i).copied().unwrap_or("<EOF>");
        let f = f_lines.get(i).copied().unwrap_or("<EOF>");
        if p != f {
            diff_count += 1;
            println!("L{:3} PRETTY: {}", i + 1, p);
            println!("L{:3} ELMFMT: {}", i + 1, f);
            println!();
            if diff_count > 30 {
                println!("... ({} more lines differ)", max - i);
                break;
            }
        }
    }
    println!(
        "Total lines: pretty={}, elm-format={}",
        p_lines.len(),
        f_lines.len()
    );
}
