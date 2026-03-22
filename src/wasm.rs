//! WASM bindings for browser-based Elm tooling.
//!
//! Requires the `wasm` feature. Provides JavaScript-callable functions
//! for parsing and printing Elm source code.
//!
//! ## Usage from JavaScript
//!
//! ```js
//! import init, { parse_elm, print_elm, parse_elm_to_json } from './elm_ast_rs.js';
//!
//! await init();
//!
//! const result = parse_elm("module Main exposing (..)\n\nx = 1");
//! console.log(result); // printed Elm (round-trip)
//!
//! const json = parse_elm_to_json("module Main exposing (..)\n\nx = 1");
//! console.log(json); // AST as JSON (requires serde feature too)
//! ```

use wasm_bindgen::prelude::*;

/// Parse Elm source and return the pretty-printed output.
///
/// Returns the formatted Elm source, or an error message string.
#[wasm_bindgen]
pub fn parse_elm(source: &str) -> Result<String, String> {
    let module = crate::parse(source).map_err(|errors| {
        errors
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    Ok(crate::print(&module))
}

/// Parse Elm source and return the AST as a JSON string.
///
/// Requires both the `wasm` and `serde` features.
#[cfg(feature = "serde")]
#[wasm_bindgen]
pub fn parse_elm_to_json(source: &str) -> Result<String, String> {
    let module = crate::parse(source).map_err(|errors| {
        errors
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    serde_json::to_string(&module).map_err(|e| format!("serialization error: {e}"))
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
