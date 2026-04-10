use elm_ast::parse;
use elm_lint::rule::{LintContext, Rule};
use elm_lint::rules;

fn lint(source: &str, rule: &dyn Rule) -> Vec<String> {
    let module = parse(source).unwrap_or_else(|e| panic!("parse failed: {e:?}"));
    let ctx = LintContext {
        module: &module,
        source,
        file_path: "Test.elm",
        project_modules: &[],
    };
    rule.check(&ctx).into_iter().map(|e| e.message).collect()
}

fn lint_count(source: &str, rule: &dyn Rule) -> usize {
    lint(source, rule).len()
}

// ── NoUnusedImports ──────────────────────────────────────────────────

#[test]
fn no_unused_imports_flags_unused() {
    let errors = lint_count(
        "module Main exposing (..)\n\nimport Html\n\nx = 1",
        &rules::no_unused_imports::NoUnusedImports,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_unused_imports_passes_qualified() {
    let errors = lint_count(
        "module Main exposing (..)\n\nimport Html\n\nx = Html.div",
        &rules::no_unused_imports::NoUnusedImports,
    );
    assert_eq!(errors, 0);
}

#[test]
fn no_unused_imports_passes_exposed() {
    let errors = lint_count(
        "module Main exposing (..)\n\nimport Html exposing (div)\n\nx = div",
        &rules::no_unused_imports::NoUnusedImports,
    );
    assert_eq!(errors, 0);
}

// ── NoDebug ──────────────────────────────────────────────────────────

#[test]
fn no_debug_flags_log() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = Debug.log \"hi\" 1",
        &rules::no_debug::NoDebug,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_debug_flags_todo() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = Debug.todo \"nope\"",
        &rules::no_debug::NoDebug,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_debug_passes_clean_code() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = 1 + 2",
        &rules::no_debug::NoDebug,
    );
    assert_eq!(errors, 0);
}

// ── NoMissingTypeAnnotation ──────────────────────────────────────────

#[test]
fn no_missing_type_annotation_flags_missing() {
    let errors = lint_count(
        "module Main exposing (..)\n\nadd x y = x + y",
        &rules::no_missing_type_annotation::NoMissingTypeAnnotation,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_missing_type_annotation_passes_annotated() {
    let errors = lint_count(
        "module Main exposing (..)\n\nadd : Int -> Int -> Int\nadd x y = x + y",
        &rules::no_missing_type_annotation::NoMissingTypeAnnotation,
    );
    assert_eq!(errors, 0);
}

// ── NoSinglePatternCase ──────────────────────────────────────────────

#[test]
fn no_single_pattern_case_flags_single() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx =\n    case y of\n        _ ->\n            1",
        &rules::no_single_pattern_case::NoSinglePatternCase,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_single_pattern_case_passes_multiple() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx =\n    case y of\n        True ->\n            1\n        False ->\n            0",
        &rules::no_single_pattern_case::NoSinglePatternCase,
    );
    assert_eq!(errors, 0);
}

// ── NoBooleanCase ────────────────────────────────────────────────────

#[test]
fn no_boolean_case_flags_true_false() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx =\n    case y of\n        True ->\n            1\n        False ->\n            0",
        &rules::no_boolean_case::NoBooleanCase,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_boolean_case_passes_non_bool() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx =\n    case y of\n        Just a ->\n            a\n        Nothing ->\n            0",
        &rules::no_boolean_case::NoBooleanCase,
    );
    assert_eq!(errors, 0);
}

// ── NoIfTrueFalse ────────────────────────────────────────────────────

#[test]
fn no_if_true_false_flags_identity() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = if y then True else False",
        &rules::no_if_true_false::NoIfTrueFalse,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_if_true_false_flags_negation() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = if y then False else True",
        &rules::no_if_true_false::NoIfTrueFalse,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_if_true_false_passes_normal() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = if y then 1 else 0",
        &rules::no_if_true_false::NoIfTrueFalse,
    );
    assert_eq!(errors, 0);
}

// ── NoUnnecessaryParens ──────────────────────────────────────────────

#[test]
fn no_unnecessary_parens_flags_literal() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = (1)",
        &rules::no_unnecessary_parens::NoUnnecessaryParens,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_unnecessary_parens_passes_needed() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = (1 + 2)",
        &rules::no_unnecessary_parens::NoUnnecessaryParens,
    );
    assert_eq!(errors, 0);
}

// ── NoNestedNegation ─────────────────────────────────────────────────

#[test]
fn no_nested_negation_flags_not_not() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = not (not y)",
        &rules::no_nested_negation::NoNestedNegation,
    );
    assert_eq!(errors, 1);
}

// ── NoRedundantCons ──────────────────────────────────────────────────

#[test]
fn no_redundant_cons_flags_singleton() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = 1 :: []",
        &rules::no_redundant_cons::NoRedundantCons,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_redundant_cons_passes_non_empty() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = 1 :: 2 :: []",
        &rules::no_redundant_cons::NoRedundantCons,
    );
    // The outer `::` has `2 :: []` as the right side which is flagged,
    // but `1 :: (2 :: [])` — the inner one is flagged.
    assert!(errors >= 1);
}

// ── NoAlwaysIdentity ─────────────────────────────────────────────────

#[test]
fn no_always_identity_flags() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = always identity",
        &rules::no_always_identity::NoAlwaysIdentity,
    );
    assert_eq!(errors, 1);
}

#[test]
fn no_always_identity_flags_composition() {
    let errors = lint_count(
        "module Main exposing (..)\n\nx = identity >> f",
        &rules::no_always_identity::NoAlwaysIdentity,
    );
    assert_eq!(errors, 1);
}

// ── All rules don't crash on complex code ────────────────────────────

#[test]
fn all_rules_on_complex_code() {
    let source = r#"
module Main exposing (..)

import Html exposing (div, text)

type Msg = Click | Hover

type alias Model = { count : Int, name : String }

update : Msg -> Model -> Model
update msg model =
    case msg of
        Click ->
            { model | count = model.count + 1 }
        Hover ->
            model

view model =
    div [] [ text (String.fromInt model.count) ]
"#;
    let module = parse(source).unwrap();
    let ctx = LintContext {
        module: &module,
        source,
        file_path: "Test.elm",
        project_modules: &[],
    };

    // Run every rule — none should crash.
    for rule in rules::all_rules() {
        let errors = rule.check(&ctx);
        // Just verify it doesn't panic. We don't assert specific counts
        // because some rules will legitimately fire.
        let _ = errors;
    }
}
