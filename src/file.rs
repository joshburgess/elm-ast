use crate::comment::Comment;
use crate::declaration::Declaration;
use crate::import::Import;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;

/// The root AST node representing a complete Elm source file.
///
/// Corresponds to:
/// - `Module` in `AST/Source.hs`
/// - `File` in `Elm.Syntax.File`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElmModule {
    /// The module header declaration.
    pub header: Spanned<ModuleHeader>,

    /// Import declarations, in source order.
    pub imports: Vec<Spanned<Import>>,

    /// Top-level declarations, in source order.
    pub declarations: Vec<Spanned<Declaration>>,

    /// Comments that are not attached to any declaration.
    ///
    /// These are collected separately to allow round-trip fidelity
    /// when printing the AST back to source.
    pub comments: Vec<Spanned<Comment>>,
}
