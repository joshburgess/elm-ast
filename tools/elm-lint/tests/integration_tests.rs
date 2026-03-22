use std::fs;
use std::path::{Path, PathBuf};

use elm_ast_rs::parse;
use elm_lint::rule::{LintContext, Rule};
use elm_lint::rules;

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

/// All rules should run without panicking on every real-world file.
#[test]
fn all_rules_no_crash_on_real_files() {
    let dirs = [
        "../../test-fixtures/core/src",
        "../../test-fixtures/html/src",
        "../../test-fixtures/browser/src",
        "../../test-fixtures/elm-css/src",
        "../../test-fixtures/elm-ui/src",
    ];

    let all_rules = rules::all_rules();
    let mut total_files = 0;

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

            let ctx = LintContext {
                module: &module,
                source: &source,
                file_path: &file.display().to_string(),
                project_modules: &[],
            };

            for rule in &all_rules {
                // Should not panic.
                let _ = rule.check(&ctx);
            }
            total_files += 1;
        }
    }

    assert!(
        total_files > 0,
        "no fixture files found — clone test-fixtures first"
    );
    eprintln!("Ran all {0} rules on {total_files} files without crashes", all_rules.len());
}

/// Each rule should produce at least one finding on SOME file.
/// This catches rules that are accidentally no-ops.
#[test]
fn each_rule_fires_on_something() {
    let test_cases: Vec<(&str, &str)> = vec![
        (
            "NoUnusedImports",
            "module T exposing (..)\n\nimport Html\n\nx = 1",
        ),
        (
            "NoDebug",
            "module T exposing (..)\n\nx = Debug.log \"hi\" 1",
        ),
        (
            "NoMissingTypeAnnotation",
            "module T exposing (..)\n\nfoo x = x",
        ),
        (
            "NoSinglePatternCase",
            "module T exposing (..)\n\nx =\n    case y of\n        _ ->\n            1",
        ),
        (
            "NoBooleanCase",
            "module T exposing (..)\n\nx =\n    case y of\n        True ->\n            1\n        False ->\n            0",
        ),
        (
            "NoIfTrueFalse",
            "module T exposing (..)\n\nx = if y then True else False",
        ),
        (
            "NoUnnecessaryParens",
            "module T exposing (..)\n\nx = (1)",
        ),
        (
            "NoRedundantCons",
            "module T exposing (..)\n\nx = 1 :: []",
        ),
        (
            "NoAlwaysIdentity",
            "module T exposing (..)\n\nx = always identity",
        ),
    ];

    let all_rules = rules::all_rules();

    for (rule_name, source) in test_cases {
        let module = parse(source).unwrap();
        let ctx = LintContext {
            module: &module,
            source,
            file_path: "test.elm",
            project_modules: &[],
        };

        let rule = all_rules
            .iter()
            .find(|r| r.name() == rule_name)
            .unwrap_or_else(|| panic!("rule {rule_name} not found"));

        let errors = rule.check(&ctx);
        assert!(
            !errors.is_empty(),
            "rule {rule_name} should fire on test input but produced 0 errors"
        );
    }
}
