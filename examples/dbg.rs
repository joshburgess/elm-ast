use elm_ast::parse::parse;
use elm_ast::print::pretty_print;
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = &args[1];
    let start: usize = args[2].parse().unwrap();
    let count: usize = args[3].parse().unwrap();
    let src = fs::read_to_string(path).unwrap();
    let module = parse(&src).unwrap();
    let out = pretty_print(&module);
    for (i, line) in out
        .lines()
        .enumerate()
        .skip(start.saturating_sub(1))
        .take(count)
    {
        println!("{:4}: {}", i + 1, line);
    }
}
