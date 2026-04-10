use std::hash::{Hash, Hasher};

use crate::comment::Comment;
use crate::span::Span;

/// A syntax node with source span information attached.
///
/// This is the universal wrapper for AST nodes, analogous to:
/// - `A.Located` in elm/compiler
/// - `Node` in stil4m/elm-syntax
/// - `Spanned` in many Rust parser crates
///
/// Each node carries optional leading comments — comments that appeared
/// immediately before this node in source. This enables round-tripping
/// of comments inside expressions (let-in, case-of, etc.).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned<T> {
    pub span: Span,
    pub value: T,
    /// Leading comments that appeared immediately before this node in source.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub comments: Vec<Spanned<Comment>>,
}

// Manual Hash: exclude comments so hashing is based on span + value only.
impl<T: Hash> Hash for Spanned<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.span.hash(state);
        self.value.hash(state);
    }
}

impl<T> Spanned<T> {
    /// Create a new spanned node with no leading comments.
    pub fn new(span: Span, value: T) -> Self {
        Self {
            span,
            value,
            comments: Vec::new(),
        }
    }

    /// Create a spanned node with a dummy span (for synthesized/constructed nodes).
    pub fn dummy(value: T) -> Self {
        Self {
            span: Span::dummy(),
            value,
            comments: Vec::new(),
        }
    }

    /// Map the inner value, preserving the span and comments.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            span: self.span,
            value: f(self.value),
            comments: self.comments,
        }
    }

    /// Attach leading comments to this node.
    pub fn with_comments(mut self, comments: Vec<Spanned<Comment>>) -> Self {
        self.comments = comments;
        self
    }

    /// Get a reference to the inner value.
    pub fn inner(&self) -> &T {
        &self.value
    }

    /// Get a mutable reference to the inner value.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Unwrap the spanned node, discarding the span.
    pub fn into_inner(self) -> T {
        self.value
    }
}
