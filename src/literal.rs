/// Literal values in Elm source code.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub enum Literal {
    /// A character literal: `'a'`
    Char(char),

    /// A single-line string literal: `"hello"`
    String(String),

    /// A multi-line string literal: `"""hello"""`
    MultilineString(String),

    /// An integer literal in decimal: `42`
    Int(i64),

    /// An integer literal in hexadecimal: `0xFF`
    Hex(i64),

    /// A floating-point literal: `3.14`. The second field is the original
    /// source lexeme (when parsed from source), which lets the pretty-printer
    /// preserve scientific notation like `1.0e10` that would otherwise be
    /// lost when round-tripping through `f64`.
    Float(f64, Option<String>),
}

// Manual PartialEq: ignore the Float lexeme so that ASTs constructed from
// source and ASTs built by codegen/tests compare equal when the numeric
// value matches.
impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Literal::Char(a), Literal::Char(b)) => a == b,
            (Literal::String(a), Literal::String(b)) => a == b,
            (Literal::MultilineString(a), Literal::MultilineString(b)) => a == b,
            (Literal::Int(a), Literal::Int(b)) => a == b,
            (Literal::Hex(a), Literal::Hex(b)) => a == b,
            (Literal::Float(a, _), Literal::Float(b, _)) => a == b,
            _ => false,
        }
    }
}

// Manual Eq impl since f64 doesn't impl Eq, but we want structural equality
// for AST comparison purposes.
impl Eq for Literal {}
