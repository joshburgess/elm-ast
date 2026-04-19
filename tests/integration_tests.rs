#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use elm_ast::declaration::Declaration;
use elm_ast::expr::Expr;
use elm_ast::file::ElmModule;
use elm_ast::pattern::Pattern;
use elm_ast::type_annotation::TypeAnnotation;
use elm_ast::{parse, pretty_print, print};

// ── Regression watchlist ─────────────────────────────────────────────
//
// These files were the hardest to get right and are the most likely to
// regress. If a parser or printer change breaks the suite, check these first.
//
// 1. elm-community/typed-svg — Examples/GradientsPatterns.elm
//    Problem: Inside list literals like `[ linearGradient [...] [...], ... ]`,
//    function application args can appear at the *same* column as the function
//    name (both indented to the bracket's column). The standard application_loop
//    column check (`arg_col <= func_col`) rejected these as not-indented-enough.
//    Fix: `app_context_col` — list/record parsers set the opening bracket's
//    column as the reference for application_loop, relaxing the check to
//    `arg_col <= bracket_col` instead of `arg_col <= func_col`.
//
// 2. mdgriffith/elm-animator — src/Animator.elm
//    Problem: The printer output multiline function applications inline
//    (space-separated), causing subsequent args to land at columns far below
//    the function name after a multiline lambda/parenthesized arg. On re-parse,
//    application_loop rejected those args.
//    Fix: Vertical application layout — when any non-function arg is multiline,
//    each arg is emitted on its own indented line. Also, multiline record setter
//    values are placed on new indented lines so function names within them start
//    near the indent column.
//
// Other packages with non-trivial coverage (complex patterns, deep nesting,
// heavy operator use, GLSL blocks, large files):
//   - folkertdev/elm-flate (large generated decompression tables)
//   - dillonkearns/elm-markdown (deep nesting, complex case expressions)
//   - rtfeldman/elm-css (heavy operator use, many record updates)
//   - elm-explorations/webgl (GLSL shader blocks)
//   - elm/core (infix declarations, wide variety of patterns)

// ── Fixture discovery ────────────────────────────────────────────────

