use std::fs;
use std::path::{Path, PathBuf};

use elm_ast_rs::parse;
use elm_search::query::parse_query;
use elm_search::search::search;

fn find_elm_files(dir: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if Path::new(dir).exists() {
        collect_elm_files(&PathBuf::from(dir), &mut files);
        files.sort();
    }
    files
}

fn collect_elm_files(dir: &Path, files: &mut Vec<PathBuf>) {
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

fn search_fixtures(query_str: &str) -> usize {
    let dirs = [
        "../../test-fixtures/core/src",
        "../../test-fixtures/html/src",
    ];
    let query = parse_query(query_str).unwrap();
    let mut total = 0;

    for dir in &dirs {
        let files = find_elm_files(dir);
        for file in &files {
            let source = match fs::read_to_string(file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let module = match parse(&source) {
                Ok(m) => m,
                Err(_) => continue,
            };
            total += search(&module, &query).len();
        }
    }
    total
}

/// All query types should run without panicking on real files.
#[test]
fn all_queries_no_crash() {
    let queries = [
        "returns Maybe",
        "type Int",
        "case-on Just",
        "update .name",
        "calls Dict",
        "unused-args",
        "lambda 2",
        "uses map",
        "def get",
        "expr case",
    ];

    for q in &queries {
        let count = search_fixtures(q);
        // Just verify no panic. Some may have 0 results.
        let _ = count;
    }
}

#[test]
fn returns_maybe_finds_results_in_core() {
    let count = search_fixtures("returns Maybe");
    assert!(
        count > 0,
        "should find functions returning Maybe in elm/core"
    );
}

#[test]
fn case_on_just_finds_results_in_core() {
    let count = search_fixtures("case-on Just");
    assert!(
        count > 0,
        "should find case expressions matching Just in elm/core"
    );
}

#[test]
fn unused_args_finds_results_in_core() {
    let count = search_fixtures("unused-args");
    // elm/core has a few unused args in kernel-backed modules.
    assert!(count > 0, "should find unused args in elm/core");
}

#[test]
fn def_finds_common_names() {
    let count = search_fixtures("def map");
    assert!(count > 0, "should find definitions containing 'map'");
}

#[test]
fn expr_case_finds_case_expressions() {
    let count = search_fixtures("expr case");
    assert!(count > 0, "should find case expressions in elm/core");
}
