// ── Core types (always available) ────────────────────────────────────
pub mod span;
pub mod node;
pub mod ident;
pub mod literal;
pub mod operator;
pub mod comment;
pub mod exposing;
pub mod module_header;
pub mod import;
pub mod type_annotation;
pub mod pattern;
pub mod expr;
pub mod declaration;
pub mod file;
pub mod token;
pub mod lexer;
pub mod builder;

// ── Feature-gated modules ───────────────────────────────────────────
#[cfg(feature = "parsing")]
pub mod parse;
#[cfg(feature = "printing")]
pub mod print;
#[cfg(feature = "printing")]
pub mod display;
#[cfg(feature = "visit")]
pub mod visit;
#[cfg(feature = "visit-mut")]
pub mod visit_mut;
#[cfg(feature = "fold")]
pub mod fold;
#[cfg(feature = "wasm")]
pub mod wasm;

// ── Re-exports ──────────────────────────────────────────────────────
pub use span::{Position, Span};
pub use node::Spanned;
pub use file::ElmModule;
pub use token::Token;
pub use lexer::Lexer;

#[cfg(feature = "parsing")]
pub use parse::parse;
#[cfg(feature = "parsing")]
pub use parse::parse_recovering;
#[cfg(feature = "printing")]
pub use print::print;
