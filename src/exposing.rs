use crate::comment::Comment;
use crate::node::Spanned;
use crate::span::Span;

/// An exposing list on a module declaration or import.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposing {
    /// `exposing (..)` — expose everything.
    All(Span),

    /// `exposing (foo, Bar, Baz(..))` — expose specific items.
    ///
    /// `trailing_comments` captures any comments appearing after the last
    /// item but before the closing `)`. elm-format preserves them on their
    /// own lines just before the closing paren.
    Explicit {
        items: Vec<Spanned<ExposedItem>>,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Vec::is_empty")
        )]
        trailing_comments: Vec<Spanned<Comment>>,
    },
}

/// A single item in an explicit exposing list.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
