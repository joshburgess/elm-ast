pub mod type_annotation;
pub mod pattern;
pub mod expr;
pub mod declaration;
pub mod module;

use crate::node::Spanned;
use crate::span::{Position, Span};
use crate::token::Token;

/// A parse error with source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.span.start.line, self.span.start.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

pub type ParseResult<T> = Result<T, ParseError>;

/// The parser. A cursor over a stream of spanned tokens.
///
/// The parser follows elm/compiler's approach to indentation: it tracks
/// indentation context using token column positions rather than virtual
/// INDENT/DEDENT tokens.
pub struct Parser {
    tokens: Vec<Spanned<Token>>,
    pos: usize,
    /// Nesting depth of parentheses/brackets/braces. When > 0,
    /// indentation-sensitive layout rules are suspended (any column is valid).
    /// This matches the elm/compiler behavior.
    paren_depth: u32,
}

impl Parser {
    /// Create a parser from a token stream (as produced by the lexer).
    pub fn new(tokens: Vec<Spanned<Token>>) -> Self {
        Self {
            tokens,
            pos: 0,
            paren_depth: 0,
        }
    }

    /// Returns true if currently inside parens/brackets/braces.
    /// When true, indentation-sensitive layout rules are suspended.
    pub fn in_paren_context(&self) -> bool {
        self.paren_depth > 0
    }

    // ── Position & peeking ───────────────────────────────────────────

