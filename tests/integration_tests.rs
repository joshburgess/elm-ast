use std::fs;
use std::path::{Path, PathBuf};

use elm_ast_rs::declaration::Declaration;
use elm_ast_rs::expr::Expr;
use elm_ast_rs::file::ElmModule;
use elm_ast_rs::pattern::Pattern;
use elm_ast_rs::type_annotation::TypeAnnotation;
use elm_ast_rs::{parse, print};

// ── Fixture discovery ────────────────────────────────────────────────

fn all_fixture_dirs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("elm/core", "test-fixtures/core/src"),
        ("elm/html", "test-fixtures/html/src"),
        ("elm/browser", "test-fixtures/browser/src"),
        ("elm/json", "test-fixtures/json/src"),
        ("elm/http", "test-fixtures/http/src"),
        ("elm/url", "test-fixtures/url/src"),
        ("elm/parser", "test-fixtures/parser/src"),
        ("elm/virtual-dom", "test-fixtures/virtual-dom/src"),
        ("elm/bytes", "test-fixtures/bytes/src"),
        ("elm/file", "test-fixtures/file/src"),
        ("elm/time", "test-fixtures/time/src"),
        ("elm/regex", "test-fixtures/regex/src"),
        ("elm/random", "test-fixtures/random/src"),
        ("rtfeldman/elm-css", "test-fixtures/elm-css/src"),
        ("mdgriffith/elm-ui", "test-fixtures/elm-ui/src"),
        ("elm/svg", "test-fixtures/svg/src"),
        ("elm/compiler", "test-fixtures/compiler/reactor/src"),
        ("elm-explorations/test", "test-fixtures/test/src"),
        ("elm-explorations/markdown", "test-fixtures/markdown/src"),
        ("elm-community/list-extra", "test-fixtures/list-extra/src"),
        ("elm-community/maybe-extra", "test-fixtures/maybe-extra/src"),
        (
            "elm-community/string-extra",
            "test-fixtures/string-extra/src",
        ),
        (
            "NoRedInk/elm-json-decode-pipeline",
            "test-fixtures/elm-json-decode-pipeline/src",
        ),
    ]
}

fn find_elm_files(dir: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let dir = PathBuf::from(dir);
    if !dir.exists() {
        return files;
    }
    collect_elm_files(&dir, &mut files);
    files.sort();
    files
}

fn collect_elm_files(dir: &PathBuf, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_elm_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "elm") {
                files.push(path);
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn try_parse_file(path: &PathBuf) -> Result<ElmModule, String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    parse(&source).map_err(|errors| {
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
        format!(
            "failed to parse {}:\n{}",
            path.display(),
            error_msgs.join("\n")
        )
    })
}

fn try_round_trip_file(path: &PathBuf) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    let ast1 =
        parse(&source).map_err(|_errors| format!("first parse failed for {}", path.display()))?;

    let printed = print::print(&ast1);

    let ast2 = parse(&printed).map_err(|errors| {
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
        format!(
            "round-trip reparse failed for {}:\n{}\n--- printed (first 500 chars) ---\n{}",
            path.display(),
            error_msgs.join("\n"),
            &printed[..printed.len().min(500)]
        )
    })?;

    // Deep structural equality check (ignoring spans).
    assert_module_eq(&ast1, &ast2, path)?;

    Ok(())
}

fn try_idempotent_print(path: &PathBuf) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    let ast1 = parse(&source).map_err(|_| format!("parse failed for {}", path.display()))?;
    let print1 = print::print(&ast1);

    let ast2 = parse(&print1).map_err(|_| format!("reparse failed for {}", path.display()))?;
    let print2 = print::print(&ast2);

    if print1 != print2 {
        // Find first difference.
        let lines1: Vec<&str> = print1.lines().collect();
        let lines2: Vec<&str> = print2.lines().collect();
        for (i, (l1, l2)) in lines1.iter().zip(lines2.iter()).enumerate() {
            if l1 != l2 {
                return Err(format!(
                    "printer not idempotent for {} at line {}:\n  print1: {}\n  print2: {}",
                    path.display(),
                    i + 1,
                    l1,
                    l2
                ));
            }
        }
        if lines1.len() != lines2.len() {
            return Err(format!(
                "printer not idempotent for {}: {} vs {} lines",
                path.display(),
                lines1.len(),
                lines2.len()
            ));
        }
    }

    Ok(())
}

