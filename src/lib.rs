//! A [`syn`]-quality Rust library for parsing and constructing Elm 0.19.1 ASTs.
//!
//! `elm-ast-rs` provides a complete, strongly-typed representation of Elm source
//! code as a Rust AST, along with a parser, pretty-printer, and visitor/fold
//! traits for traversal and transformation.
//!
//! # Quick start
//!
//! ```rust
//! use elm_ast::{parse, print};
//!
//! let source = r#"
//! module Main exposing (..)
//!
//! add : Int -> Int -> Int
//! add x y =
//!     x + y
//! "#;
//!
//! let module = parse(source).unwrap();
//! assert_eq!(module.declarations.len(), 1);
//!
//! let output = print(&module);
//! assert!(output.contains("add x y"));
//! ```
//!
//! # Features
//!
//! All features are enabled by default via `full`. Disable `default-features`
//! and pick what you need to reduce compile times.
//!
//! | Feature | Description |
//! |-----------|-----------------------------------------------------|
//! | `full` | Enables all features below (default) |
//! | `parsing` | [`parse()`] and [`parse_recovering()`] functions |
//! | `printing`| [`print()`], `Display` impls, `Printer` struct |
//! | `visit` | [`Visit`](visit::Visit) trait for immutable traversal |
//! | `visit-mut`| [`VisitMut`](visit_mut::VisitMut) for in-place mutation |
//! | `fold` | [`Fold`](fold::Fold) for owned transformation |
//! | `serde` | `Serialize`/`Deserialize` on all AST types |
//! | `wasm` | WASM bindings via `wasm-bindgen` |
//!
//! # AST overview
//!
//! The root type is [`ElmModule`], representing a complete `.elm` file. Every
//! AST node is wrapped in [`Spanned<T>`], which carries source location info
//! ([`Span`]) alongside the value.
//!
//! Key types: [`Expr`](expr::Expr), [`Pattern`](pattern::Pattern),
//! [`TypeAnnotation`](type_annotation::TypeAnnotation),
//! [`Declaration`](declaration::Declaration), [`Import`](import::Import),
//! [`ModuleHeader`](module_header::ModuleHeader).
//!
//! [`syn`]: https://docs.rs/syn

// ── Core types (always available) ────────────────────────────────────
pub mod builder;
pub mod comment;
pub mod declaration;
pub mod exposing;
pub mod expr;
pub mod file;
pub mod ident;
pub mod import;
pub mod lexer;
pub mod literal;
pub mod module_header;
pub mod node;
pub mod operator;
pub mod pattern;
pub mod span;
pub mod token;
pub mod type_annotation;

// ── Feature-gated modules ───────────────────────────────────────────
#[cfg(feature = "printing")]
pub mod display;
#[cfg(feature = "fold")]
pub mod fold;
#[cfg(feature = "parsing")]
pub mod parse;
#[cfg(feature = "printing")]
pub mod print;
#[cfg(feature = "visit")]
pub mod visit;
#[cfg(feature = "visit-mut")]
pub mod visit_mut;
#[cfg(feature = "wasm")]
pub mod wasm;

// ── Re-exports ──────────────────────────────────────────────────────
pub use file::ElmModule;
pub use lexer::Lexer;
pub use node::Spanned;
pub use span::{Position, Span};
pub use token::Token;

#[cfg(feature = "parsing")]
pub use parse::parse;
#[cfg(feature = "parsing")]
pub use parse::parse_recovering;
#[cfg(feature = "printing")]
pub use print::print;
#[cfg(feature = "printing")]
pub use print::pretty_print;
