use crate::span::Span;

/// A syntax node with source span information attached.
///
/// This is the universal wrapper for AST nodes, analogous to:
/// - `A.Located` in elm/compiler
/// - `Node` in stil4m/elm-syntax
/// - `Spanned` in many Rust parser crates
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    pub span: Span,
    pub value: T,
}

impl<T> Spanned<T> {
    /// Create a new spanned node.
    pub fn new(span: Span, value: T) -> Self {
        Self { span, value }
    }

    /// Create a spanned node with a dummy span (for synthesized/constructed nodes).
    pub fn dummy(value: T) -> Self {
        Self {
            span: Span::dummy(),
            value,
        }
    }

    /// Map the inner value, preserving the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            span: self.span,
            value: f(self.value),
        }
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