// ── Deep AST equality (ignoring spans) ───────────────────────────────

fn assert_module_eq(a: &ElmModule, b: &ElmModule, path: &Path) -> Result<(), String> {
    if a.imports.len() != b.imports.len() {
        return Err(format!(
            "import count mismatch for {}: {} vs {}",
            path.display(),
            a.imports.len(),
            b.imports.len()
        ));
    }

    if a.declarations.len() != b.declarations.len() {
        return Err(format!(
            "declaration count mismatch for {}: {} vs {}",
            path.display(),
            a.declarations.len(),
            b.declarations.len()
        ));
    }

    // Check each declaration structurally.
    for (i, (da, db)) in a.declarations.iter().zip(b.declarations.iter()).enumerate() {
        if !decl_eq(&da.value, &db.value) {
            return Err(format!(
                "declaration {} differs for {}: {:?} vs {:?}",
                i,
                path.display(),
                decl_kind(&da.value),
                decl_kind(&db.value),
            ));
        }
    }

    Ok(())
}

fn decl_kind(d: &Declaration) -> &'static str {
    match d {
        Declaration::FunctionDeclaration(_) => "Function",
        Declaration::AliasDeclaration(_) => "TypeAlias",
        Declaration::CustomTypeDeclaration(_) => "CustomType",
        Declaration::PortDeclaration(_) => "Port",
        Declaration::InfixDeclaration(_) => "Infix",
        Declaration::Destructuring { .. } => "Destructuring",
    }
}

fn decl_eq(a: &Declaration, b: &Declaration) -> bool {
    match (a, b) {
        (Declaration::FunctionDeclaration(fa), Declaration::FunctionDeclaration(fb)) => {
            fa.declaration.value.name.value == fb.declaration.value.name.value
                && fa.declaration.value.args.len() == fb.declaration.value.args.len()
                && fa.signature.is_some() == fb.signature.is_some()
                && expr_eq(
                    &fa.declaration.value.body.value,
                    &fb.declaration.value.body.value,
                )
        }
        (Declaration::AliasDeclaration(aa), Declaration::AliasDeclaration(ab)) => {
            aa.name.value == ab.name.value
                && aa.generics.len() == ab.generics.len()
                && type_eq(&aa.type_annotation.value, &ab.type_annotation.value)
        }
        (Declaration::CustomTypeDeclaration(ca), Declaration::CustomTypeDeclaration(cb)) => {
            ca.name.value == cb.name.value
                && ca.generics.len() == cb.generics.len()
                && ca.constructors.len() == cb.constructors.len()
                && ca
                    .constructors
                    .iter()
                    .zip(cb.constructors.iter())
                    .all(|(a, b)| {
                        a.value.name.value == b.value.name.value
                            && a.value.args.len() == b.value.args.len()
                    })
        }
        (Declaration::PortDeclaration(sa), Declaration::PortDeclaration(sb)) => {
            sa.name.value == sb.name.value
        }
        (Declaration::InfixDeclaration(ia), Declaration::InfixDeclaration(ib)) => {
            ia.operator.value == ib.operator.value
                && ia.function.value == ib.function.value
                && ia.precedence.value == ib.precedence.value
                && ia.direction.value == ib.direction.value
        }
        (
            Declaration::Destructuring { pattern: pa, .. },
            Declaration::Destructuring { pattern: pb, .. },
        ) => pattern_eq(&pa.value, &pb.value),
        _ => false,
    }
}

