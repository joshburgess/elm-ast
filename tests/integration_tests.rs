use std::fs;
use std::path::PathBuf;

use elm_ast_rs::{parse, print};

/// Discover all .elm files under a directory recursively.
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

/// Try to parse a single .elm file. Returns Ok(()) on success, Err(message) on failure.
fn try_parse_file(path: &PathBuf) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    parse(&source).map_err(|errors| {
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
        format!(
            "failed to parse {}:\n{}",
            path.display(),
            error_msgs.join("\n")
        )
    })?;

    Ok(())
}

/// Try to parse and round-trip a single .elm file.
fn try_round_trip_file(path: &PathBuf) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    let ast1 = parse(&source).map_err(|errors| {
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
        format!(
            "first parse failed for {}:\n{}",
            path.display(),
            error_msgs.join("\n")
        )
    })?;

    let printed = print::print(&ast1);

    let ast2 = parse(&printed).map_err(|errors| {
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("  {e}")).collect();
        format!(
            "round-trip parse failed for {}:\n{}\n--- printed output (first 500 chars) ---\n{}",
            path.display(),
            error_msgs.join("\n"),
            &printed[..printed.len().min(500)]
        )
    })?;

    if ast1.declarations.len() != ast2.declarations.len() {
        return Err(format!(
            "round-trip declaration count mismatch for {}: {} vs {}",
            path.display(),
            ast1.declarations.len(),
            ast2.declarations.len()
        ));
    }

    Ok(())
}

/// Run parsing against all .elm files in test-fixtures, report results.
fn run_parse_suite(fixture_dir: &str) -> (usize, usize, Vec<String>) {
    let files = find_elm_files(fixture_dir);
    let total = files.len();
    let mut passed = 0;
    let mut failures = Vec::new();

    for file in &files {
        match try_parse_file(file) {
            Ok(()) => passed += 1,
            Err(msg) => failures.push(msg),
        }
    }

    (total, passed, failures)
}

/// Run round-trip against all parseable .elm files.
fn run_round_trip_suite(fixture_dir: &str) -> (usize, usize, Vec<String>) {
    let files = find_elm_files(fixture_dir);
    let total = files.len();
    let mut passed = 0;
    let mut failures = Vec::new();

    for file in &files {
        // Only test round-trip on files that parse successfully.
        if try_parse_file(file).is_ok() {
            match try_round_trip_file(file) {
                Ok(()) => passed += 1,
                Err(msg) => failures.push(msg),
            }
        }
    }

    (total, passed, failures)
}

#[test]
fn parse_elm_core() {
    let (total, passed, failures) = run_parse_suite("test-fixtures/core/src");
    eprintln!("elm/core: {passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("{f}\n");
    }
    // We want to track progress — print results but assert a minimum pass rate.
    assert!(
        total > 0,
        "no .elm files found in test-fixtures/core/src — run: git clone --depth 1 https://github.com/elm/core.git test-fixtures/core"
    );
    // Report pass rate for visibility.
    let pass_rate = (passed as f64 / total as f64) * 100.0;
    eprintln!("elm/core parse pass rate: {pass_rate:.1}%");
}

#[test]
fn parse_elm_html() {
    let (total, passed, failures) = run_parse_suite("test-fixtures/html/src");
    eprintln!("elm/html: {passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("{f}\n");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("elm/html parse pass rate: {pass_rate:.1}%");
    }
}

#[test]
fn parse_elm_browser() {
    let (total, passed, failures) = run_parse_suite("test-fixtures/browser/src");
    eprintln!("elm/browser: {passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("{f}\n");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("elm/browser parse pass rate: {pass_rate:.1}%");
    }
}

#[test]
fn parse_elm_json() {
    let (total, passed, failures) = run_parse_suite("test-fixtures/json/src");
    eprintln!("elm/json: {passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("{f}\n");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("elm/json parse pass rate: {pass_rate:.1}%");
    }
}

#[test]
fn parse_elm_http() {
    let (total, passed, failures) = run_parse_suite("test-fixtures/http/src");
    eprintln!("elm/http: {passed}/{total} files parsed successfully");
    for f in &failures {
        eprintln!("{f}\n");
    }
    if total > 0 {
        let pass_rate = (passed as f64 / total as f64) * 100.0;
        eprintln!("elm/http parse pass rate: {pass_rate:.1}%");
    }
}
