use crate::comment::Comment;
use crate::declaration::Declaration;
use crate::import::Import;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::token::Token;

/// The root AST node representing a complete Elm source file.
///
/// Corresponds to:
/// - `Module` in `AST/Source.hs`
/// - `File` in `Elm.Syntax.File`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElmModule {
    /// The module header declaration.
    pub header: Spanned<ModuleHeader>,

    /// Module-level documentation comment (appears after the header, before imports).
    ///
    /// In Elm, a `{-| ... -}` comment immediately after the `module ... exposing (...)`
    /// header is the module's documentation. It is distinct from comments attached
    /// to individual declarations.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub module_documentation: Option<Spanned<String>>,

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

impl ElmModule {
    /// Get comments that appear immediately before a declaration.
    ///
    /// A comment is considered "leading" if it appears between the end of
    /// the previous declaration (or the last import, or the module header)
    /// and the start of this declaration.
    ///
    /// Note: only comments that were captured during parsing are available.
    /// For complete comment extraction, use [`extract_comments`] on the
    /// original token stream.
    pub fn leading_comments(&self, decl_index: usize) -> Vec<&Spanned<Comment>> {
        if decl_index >= self.declarations.len() {
            return Vec::new();
        }

        let decl_start = self.declarations[decl_index].span.start.offset;

        // Find the end of the previous item.
        let prev_end = if decl_index > 0 {
            self.declarations[decl_index - 1].span.end.offset
        } else if let Some(last_import) = self.imports.last() {
            last_import.span.end.offset
        } else {
            self.header.span.end.offset
        };

        self.comments
            .iter()
            .filter(|c| c.span.start.offset > prev_end && c.span.end.offset <= decl_start)
            .collect()
    }

    /// Get comments that appear on the same line as the end of a declaration
    /// (trailing line comments).
    pub fn trailing_comment(&self, decl_index: usize) -> Option<&Spanned<Comment>> {
        if decl_index >= self.declarations.len() {
            return None;
        }

        let decl_end_line = self.declarations[decl_index].span.end.line;

        // Find the next item's start to bound the search.
        let next_start = if decl_index + 1 < self.declarations.len() {
            self.declarations[decl_index + 1].span.start.offset
        } else {
            usize::MAX
        };

        self.comments.iter().find(|c| {
            c.span.start.line == decl_end_line
                && c.span.start.offset < next_start
                && matches!(c.value, Comment::Line(_))
        })
    }

    /// Get all comments that appear before any declarations (module-level
    /// header comments, after imports).
    pub fn module_comments(&self) -> Vec<&Spanned<Comment>> {
        let first_decl_start = self
            .declarations
            .first()
            .map(|d| d.span.start.offset)
            .unwrap_or(usize::MAX);

        self.comments
            .iter()
            .filter(|c| c.span.end.offset <= first_decl_start)
            .collect()
    }
}

/// Extract all comments from a token stream.
///
/// This provides complete comment coverage — unlike `ElmModule.comments`,
/// which may miss comments consumed by `skip_whitespace` during parsing.
/// Use this when you need every comment in the source.
pub fn extract_comments(tokens: &[Spanned<Token>]) -> Vec<Spanned<Comment>> {
    tokens
        .iter()
        .filter_map(|tok| match &tok.value {
            Token::LineComment(text) => Some(Spanned::new(tok.span, Comment::Line(text.clone()))),
            Token::BlockComment(text) => Some(Spanned::new(tok.span, Comment::Block(text.clone()))),
            Token::DocComment(text) => Some(Spanned::new(tok.span, Comment::Doc(text.clone()))),
            _ => None,
        })
        .collect()
}

/// Associate comments with declarations by source position.
///
/// Returns a vec parallel to `module.declarations` where each entry
/// is the list of comments that appear between the previous declaration
/// (or imports/header) and this declaration.
/// Associate comments with declarations by source line position.
///
/// Returns a vec parallel to `module.declarations` where each entry
/// is the list of comments that appear between the previous declaration's
/// start line and this declaration's start line.
pub fn associate_comments(
    module: &ElmModule,
    all_comments: &[Spanned<Comment>],
) -> Vec<Vec<Spanned<Comment>>> {
    let mut result = Vec::with_capacity(module.declarations.len());

    for (i, decl) in module.declarations.iter().enumerate() {
        let decl_start_line = decl.span.start.line;

        // Previous declaration's start line (or import/header end line).
        let prev_start_line = if i > 0 {
            module.declarations[i - 1].span.start.line
        } else if let Some(last_import) = module.imports.last() {
            last_import.span.end.line
        } else {
            module.header.span.end.line
        };

        let leading: Vec<Spanned<Comment>> = all_comments
            .iter()
            .filter(|c| c.span.start.line > prev_start_line && c.span.start.line < decl_start_line)
            .cloned()
            .collect();

        result.push(leading);
    }

    result
}
