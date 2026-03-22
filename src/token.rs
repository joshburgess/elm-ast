use crate::literal::Literal;

/// A token produced by the Elm lexer.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    // ── Keywords ─────────────────────────────────────────────────────
    /// `module`
    Module,
    /// `where`
    Where,
    /// `import`
    Import,
    /// `as`
    As,
    /// `exposing`
    Exposing,
    /// `type`
    Type,
    /// `alias`
    Alias,
    /// `port`
    Port,
    /// `effect`
    Effect,
    /// `if`
    If,
    /// `then`
    Then,
    /// `else`
    Else,
    /// `case`
    Case,
    /// `of`
    Of,
    /// `let`
    Let,
    /// `in`
    In,
    /// `infix`
    Infix,

    // ── Delimiters ───────────────────────────────────────────────────
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `[`
    LeftBracket,
    /// `]`
    RightBracket,
    /// `{`
    LeftBrace,
    /// `}`
    RightBrace,
    /// `,`
    Comma,
    /// `|`
    Pipe,
    /// `=`
    Equals,
    /// `:`
    Colon,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `\`
    Backslash,
    /// `_`
    Underscore,
    /// `->`
    Arrow,

    // ── Operators ────────────────────────────────────────────────────
    /// Any operator: `+`, `-`, `*`, `/`, `//`, `^`, `++`, `::`, `<|`, `|>`,
    /// `>>`, `<<`, `==`, `/=`, `<`, `>`, `<=`, `>=`, `&&`, `||`, `</>`, etc.
    ///
    /// We store operators as strings rather than individual variants because
    /// Elm's operator set is extensible (via `infix` declarations in core
    /// packages), and the parser handles precedence/associativity.
    Operator(String),

    /// `-` when used as prefix negation (contextually disambiguated from
    /// the `-` operator).
    Minus,

    // ── Identifiers ──────────────────────────────────────────────────
    /// A lowercase identifier: `foo`, `myFunction`, `x1`
    LowerName(String),

    /// An uppercase identifier: `Maybe`, `Cmd`, `Html`
    UpperName(String),

    // ── Literals ─────────────────────────────────────────────────────
    /// A literal value (char, string, int, hex, float).
    Literal(Literal),

    // ── Comments ─────────────────────────────────────────────────────
    /// A single-line comment: `-- ...`
    LineComment(String),

    /// A block comment: `{- ... -}` (may be nested)
    BlockComment(String),

    /// A documentation comment: `{-| ... -}`
    DocComment(String),

    // ── Special ──────────────────────────────────────────────────────
    /// A GLSL shader block: `[glsl| ... |]`
    Glsl(String),

    /// A newline. The lexer emits these so the parser can track
    /// indentation-sensitive layout.
    Newline,

    /// End of file.
    Eof,
}

// Manual Eq because Token contains Literal which contains f64.
impl Eq for Token {}

impl Token {
    /// Look up a keyword from a lowercase identifier string.
    /// Returns `None` if the string is not a keyword.
    pub fn keyword(s: &str) -> Option<Token> {
        match s {
            "module" => Some(Token::Module),
            "where" => Some(Token::Where),
            "import" => Some(Token::Import),
            "as" => Some(Token::As),
            "exposing" => Some(Token::Exposing),
            "type" => Some(Token::Type),
            "alias" => Some(Token::Alias),
            "port" => Some(Token::Port),
            "effect" => Some(Token::Effect),
            "if" => Some(Token::If),
            "then" => Some(Token::Then),
            "else" => Some(Token::Else),
            "case" => Some(Token::Case),
            "of" => Some(Token::Of),
            "let" => Some(Token::Let),
            "in" => Some(Token::In),
            "infix" => Some(Token::Infix),
            // Note: `left`, `right`, `non` are NOT keywords — they are only
            // contextual in `infix` declarations and are valid identifiers
            // everywhere else (e.g., `String.left`, `Dict` node fields).
            _ => None,
        }
    }

    /// Returns `true` if this token is a comment.
    pub fn is_comment(&self) -> bool {
        matches!(
            self,
            Token::LineComment(_) | Token::BlockComment(_) | Token::DocComment(_)
        )
    }

    /// Returns `true` if this token is whitespace or a newline.
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Token::Newline)
    }
}
