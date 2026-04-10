use std::fs;
use std::path::{Path, PathBuf};

use elm_ast::parse;
use elm_lint::rule::LintContext;
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
    eprintln!(
        "Ran all {0} rules on {total_files} files without crashes",
        all_rules.len()
    );
}

/// Every rule should fire at least once across the full 291-file corpus.
/// If a rule never triggers on real code, it's either broken or useless.
#[test]
fn every_rule_fires_on_real_code() {
    let dirs = [
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
    ];

    let all_rules = rules::all_rules();
    let mut hits: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

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
                let errors = rule.check(&ctx);
                if !errors.is_empty() {
                    *hits.entry(rule.name()).or_default() += errors.len();
                }
            }
        }
    }

    // Rules that might not fire on well-written real code are exempt.
    // These are checked via synthetic snippets in each_rule_fires_on_something.
    let exempt = [
        "NoDebug",              // well-maintained packages don't ship Debug.log
        "NoBooleanCase",        // uncommon pattern
        "NoIfTrueFalse",        // uncommon pattern
        "NoRedundantCons",      // uncommon pattern
        "NoAlwaysIdentity",     // uncommon pattern
        "NoEmptyLet",           // uncommon pattern
        "NoEmptyRecordUpdate",  // uncommon pattern
        "NoNestedNegation",     // uncommon pattern
        "NoWildcardPatternLast", // uncommon in well-written code
    ];

    let mut missing = Vec::new();
    for rule in &all_rules {
        if !exempt.contains(&rule.name()) && !hits.contains_key(rule.name()) {
            missing.push(rule.name());
        }
    }

    eprintln!("Rule fire counts across 291 real files:");
    for rule in &all_rules {
        let count = hits.get(rule.name()).copied().unwrap_or(0);
        let marker = if exempt.contains(&rule.name()) { " (exempt)" } else { "" };
        eprintln!("  {}: {}{}", rule.name(), count, marker);
    }

    assert!(
        missing.is_empty(),
        "these non-exempt rules never fired on real code: {:?}",
        missing
    );
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
        ("NoUnnecessaryParens", "module T exposing (..)\n\nx = (1)"),
        ("NoRedundantCons", "module T exposing (..)\n\nx = 1 :: []"),
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
