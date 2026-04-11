#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

use elm_ast::wasm::*;

const VALID_SOURCE: &str = "module Main exposing (..)\n\n\nx =\n    1\n";
const INVALID_SOURCE: &str = "this is not valid elm {{{";
const PARTIAL_SOURCE: &str = "module Main exposing (..)\n\n\nx =\n    1\n\n\ny = {{{ invalid\n";

// ── parse_elm ────────────────────────────────────────────────────────

#[wasm_bindgen_test]
fn parse_elm_valid_source() {
    let result = parse_elm(VALID_SOURCE);
    assert!(result.is_ok(), "valid source should parse: {:?}", result);
    let output = result.unwrap();
    assert!(
        output.contains("module Main"),
        "output should contain module header"
    );
    assert!(output.contains("x ="), "output should contain declaration");
}

#[wasm_bindgen_test]
fn parse_elm_invalid_source_returns_structured_errors() {
    let result = parse_elm(INVALID_SOURCE);
    assert!(result.is_err(), "invalid source should fail");
    let err = result.unwrap_err();
    // Errors should be a JSON array with structured error objects.
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&err).expect("error should be valid JSON array");
    assert!(!parsed.is_empty(), "should have at least one error");
    let first = &parsed[0];
    assert!(first.get("message").is_some(), "error should have message");
    assert!(
        first.get("start").is_some(),
        "error should have start position"
    );
    assert!(first.get("end").is_some(), "error should have end position");
    assert!(
        first["start"].get("line").is_some(),
        "start should have line"
    );
    assert!(
        first["start"].get("column").is_some(),
        "start should have column"
    );
}

#[wasm_bindgen_test]
fn parse_elm_round_trips() {
    let output = parse_elm(VALID_SOURCE).unwrap();
    // Parsing the output again should also succeed (idempotency).
    let output2 = parse_elm(&output).unwrap();
    assert_eq!(output, output2, "parse_elm should be idempotent");
}

// ── parse_elm_recovering ─────────────────────────────────────────────

#[wasm_bindgen_test]
fn parse_elm_recovering_valid_source() {
    let json = parse_elm_recovering(VALID_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).expect("should return valid JSON");
    assert!(
        result["module"].is_string(),
        "valid source should produce module output"
    );
    assert!(
        result["errors"].as_array().unwrap().is_empty(),
        "valid source should have no errors"
    );
}

#[wasm_bindgen_test]
fn parse_elm_recovering_invalid_source() {
    let json = parse_elm_recovering(INVALID_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).expect("should return valid JSON");
    assert!(
        !result["errors"].as_array().unwrap().is_empty(),
        "invalid source should have errors"
    );
}

#[wasm_bindgen_test]
fn parse_elm_recovering_partial_source() {
    let json = parse_elm_recovering(PARTIAL_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).expect("should return valid JSON");
    // Should have a partial module (the valid declaration).
    assert!(
        result["module"].is_string(),
        "should produce partial module output"
    );
    let module_str = result["module"].as_str().unwrap();
    assert!(
        module_str.contains("x ="),
        "partial output should contain valid declaration"
    );
    // Should also have errors for the invalid part.
    assert!(
        !result["errors"].as_array().unwrap().is_empty(),
        "should have parse errors"
    );
}

#[wasm_bindgen_test]
fn parse_elm_recovering_errors_have_structure() {
    let json = parse_elm_recovering(INVALID_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).unwrap();
    let errors = result["errors"].as_array().unwrap();
    let first = &errors[0];
    assert!(
        first["message"].is_string(),
        "error should have string message"
    );
    assert!(
        first["start"]["line"].is_number(),
        "start.line should be a number"
    );
    assert!(
        first["start"]["column"].is_number(),
        "start.column should be a number"
    );
    assert!(
        first["start"]["offset"].is_number(),
        "start.offset should be a number"
    );
    assert!(
        first["end"]["line"].is_number(),
        "end.line should be a number"
    );
}

// ── parse_elm_to_json ────────────────────────────────────────────────

#[wasm_bindgen_test]
fn parse_elm_to_json_valid_source() {
    let result = parse_elm_to_json(VALID_SOURCE);
    assert!(result.is_ok(), "valid source should produce JSON");
    let json = result.unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("output should be valid JSON");
    // Should have top-level AST fields.
    assert!(parsed.get("header").is_some(), "AST should have header");
    assert!(
        parsed.get("declarations").is_some(),
        "AST should have declarations"
    );
    assert!(parsed.get("imports").is_some(), "AST should have imports");
}

#[wasm_bindgen_test]
fn parse_elm_to_json_invalid_returns_structured_errors() {
    let result = parse_elm_to_json(INVALID_SOURCE);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&err).expect("error should be valid JSON array");
    assert!(!parsed.is_empty());
}

// ── parse_elm_recovering_to_json ─────────────────────────────────────

#[wasm_bindgen_test]
fn parse_elm_recovering_to_json_valid_source() {
    let json = parse_elm_recovering_to_json(VALID_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).expect("should return valid JSON");
    assert!(
        result["module"].is_object(),
        "valid source should produce AST object"
    );
    assert!(
        result["module"]["header"].is_object(),
        "AST should have header"
    );
    assert!(
        result["errors"].as_array().unwrap().is_empty(),
        "valid source should have no errors"
    );
}

#[wasm_bindgen_test]
fn parse_elm_recovering_to_json_partial_source() {
    let json = parse_elm_recovering_to_json(PARTIAL_SOURCE);
    let result: serde_json::Value = serde_json::from_str(&json).expect("should return valid JSON");
    assert!(result["module"].is_object(), "should produce partial AST");
    assert!(
        !result["errors"].as_array().unwrap().is_empty(),
        "should have errors"
    );
}

// ── print_elm_from_json ──────────────────────────────────────────────

#[wasm_bindgen_test]
fn print_elm_from_json_round_trip() {
    let ast_json = parse_elm_to_json(VALID_SOURCE).unwrap();
    let printed = print_elm_from_json(&ast_json).unwrap();
    assert!(
        printed.contains("module Main"),
        "should contain module header"
    );
    assert!(printed.contains("x ="), "should contain declaration");
}

#[wasm_bindgen_test]
fn print_elm_from_json_invalid_json() {
    let result = print_elm_from_json("not valid json");
    assert!(result.is_err(), "invalid JSON should fail");
}
