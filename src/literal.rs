/// Literal values in Elm source code.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
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

    /// A floating-point literal: `3.14`
    Float(f64),
}

// Manual Eq impl since f64 doesn't impl Eq, but we want structural equality
// for AST comparison purposes.
impl Eq for Literal {}