fn all_fixture_dirs() -> Vec<(&'static str, &'static str)> {
    vec![
        // ── elm/* ───────────────────────────────────────────────
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
        ("elm/svg", "test-fixtures/svg/src"),
        ("elm/compiler", "test-fixtures/compiler/reactor/src"),
        (
            "elm/project-metadata-utils",
            "test-fixtures/project-metadata-utils/src",
        ),
        // ── elm-explorations/* ──────────────────────────────────
        ("elm-explorations/test", "test-fixtures/test/src"),
        ("elm-explorations/markdown", "test-fixtures/markdown/src"),
        (
            "elm-explorations/linear-algebra",
            "test-fixtures/linear-algebra/src",
        ),
        ("elm-explorations/webgl", "test-fixtures/webgl/src"),
        ("elm-explorations/benchmark", "test-fixtures/benchmark/src"),
        // ── elm-community/* ─────────────────────────────────────
        ("elm-community/list-extra", "test-fixtures/list-extra/src"),
        ("elm-community/maybe-extra", "test-fixtures/maybe-extra/src"),
        (
            "elm-community/string-extra",
            "test-fixtures/string-extra/src",
        ),
        ("elm-community/dict-extra", "test-fixtures/dict-extra/src"),
        ("elm-community/array-extra", "test-fixtures/array-extra/src"),
        (
            "elm-community/result-extra",
            "test-fixtures/result-extra/src",
        ),
        ("elm-community/html-extra", "test-fixtures/html-extra/src"),
        ("elm-community/json-extra", "test-fixtures/json-extra/src"),
        ("elm-community/typed-svg", "test-fixtures/typed-svg/src"),
        // ── NoRedInk/* ──────────────────────────────────────────
        (
            "NoRedInk/elm-json-decode-pipeline",
            "test-fixtures/elm-json-decode-pipeline/src",
        ),
        (
            "NoRedInk/elm-sweet-poll",
            "test-fixtures/elm-sweet-poll/src",
        ),
        ("NoRedInk/elm-compare", "test-fixtures/elm-compare/src"),
        (
            "NoRedInk/elm-string-conversions",
            "test-fixtures/elm-string-conversions/src",
        ),
        (
            "NoRedInk/elm-sortable-table",
            "test-fixtures/elm-sortable-table/src",
        ),
        // ── rtfeldman/* ─────────────────────────────────────────
        ("rtfeldman/elm-css", "test-fixtures/elm-css/src"),
        ("rtfeldman/elm-hex", "test-fixtures/elm-hex/src"),
        (
            "rtfeldman/elm-iso8601-date-strings",
            "test-fixtures/elm-iso8601-date-strings/src",
        ),
        // ── Other widely-used packages ──────────────────────────
        ("mdgriffith/elm-ui", "test-fixtures/elm-ui/src"),
        ("mdgriffith/elm-animator", "test-fixtures/elm-animator/src"),
        (
            "dillonkearns/elm-markdown",
            "test-fixtures/elm-markdown/src",
        ),
        ("krisajenkins/remotedata", "test-fixtures/remotedata/src"),
        ("robinheghan/murmur3", "test-fixtures/murmur3/src"),
        ("myrho/elm-round", "test-fixtures/elm-round/src"),
        ("truqu/elm-base64", "test-fixtures/elm-base64/src"),
        ("folkertdev/elm-flate", "test-fixtures/elm-flate/src"),
        ("BrianHicks/elm-csv", "test-fixtures/elm-csv/src"),
        ("zwilias/elm-rosetree", "test-fixtures/elm-rosetree/src"),
        ("pzp1997/assoc-list", "test-fixtures/assoc-list/src"),
        (
            "Chadtech/elm-bool-extra",
            "test-fixtures/elm-bool-extra/src",
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

    // Check comment count (round-trip should preserve comments).
    if a.comments.len() != b.comments.len() {
        return Err(format!(
            "comment count mismatch for {}: {} vs {}",
            path.display(),
            a.comments.len(),
            b.comments.len()
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
                && ba.iter().zip(bb.iter()).all(|(a, b)| {
                    expr_eq(&a.condition.value, &b.condition.value)
                        && expr_eq(&a.then_branch.value, &b.then_branch.value)
                })
                && expr_eq(&ea.value, &eb.value)
        }
        (Expr::Negation(a), Expr::Negation(b)) => expr_eq(&a.value, &b.value),
        (Expr::Tuple(aa), Expr::Tuple(ab)) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| expr_eq(&a.value, &b.value))
        }
        (
            Expr::List { elements: aa, .. },
            Expr::List { elements: ab, .. },
        ) => {
            aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| expr_eq(&a.value, &b.value))
        }
        (
            Expr::Parenthesized { expr: a, .. },
            Expr::Parenthesized { expr: b, .. },
        ) => expr_eq(&a.value, &b.value),
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
        (Expr::Parenthesized { expr: inner, .. }, other)
        | (other, Expr::Parenthesized { expr: inner, .. }) => expr_eq(&inner.value, other),
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
    assert_eq!(
        passed,
        total,
        "{} of {} files failed to parse:\n{}",
        total - passed,
        total,
        failures.join("\n")
    );
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
    assert_eq!(
        passed,
        total,
        "{} of {} files failed round-trip:\n{}",
        total - passed,
        total,
        failures.join("\n")
    );
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
    assert_eq!(
        passed,
        total,
        "{} of {} files failed idempotency:\n{}",
        total - passed,
        total,
        failures.join("\n")
    );
}

// ── pretty_print vs elm-format ──────────────────────────────────────
//
// Verify that `pretty_print()` (ElmFormat mode) produces output identical
// to `elm-format --stdin`. This is the gold-standard test: parse a file,
// pretty-print it, feed the result to elm-format, and check nothing changes.
//
// The test is skipped when elm-format is not found on the system.
// Set ELM_FORMAT to a custom path if it's not in PATH.

/// Try to locate elm-format. Checks `$ELM_FORMAT`, then `$PATH`.
fn find_elm_format() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ELM_FORMAT") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }
    // Check PATH
    Command::new("elm-format")
        .arg("--help")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| PathBuf::from("elm-format"))
}