    /// The current token (without advancing).
    pub fn current(&self) -> &Spanned<Token> {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    /// Peek at the current token value.
    pub fn peek(&self) -> &Token {
        &self.current().value
    }

    /// Peek at the current token's span.
    pub fn peek_span(&self) -> Span {
        self.current().span
    }

    /// The current position in source.
    pub fn current_pos(&self) -> Position {
        self.current().span.start
    }

    /// The column of the current token (1-based).
    pub fn current_column(&self) -> u32 {
        self.current().span.start.column
    }

    /// Check if we've reached Eof.
    pub fn is_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    // ── Advancing ────────────────────────────────────────────────────

    /// Advance past the current token and return it.
    /// Automatically tracks paren/bracket/brace nesting depth.
    pub fn advance(&mut self) -> Spanned<Token> {
        let tok = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        // Track paren depth for indentation-context suspension.
        match &tok.value {
            Token::LeftParen | Token::LeftBracket | Token::LeftBrace => {
                self.paren_depth += 1;
            }
            Token::RightParen | Token::RightBracket | Token::RightBrace => {
                self.paren_depth = self.paren_depth.saturating_sub(1);
            }
            _ => {}
        }
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    /// Skip over any newline and comment tokens.
    pub fn skip_whitespace(&mut self) {
        while matches!(
            self.peek(),
            Token::Newline
                | Token::LineComment(_)
                | Token::BlockComment(_)
                | Token::DocComment(_)
        ) {
            self.advance();
        }
    }

    /// Skip newlines only (preserve comments for doc comment attachment).
    pub fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }

    // ── Expecting specific tokens ────────────────────────────────────

    /// Consume the current token if it matches, otherwise return an error.
    pub fn expect(&mut self, expected: &Token) -> ParseResult<Spanned<Token>> {
        self.skip_whitespace();
        if self.peek() == expected {
            Ok(self.advance())
        } else {
            Err(self.error(format!("expected {}, found {}", describe(expected), describe(self.peek()))))
        }
    }

    /// Consume a `LowerName` and return the string.
    pub fn expect_lower_name(&mut self) -> ParseResult<Spanned<String>> {
        self.skip_whitespace();
        match self.peek().clone() {
            Token::LowerName(name) => {
                let tok = self.advance();
                Ok(Spanned::new(tok.span, name))
            }
            _ => Err(self.error(format!(
                "expected lowercase name, found {}",
                describe(self.peek())
            ))),
        }
    }

    /// Consume an `UpperName` and return the string.
    pub fn expect_upper_name(&mut self) -> ParseResult<Spanned<String>> {
        self.skip_whitespace();
        match self.peek().clone() {
            Token::UpperName(name) => {
                let tok = self.advance();
                Ok(Spanned::new(tok.span, name))
            }
            _ => Err(self.error(format!(
                "expected uppercase name, found {}",
                describe(self.peek())
            ))),
        }
    }

    // ── Lookahead helpers ────────────────────────────────────────────

    /// Check if the current token matches (after skipping whitespace),
    /// without consuming it.
    pub fn check(&mut self, expected: &Token) -> bool {
        self.skip_whitespace();
        self.peek() == expected
    }

    /// If the current token matches, consume it and return `true`.
    pub fn eat(&mut self, expected: &Token) -> bool {
        self.skip_whitespace();
        if self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Peek ahead past whitespace, returning the next non-whitespace token
    /// without consuming anything.
    pub fn peek_past_whitespace(&self) -> &Token {
        let mut i = self.pos;
        while i < self.tokens.len() {
            match &self.tokens[i].value {
                Token::Newline
                | Token::LineComment(_)
                | Token::BlockComment(_)
                | Token::DocComment(_) => i += 1,
                tok => return tok,
            }
        }
        &Token::Eof
    }

    /// Peek at the token N positions ahead of current (ignoring whitespace).
    pub fn peek_nth_past_whitespace(&self, n: usize) -> &Token {
        let mut i = self.pos;
        let mut count = 0;
        while i < self.tokens.len() {
            match &self.tokens[i].value {
                Token::Newline
                | Token::LineComment(_)
                | Token::BlockComment(_)
                | Token::DocComment(_) => i += 1,
                tok => {
                    if count == n {
                        return tok;
                    }
                    count += 1;
                    i += 1;
                }
            }
        }
        &Token::Eof
    }

    // ── Indentation ──────────────────────────────────────────────────

    /// Check if the current token is indented past `min_col`.
    /// When inside parens/brackets, indentation is always satisfied.
    pub fn is_indented_past(&mut self, min_col: u32) -> bool {
        self.skip_newlines();
        !self.is_eof() && (self.in_paren_context() || self.current_column() > min_col)
    }

    /// Check if the current token is at or past `min_col`.
    /// When inside parens/brackets, indentation is always satisfied.
    pub fn is_at_or_past(&mut self, min_col: u32) -> bool {
        self.skip_newlines();
        !self.is_eof() && (self.in_paren_context() || self.current_column() >= min_col)
    }

    // ── Collecting a doc comment ─────────────────────────────────────

    /// If the current token is a doc comment, consume and return it.
    pub fn try_doc_comment(&mut self) -> Option<Spanned<String>> {
        self.skip_newlines();
        if let Token::DocComment(text) = self.peek().clone() {
            let tok = self.advance();
            Some(Spanned::new(tok.span, text))
        } else {
            None
        }
    }

    // ── Error construction ───────────────────────────────────────────

    pub fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            span: self.peek_span(),
        }
    }

    pub fn error_at(&self, span: Span, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            span,
        }
    }

    // ── Span helpers ─────────────────────────────────────────────────

    /// Create a span from `start` to the end of the previously consumed token.
    pub fn span_from(&self, start: Position) -> Span {
        let end = if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else {
            start
        };
        Span::new(start, end)
    }

    /// Wrap a value with a span from `start` to the last consumed token.
    pub fn spanned_from<T>(&self, start: Position, value: T) -> Spanned<T> {
        Spanned::new(self.span_from(start), value)
    }

    /// Skip tokens until we reach the start of a new top-level declaration
    /// or end of file. Used for error recovery.
    ///
    /// A new declaration starts with a token at column 1 that could begin
    /// a declaration: lowercase name, `type`, `port`, `infix`, or doc comment.
    pub fn skip_to_next_declaration(&mut self) {
        loop {
            self.skip_whitespace();
            if self.is_eof() {
                break;
            }
            let col = self.current_column();
            let tok = self.peek();
            // A token at column 1 that can start a declaration.
            if col == 1
                && matches!(
                    tok,
                    Token::LowerName(_)
                        | Token::Type
                        | Token::Port
                        | Token::Infix
                        | Token::DocComment(_)
                )
            {
                break;
            }
            self.advance();
        }
    }
}

