use crate::node::Spanned;
use crate::span::Span;

/// An exposing list on a module declaration or import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposing {
    /// `exposing (..)` — expose everything.
    All(Span),

    /// `exposing (foo, Bar, Baz(..))` — expose specific items.
    Explicit(Vec<Spanned<ExposedItem>>),
}

/// A single item in an explicit exposing list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExposedItem {
    /// An exposed value or function: `foo`
    Function(String),

    /// An exposed type with no constructors: `Foo`
    TypeOrAlias(String),

    /// An exposed type with specified constructor visibility:
    /// `Foo(..)` (all constructors exposed).
    TypeExpose {
        name: String,
        /// `Some(span)` if `(..)` is present; `None` if constructors are not exposed.
        open: Option<Span>,
    },

    /// An exposed infix operator: `(+)`
    Infix(String),
}