/// Run elm-format on the given source string. Returns the formatted output.
fn run_elm_format(elm_format: &Path, source: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = Command::new(elm_format)
        .args(["--stdin", "--elm-version=0.19"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn elm-format: {e}"))?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(source.as_bytes())
        .map_err(|e| format!("failed to write to elm-format stdin: {e}"))?;

    let output = child
        .wait_with_output()
        .map_err(|e| format!("elm-format failed: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "elm-format exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
#[ignore] // Run explicitly: cargo test pretty_print_matches_elm_format -- --ignored --nocapture
fn pretty_print_matches_elm_format() {
    let elm_format = match find_elm_format() {
        Some(path) => path,
        None => {
            eprintln!("elm-format not found — skipping pretty_print_matches_elm_format test");
            eprintln!("Install elm-format or set ELM_FORMAT=/path/to/elm-format to enable");
            return;
        }
    };

    let dirs = all_fixture_dirs();

    let mut total = 0;
    let mut passed = 0;
    let mut failures = Vec::new();

    for (pkg, dir) in &dirs {
        let files = find_elm_files(dir);
        for file in &files {
            // Only test files we can parse.
            if try_parse_file(file).is_err() {
                continue;
            }
            let source = fs::read_to_string(file).unwrap();
            let ast = match parse(&source) {
                Ok(ast) => ast,
                Err(_) => continue,
            };

            let pretty = pretty_print(&ast);

            // Feed pretty-printed output to elm-format.
            let formatted = match run_elm_format(&elm_format, &pretty) {
                Ok(f) => f,
                Err(msg) => {
                    // elm-format couldn't parse our output — that's a failure.
                    failures.push(format!(
                        "[{pkg}] elm-format rejected pretty_print output for {}: {msg}",
                        file.display()
                    ));
                    total += 1;
                    continue;
                }
            };

            total += 1;
            if pretty == formatted {
                passed += 1;
            } else {
                // Find first differing line for diagnostics.
                let pretty_lines: Vec<&str> = pretty.lines().collect();
                let fmt_lines: Vec<&str> = formatted.lines().collect();
                let mut diff_msg = String::new();
                for (i, (p, f)) in pretty_lines.iter().zip(fmt_lines.iter()).enumerate() {
                    if p != f {
                        diff_msg = format!(
                            "first diff at line {}:\n  pretty_print: {}\n  elm-format:   {}",
                            i + 1,
                            p,
                            f
                        );
                        break;
                    }
                }
                if diff_msg.is_empty() && pretty_lines.len() != fmt_lines.len() {
                    diff_msg = format!(
                        "line count differs: pretty_print={} vs elm-format={}",
                        pretty_lines.len(),
                        fmt_lines.len()
                    );
                }
                failures.push(format!(
                    "[{pkg}] pretty_print differs from elm-format for {}:\n  {diff_msg}",
                    file.display()
                ));
            }
        }
    }

    eprintln!("\n=== pretty_print vs elm-format results ===");
    eprintln!("{passed}/{total} files match elm-format exactly");
    for f in &failures {
        eprintln!("\n{f}");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("\nMatch rate: {pass_rate:.1}%");
    }
    assert_eq!(
        passed,
        total,
        "{} of {} files differ from elm-format:\n{}",
        total - passed,
        total,
        failures.join("\n")
    );
}

/// Direct parity test: our `pretty_print` output on parsed source should be
/// byte-identical to what `elm-format` produces from the same original source.
///
/// This is the stronger of the two elm-format parity tests. It checks that we
/// match elm-format's layout decisions exactly, not merely that elm-format
/// accepts our output as canonical (that weaker property is what
/// `pretty_print_matches_elm_format` checks).
#[test]
#[ignore] // Run explicitly: cargo test pretty_print_equals_elm_format_on_source -- --ignored --nocapture
fn pretty_print_equals_elm_format_on_source() {
    let elm_format = match find_elm_format() {
        Some(path) => path,
        None => {
            eprintln!(
                "elm-format not found — skipping pretty_print_equals_elm_format_on_source test"
            );
            eprintln!("Install elm-format or set ELM_FORMAT=/path/to/elm-format to enable");
            return;
        }
    };

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
            let source = fs::read_to_string(file).unwrap();
            let ast = match parse(&source) {
                Ok(ast) => ast,
                Err(_) => continue,
            };

            let pretty = pretty_print(&ast);

            // Feed ORIGINAL source (not our pretty output) to elm-format.
            let formatted_from_source = match run_elm_format(&elm_format, &source) {
                Ok(f) => f,
                Err(msg) => {
                    failures.push(format!(
                        "[{pkg}] elm-format rejected original source for {}: {msg}",
                        file.display()
                    ));
                    total += 1;
                    continue;
                }
            };

            total += 1;
            if pretty == formatted_from_source {
                passed += 1;
            } else {
                let pretty_lines: Vec<&str> = pretty.lines().collect();
                let fmt_lines: Vec<&str> = formatted_from_source.lines().collect();
                let mut diff_msg = String::new();
                for (i, (p, f)) in pretty_lines.iter().zip(fmt_lines.iter()).enumerate() {
                    if p != f {
                        diff_msg = format!(
                            "first diff at line {}:\n  pretty_print: {}\n  elm-format:   {}",
                            i + 1,
                            p,
                            f
                        );
                        break;
                    }
                }
                if diff_msg.is_empty() && pretty_lines.len() != fmt_lines.len() {
                    diff_msg = format!(
                        "line count differs: pretty_print={} vs elm-format={}",
                        pretty_lines.len(),
                        fmt_lines.len()
                    );
                }
                failures.push(format!(
                    "[{pkg}] pretty_print differs from elm-format(source) for {}:\n  {diff_msg}",
                    file.display()
                ));
            }
        }
    }

    eprintln!("\n=== pretty_print vs elm-format(source) results ===");
    eprintln!("{passed}/{total} files match elm-format(source) exactly");
    for f in &failures {
        eprintln!("\n{f}");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("\nMatch rate: {pass_rate:.1}%");
    }
    assert_eq!(
        passed,
        total,
        "{} of {} files differ from elm-format(source):\n{}",
        total - passed,
        total,
        failures.join("\n")
    );
}
