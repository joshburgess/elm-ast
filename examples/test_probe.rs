fn main() {
    let raw = "import Time -- elm install elm/time\nimport Process\n\ntimeInOneHour : Task x Time.Posix\ntimeInOneHour =\n  Process.sleep (60 * 60 * 1000)\n    |> andThen (\\_ -> Time.now)";
    let wrapped = format!("module DocTemp__ exposing (..)\n\n\n{}", raw);
    match elm_ast::parse::parse(&wrapped) {
        Ok(m) => {
            let pp = elm_ast::print::pretty_print(&m);
            println!("{}", pp);
        }
        Err(e) => println!("Parse failed: {:?}", e),
    }
}