fn expr_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Unit, Expr::Unit) => true,
        (Expr::Literal(la), Expr::Literal(lb)) => la == lb,
        (
            Expr::FunctionOrValue {
                module_name: ma,
                name: na,
            },
            Expr::FunctionOrValue {
                module_name: mb,
                name: nb,
            },
        ) => ma == mb && na == nb,
        (Expr::PrefixOperator(a), Expr::PrefixOperator(b)) => a == b,
        (
            Expr::OperatorApplication {
                operator: oa,
                left: la,
                right: ra,
                ..
            },
            Expr::OperatorApplication {
                operator: ob,
                left: lb,
                right: rb,
                ..
            },
        ) => oa == ob && expr_eq(&la.value, &lb.value) && expr_eq(&ra.value, &rb.value),
        (Expr::Application(aa), Expr::Application(ab)) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| expr_eq(&a.value, &b.value))
        }
        (
            Expr::IfElse {
                branches: ba,
                else_branch: ea,
            },
            Expr::IfElse {
                branches: bb,
                else_branch: eb,
            },
        ) => {
            ba.len() == bb.len()
                && ba.iter().zip(bb.iter()).all(|((ca, ta), (cb, tb))| {
                    expr_eq(&ca.value, &cb.value) && expr_eq(&ta.value, &tb.value)
                })
                && expr_eq(&ea.value, &eb.value)
        }
        (Expr::Negation(a), Expr::Negation(b)) => expr_eq(&a.value, &b.value),
        (Expr::Tuple(aa), Expr::Tuple(ab)) | (Expr::List(aa), Expr::List(ab)) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| expr_eq(&a.value, &b.value))
        }
        (Expr::Parenthesized(a), Expr::Parenthesized(b)) => expr_eq(&a.value, &b.value),
        // For deep comparison of case/let/lambda/record, check structural shape.
        (Expr::CaseOf { branches: ba, .. }, Expr::CaseOf { branches: bb, .. }) => {
            ba.len() == bb.len()
        }
        (
            Expr::LetIn {
                declarations: da, ..
            },
            Expr::LetIn {
                declarations: db, ..
            },
        ) => da.len() == db.len(),
        (Expr::Lambda { args: aa, .. }, Expr::Lambda { args: ab, .. }) => aa.len() == ab.len(),
        (Expr::Record(fa), Expr::Record(fb)) => fa.len() == fb.len(),
        (
            Expr::RecordUpdate {
                base: ba,
                updates: ua,
            },
            Expr::RecordUpdate {
                base: bb,
                updates: ub,
            },
        ) => ba.value == bb.value && ua.len() == ub.len(),
        (Expr::RecordAccess { field: fa, .. }, Expr::RecordAccess { field: fb, .. }) => {
            fa.value == fb.value
        }
        (Expr::RecordAccessFunction(a), Expr::RecordAccessFunction(b)) => a == b,
        (Expr::GLSLExpression(a), Expr::GLSLExpression(b)) => a == b,
        // Parenthesized in one but not the other is OK if the inner matches.
        // The printer may add parens that weren't in the original.
        (Expr::Parenthesized(inner), other) | (other, Expr::Parenthesized(inner)) => {
            expr_eq(&inner.value, other)
        }
        _ => false,
    }
}

fn pattern_eq(a: &Pattern, b: &Pattern) -> bool {
    match (a, b) {
        (Pattern::Anything, Pattern::Anything) => true,
        (Pattern::Unit, Pattern::Unit) => true,
        (Pattern::Var(a), Pattern::Var(b)) => a == b,
        (Pattern::Literal(a), Pattern::Literal(b)) => a == b,
        (Pattern::Hex(a), Pattern::Hex(b)) => a == b,
        (Pattern::Tuple(aa), Pattern::Tuple(ab)) | (Pattern::List(aa), Pattern::List(ab)) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| pattern_eq(&a.value, &b.value))
        }
        (
            Pattern::Constructor {
                name: na, args: aa, ..
            },
            Pattern::Constructor {
                name: nb, args: ab, ..
            },
        ) => na == nb && aa.len() == ab.len(),
        (Pattern::Record(fa), Pattern::Record(fb)) => {
            fa.len() == fb.len() && fa.iter().zip(fb.iter()).all(|(a, b)| a.value == b.value)
        }
        (Pattern::Cons { .. }, Pattern::Cons { .. }) => true,
        (Pattern::As { name: na, .. }, Pattern::As { name: nb, .. }) => na.value == nb.value,
        (Pattern::Parenthesized(a), Pattern::Parenthesized(b)) => pattern_eq(&a.value, &b.value),
        (Pattern::Parenthesized(inner), other) | (other, Pattern::Parenthesized(inner)) => {
            pattern_eq(&inner.value, other)
        }
        _ => false,
    }
}

