use std::fs;
use std::path::{Path, PathBuf};

use elm_ast::module_header::ModuleHeader;
use elm_ast::parse;
use elm_deps::graph::{build_graph, find_cycles};

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

fn all_fixture_dirs() -> Vec<&'static str> {
    vec![
        "../../test-fixtures/core/src",
        "../../test-fixtures/html/src",
        "../../test-fixtures/browser/src",
        "../../test-fixtures/json/src",
        "../../test-fixtures/http/src",
        "../../test-fixtures/url/src",
        "../../test-fixtures/parser/src",
        "../../test-fixtures/virtual-dom/src",
        "../../test-fixtures/bytes/src",
        "../../test-fixtures/file/src",
        "../../test-fixtures/time/src",
        "../../test-fixtures/regex/src",
        "../../test-fixtures/random/src",
        "../../test-fixtures/svg/src",
        "../../test-fixtures/compiler/reactor/src",
        "../../test-fixtures/project-metadata-utils/src",
        "../../test-fixtures/test/src",
        "../../test-fixtures/markdown/src",
        "../../test-fixtures/linear-algebra/src",
        "../../test-fixtures/webgl/src",
        "../../test-fixtures/benchmark/src",
        "../../test-fixtures/list-extra/src",
        "../../test-fixtures/maybe-extra/src",
        "../../test-fixtures/string-extra/src",
        "../../test-fixtures/dict-extra/src",
        "../../test-fixtures/array-extra/src",
        "../../test-fixtures/result-extra/src",
        "../../test-fixtures/html-extra/src",
        "../../test-fixtures/json-extra/src",
        "../../test-fixtures/typed-svg/src",
        "../../test-fixtures/elm-json-decode-pipeline/src",
        "../../test-fixtures/elm-sweet-poll/src",
        "../../test-fixtures/elm-compare/src",
        "../../test-fixtures/elm-string-conversions/src",
        "../../test-fixtures/elm-sortable-table/src",
        "../../test-fixtures/elm-css/src",
        "../../test-fixtures/elm-hex/src",
        "../../test-fixtures/elm-iso8601-date-strings/src",
        "../../test-fixtures/elm-ui/src",
        "../../test-fixtures/elm-animator/src",
        "../../test-fixtures/elm-markdown/src",
        "../../test-fixtures/remotedata/src",
        "../../test-fixtures/murmur3/src",
        "../../test-fixtures/elm-round/src",
        "../../test-fixtures/elm-base64/src",
        "../../test-fixtures/elm-flate/src",
        "../../test-fixtures/elm-csv/src",
        "../../test-fixtures/elm-rosetree/src",
        "../../test-fixtures/assoc-list/src",
        "../../test-fixtures/elm-bool-extra/src",
    ]
}

/// build_graph and find_cycles should not panic on real-world module graphs.
#[test]
fn graph_no_crash_on_all_fixtures() {
    let mut modules: Vec<(String, Vec<String>)> = Vec::new();

    for dir in all_fixture_dirs() {
        for file in find_elm_files(dir) {
            let source = match fs::read_to_string(&file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let module = match parse(&source) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mod_name = match &module.header.value {
                ModuleHeader::Normal { name, .. }
                | ModuleHeader::Port { name, .. }
                | ModuleHeader::Effect { name, .. } => name.value.join("."),
            };
            let imports: Vec<String> = module
                .imports
                .iter()
                .map(|imp| imp.value.module_name.value.join("."))
                .collect();
            modules.push((mod_name, imports));
        }
    }

    assert!(!modules.is_empty(), "no modules found in fixtures");

    // Should not panic.
    let (graph, _project_modules) = build_graph(&modules);
    let cycles = find_cycles(&graph);

    eprintln!(
        "Built graph with {} modules, {} edges, {} cycles",
        graph.len(),
        graph.values().map(|v| v.len()).sum::<usize>(),
        cycles.len()
    );
}
