//! WASM bindings for browser-based Elm tooling.
//!
//! Requires the `wasm` feature. Provides JavaScript-callable functions
//! for parsing and printing Elm source code.
//!
//! ## Usage from JavaScript
//!
//! ```js
//! import init, { parse_elm, print_elm, parse_elm_recovering } from './elm_ast.js';
//!
//! await init();
//!
//! // Strict parse — returns formatted Elm or throws
//! const result = parse_elm("module Main exposing (..)\n\nx = 1");
//!
//! // Recovering parse — always returns { module: string|null, errors: [...] }
//! const { module, errors } = JSON.parse(
//!     parse_elm_recovering("module Main exposing (..)\n\nx = 1\n\ny = {{{ bad")
//! );
//!
//! // Parse to JSON AST
//! const json = parse_elm_to_json("module Main exposing (..)\n\nx = 1");
//! ```

use wasm_bindgen::prelude::*;

use crate::parse::ParseError;

fn error_to_json(e: &ParseError) -> serde_json::Value {
    serde_json::json!({
        "message": e.message,
        "start": {
            "line": e.span.start.line,
            "column": e.span.start.column,
            "offset": e.span.start.offset,
        },
        "end": {
            "line": e.span.end.line,
            "column": e.span.end.column,
            "offset": e.span.end.offset,
        },
    })
}

fn errors_to_json_string(errors: &[ParseError]) -> String {
    let arr: Vec<serde_json::Value> = errors.iter().map(error_to_json).collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

/// Parse Elm source and return the pretty-printed output.
///
/// Returns the formatted Elm source, or an error string with structured
/// JSON error details.
#[wasm_bindgen]
pub fn parse_elm(source: &str) -> Result<String, String> {
    let module = crate::parse(source).map_err(|errors| errors_to_json_string(&errors))?;
    Ok(crate::print(&module))
}

/// Parse Elm source with error recovery.
///
/// Always succeeds. Returns a JSON string with:
/// - `module`: the pretty-printed partial AST (null if nothing could be parsed)
/// - `errors`: array of `{ message, start: { line, column, offset }, end: { line, column, offset } }`
#[wasm_bindgen]
pub fn parse_elm_recovering(source: &str) -> String {
    let (maybe_module, errors) = crate::parse_recovering(source);

    let module_str = maybe_module.as_ref().map(crate::print);
    let errors_json: Vec<serde_json::Value> = errors.iter().map(error_to_json).collect();

    let result = serde_json::json!({
        "module": module_str,
        "errors": errors_json,
    });

    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"module":null,"errors":[]}"#.into())
}

/// Parse Elm source and return the AST as a JSON string.
///
/// Requires both the `wasm` and `serde` features.
#[cfg(feature = "serde")]
#[wasm_bindgen]
pub fn parse_elm_to_json(source: &str) -> Result<String, String> {
    let module = crate::parse(source).map_err(|errors| errors_to_json_string(&errors))?;
    serde_json::to_string(&module).map_err(|e| format!("serialization error: {e}"))
}

/// Parse Elm source with error recovery and return the AST as JSON.
///
/// Always succeeds. Returns a JSON string with:
/// - `module`: the AST as a JSON object (null if nothing could be parsed)
/// - `errors`: array of structured error objects
///
/// Requires both the `wasm` and `serde` features.
#[cfg(feature = "serde")]
#[wasm_bindgen]
pub fn parse_elm_recovering_to_json(source: &str) -> String {
    let (maybe_module, errors) = crate::parse_recovering(source);
    let errors_json: Vec<serde_json::Value> = errors.iter().map(error_to_json).collect();

    let result = serde_json::json!({
        "module": maybe_module,
        "errors": errors_json,
    });

    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"module":null,"errors":[]}"#.into())
}

/// Print an Elm AST (given as JSON) back to Elm source.
///
/// Requires both the `wasm` and `serde` features.
#[cfg(feature = "serde")]
#[wasm_bindgen]
pub fn print_elm_from_json(json: &str) -> Result<String, String> {
    let module: crate::ElmModule =
        serde_json::from_str(json).map_err(|e| format!("deserialization error: {e}"))?;
    Ok(crate::print(&module))
}
