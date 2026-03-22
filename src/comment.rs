/// A comment in Elm source code.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Comment {
    /// A single-line comment: `-- this is a comment`
    Line(String),

    /// A multi-line block comment: `{- this is a comment -}`
    /// These can be nested in Elm.
    Block(String),

    /// A documentation comment: `{-| This is a doc comment -}`
    Doc(String),
}
