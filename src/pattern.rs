use crate::ident::{Ident, ModuleName};
use crate::literal::Literal;
use crate::node::Spanned;

/// A pattern in Elm source code.
///
/// Patterns appear in function arguments, case branches, let destructuring, and
/// lambda arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Wildcard pattern: `_`
    Anything,

    /// Variable binding: `x`, `name`
    Var(Ident),

    /// Literal pattern: `42`, `"hello"`, `'c'`
    Literal(Literal),

    /// Unit pattern: `()`
    Unit,

    /// Tuple pattern: `( a, b )` or `( a, b, c )`
    Tuple(Vec<Spanned<Pattern>>),

    /// Constructor pattern, possibly qualified: `Just x`, `Maybe.Nothing`
    Constructor {
        module_name: ModuleName,
        name: Ident,
        args: Vec<Spanned<Pattern>>,
    },

    /// Record destructuring pattern: `{ name, age }`
    Record(Vec<Spanned<Ident>>),

    /// Cons pattern: `x :: xs`
    Cons {
        head: Box<Spanned<Pattern>>,
        tail: Box<Spanned<Pattern>>,
    },

    /// List pattern: `[ x, y, z ]`
    List(Vec<Spanned<Pattern>>),

    /// As pattern: `_ as name`, `(x, y) as point`
    As {
        pattern: Box<Spanned<Pattern>>,
        name: Spanned<Ident>,
    },

    /// Parenthesized pattern: `( pattern )`
    Parenthesized(Box<Spanned<Pattern>>),

    /// Hex literal pattern: `0xFF`
    Hex(i64),
}