/// Produce a human-readable description of a token for error messages.
fn describe(tok: &Token) -> String {
    match tok {
        Token::Module => "`module`".into(),
        Token::Where => "`where`".into(),
        Token::Import => "`import`".into(),
        Token::As => "`as`".into(),
        Token::Exposing => "`exposing`".into(),
        Token::Type => "`type`".into(),
        Token::Alias => "`alias`".into(),
        Token::Port => "`port`".into(),
        Token::Effect => "`effect`".into(),
        Token::If => "`if`".into(),
        Token::Then => "`then`".into(),
        Token::Else => "`else`".into(),
        Token::Case => "`case`".into(),
        Token::Of => "`of`".into(),
        Token::Let => "`let`".into(),
        Token::In => "`in`".into(),
        Token::Infix => "`infix`".into(),
        Token::LeftParen => "`(`".into(),
        Token::RightParen => "`)`".into(),
        Token::LeftBracket => "`[`".into(),
        Token::RightBracket => "`]`".into(),
        Token::LeftBrace => "`{`".into(),
        Token::RightBrace => "`}`".into(),
        Token::Comma => "`,`".into(),
        Token::Pipe => "`|`".into(),
        Token::Equals => "`=`".into(),
        Token::Colon => "`:`".into(),
        Token::Dot => "`.`".into(),
        Token::DotDot => "`..`".into(),
        Token::Backslash => "`\\`".into(),
        Token::Underscore => "`_`".into(),
        Token::Arrow => "`->`".into(),
        Token::Operator(op) => format!("`{op}`"),
        Token::Minus => "`-`".into(),
        Token::LowerName(n) => format!("identifier `{n}`"),
        Token::UpperName(n) => format!("type `{n}`"),
        Token::Literal(_) => "literal".into(),
        Token::LineComment(_) => "comment".into(),
        Token::BlockComment(_) => "comment".into(),
        Token::DocComment(_) => "doc comment".into(),
        Token::Glsl(_) => "GLSL block".into(),
        Token::Newline => "newline".into(),
        Token::Eof => "end of file".into(),
    }
}

/// Parse an Elm source string into an `ElmModule`.
///
/// Returns `Err` if the module header or imports fail to parse.
/// For declaration-level errors, use [`parse_recovering`] instead to
/// get a partial AST along with the errors.
pub fn parse(source: &str) -> Result<crate::file::ElmModule, Vec<ParseError>> {
    let lexer = crate::lexer::Lexer::new(source);
    let (tokens, lex_errors) = lexer.tokenize();

    if !lex_errors.is_empty() {
        return Err(lex_errors
            .into_iter()
            .map(|e| ParseError {
                message: e.message,
                span: e.span,
            })
            .collect());
    }

    let mut parser = Parser::new(tokens);
    module::parse_module(&mut parser).map_err(|e| vec![e])
}

/// Parse an Elm source string with error recovery.
///
/// Unlike [`parse`], this always returns a (possibly partial) AST along
/// with any errors encountered. Declarations that fail to parse are skipped,
/// and parsing continues with the next declaration.
pub fn parse_recovering(source: &str) -> (Option<crate::file::ElmModule>, Vec<ParseError>) {
    let lexer = crate::lexer::Lexer::new(source);
    let (tokens, lex_errors) = lexer.tokenize();

    if !lex_errors.is_empty() {
        return (
            None,
            lex_errors
                .into_iter()
                .map(|e| ParseError {
                    message: e.message,
                    span: e.span,
                })
                .collect(),
        );
    }

    let mut parser = Parser::new(tokens);
    module::parse_module_recovering(&mut parser)
}