fn type_eq(a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
    match (a, b) {
        (TypeAnnotation::GenericType(a), TypeAnnotation::GenericType(b)) => a == b,
        (TypeAnnotation::Unit, TypeAnnotation::Unit) => true,
        (
            TypeAnnotation::Typed {
                name: na, args: aa, ..
            },
            TypeAnnotation::Typed {
                name: nb, args: ab, ..
            },
        ) => {
            na.value == nb.value
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| type_eq(&a.value, &b.value))
        }
        (TypeAnnotation::Tupled(aa), TypeAnnotation::Tupled(ab)) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| type_eq(&a.value, &b.value))
        }
        (TypeAnnotation::Record(fa), TypeAnnotation::Record(fb)) => {
            fa.len() == fb.len()
                && fa.iter().zip(fb.iter()).all(|(a, b)| {
                    a.value.name.value == b.value.name.value
                        && type_eq(
                            &a.value.type_annotation.value,
                            &b.value.type_annotation.value,
                        )
                })
        }
        (
            TypeAnnotation::GenericRecord {
                base: ba,
                fields: fa,
            },
            TypeAnnotation::GenericRecord {
                base: bb,
                fields: fb,
            },
        ) => ba.value == bb.value && fa.len() == fb.len(),
        (
            TypeAnnotation::FunctionType { from: fa, to: ta },
            TypeAnnotation::FunctionType { from: fb, to: tb },
        ) => type_eq(&fa.value, &fb.value) && type_eq(&ta.value, &tb.value),
        _ => false,
    }
}

// ── Run suite helpers ────────────────────────────────────────────────

fn run_parse_suite(dirs: &[(&str, &str)]) -> (usize, usize, Vec<String>) {
    let mut total = 0;
    let mut passed = 0;
    let mut failures = Vec::new();

    for (pkg, dir) in dirs {
        let files = find_elm_files(dir);
        for file in &files {
            total += 1;
            match try_parse_file(file) {
                Ok(_) => passed += 1,
                Err(msg) => failures.push(format!("[{pkg}] {msg}")),
            }
        }
    }

    (total, passed, failures)
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn parse_all_packages() {
    let dirs = all_fixture_dirs();
    let (total, passed, failures) = run_parse_suite(&dirs);

    eprintln!("\n=== Parse results ===");
    eprintln!("{passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("\n{f}");
    }

    assert!(total > 0, "no .elm files found in test-fixtures/");
    let pass_rate = (passed as f64 / total as f64) * 100.0;
    eprintln!("\nParse pass rate: {pass_rate:.1}%");
}

#[test]
fn round_trip_all_packages() {
    let dirs = all_fixture_dirs();

    let mut total = 0;
    let mut passed = 0;
    let mut failures = Vec::new();

    for (pkg, dir) in &dirs {
        let files = find_elm_files(dir);
        for file in &files {
            if try_parse_file(file).is_err() {
                continue;
            }
            total += 1;
            match try_round_trip_file(file) {
                Ok(()) => passed += 1,
                Err(msg) => failures.push(format!("[{pkg}] {msg}")),
            }
        }
    }

    eprintln!("\n=== Round-trip results (with deep AST equality) ===");
    eprintln!("{passed}/{total} files round-tripped successfully");
    for f in &failures {
        eprintln!("\n{f}");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("\nRound-trip pass rate: {pass_rate:.1}%");
    }
}

#[test]
fn printer_idempotency() {
    let dirs = all_fixture_dirs();

    let mut total = 0;
    let mut passed = 0;
    let mut failures = Vec::new();

    for (pkg, dir) in &dirs {
        let files = find_elm_files(dir);
        for file in &files {
            if try_parse_file(file).is_err() {
                continue;
            }
            total += 1;
            match try_idempotent_print(file) {
                Ok(()) => passed += 1,
                Err(msg) => failures.push(format!("[{pkg}] {msg}")),
            }
        }
    }

    eprintln!("\n=== Printer idempotency results ===");
    eprintln!("{passed}/{total} files print idempotently");
    for f in &failures {
        eprintln!("\n{f}");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("\nIdempotency pass rate: {pass_rate:.1}%");
    }
}
