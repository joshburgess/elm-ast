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
pub mod parse;

// Re-export core types at the crate root for convenience.
pub use span::{Position, Span};
pub use node::Spanned;
pub use file::ElmModule;
pub use token::Token;
pub use lexer::Lexer;
pub use parse::parse;
