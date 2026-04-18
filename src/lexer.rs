use crate::literal::Literal;
use crate::node::Spanned;
use crate::span::{Position, Span};
use crate::token::Token;

/// An error encountered during lexing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.span.start.line, self.span.start.column, self.message
        )
    }
}

impl std::error::Error for LexError {}

/// The Elm lexer. Converts source text into a stream of spanned tokens.
///
/// The lexer follows the elm/compiler approach: it produces flat tokens with
/// accurate source positions, and leaves indentation-sensitive layout to the
/// parser. Newline tokens are emitted so the parser can track indentation.
pub struct Lexer<'src> {
    source: &'src str,
    bytes: &'src [u8],
    /// Current byte offset into source.
    offset: usize,
    /// Current 1-based line number.
    line: u32,
    /// Current 1-based column number.
    column: u32,
    /// Accumulated errors (the lexer tries to recover and continue).
    errors: Vec<LexError>,
}

impl<'src> Lexer<'src> {
    /// Create a new lexer for the given source text.
    pub fn new(source: &'src str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            offset: 0,
            line: 1,
            column: 1,
            errors: Vec::new(),
        }
    }

    /// Tokenize the entire source, returning all tokens and any errors.
    pub fn tokenize(mut self) -> (Vec<Spanned<Token>>, Vec<LexError>) {
        let mut tokens = Vec::new();

        loop {
            self.skip_spaces();
            if self.is_eof() {
                tokens.push(self.make_span_here(Token::Eof));
                break;
            }
            match self.next_token() {
                Ok(tok) => tokens.push(tok),
                Err(e) => {
                    self.errors.push(e);
                    // Skip one byte to attempt recovery.
                    self.advance();
                }
            }
        }

        (tokens, self.errors)
    }

    /// Consume any errors accumulated during lexing.
    pub fn into_errors(self) -> Vec<LexError> {
        self.errors
    }

    // ── Core helpers ─────────────────────────────────────────────────

    fn is_eof(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(offset).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.bytes.get(self.offset + 1).copied()
    }

    fn current_pos(&self) -> Position {
        Position {
            offset: self.offset,
            line: self.line,
            column: self.column,
        }
    }

    fn make_span(&self, start: Position, value: Token) -> Spanned<Token> {
        Spanned::new(
            Span {
                start,
                end: self.current_pos(),
            },
            value,
        )
    }

    fn make_span_here(&self, value: Token) -> Spanned<Token> {
        let pos = self.current_pos();
        Spanned::new(
            Span {
                start: pos,
                end: pos,
            },
            value,
        )
    }

    fn make_error(&self, start: Position, message: impl Into<String>) -> LexError {
        LexError {
            message: message.into(),
            span: Span {
                start,
                end: self.current_pos(),
            },
        }
    }

    /// Advance by one byte, updating line/column tracking.
    fn advance(&mut self) {
        if let Some(b) = self.peek() {
            self.offset += 1;
            if b == b'\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }

    /// Advance by `n` bytes (only call when you know they're non-newline ASCII).
    fn advance_n(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    /// Skip horizontal whitespace (spaces and tabs), but NOT newlines.
    fn skip_spaces(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Read a slice of the source from `start` offset to current offset.
    fn slice_from(&self, start: usize) -> &'src str {
        &self.source[start..self.offset]
    }

    // ── Main dispatch ────────────────────────────────────────────────

    fn next_token(&mut self) -> Result<Spanned<Token>, LexError> {
        let start = self.current_pos();
        let b = self.peek().unwrap();

        match b {
            b'\n' => {
                self.advance();
                Ok(self.make_span(start, Token::Newline))
            }

            // Single-line comment or operator starting with -
            b'-' => {
                if self.peek_next() == Some(b'-') {
                    self.lex_line_comment(start)
                } else if self.peek_next() == Some(b'>') {
                    self.advance_n(2);
                    Ok(self.make_span(start, Token::Arrow))
                } else {
                    self.lex_operator_or_minus(start)
                }
            }

            // Block comment or left brace
            b'{' => {
                if self.peek_next() == Some(b'-') {
                    self.lex_block_comment(start)
                } else {
                    self.advance();
                    Ok(self.make_span(start, Token::LeftBrace))
                }
            }

            // Delimiters
            b'(' => {
                self.advance();
                Ok(self.make_span(start, Token::LeftParen))
            }
            b')' => {
                self.advance();
                Ok(self.make_span(start, Token::RightParen))
            }
            b']' => {
                self.advance();
                Ok(self.make_span(start, Token::RightBracket))
            }
            b'}' => {
                self.advance();
                Ok(self.make_span(start, Token::RightBrace))
            }
            b',' => {
                self.advance();
                Ok(self.make_span(start, Token::Comma))
            }

            // GLSL block or left bracket
            b'[' => {
                if self.matches_ahead(b"[glsl|") {
                    self.lex_glsl(start)
                } else {
                    self.advance();
                    Ok(self.make_span(start, Token::LeftBracket))
                }
            }

            // Backslash (lambda)
            b'\\' => {
                self.advance();
                Ok(self.make_span(start, Token::Backslash))
            }

            // Dot or dot-dot
            b'.' => {
                if self.peek_next() == Some(b'.') {
                    self.advance_n(2);
                    Ok(self.make_span(start, Token::DotDot))
                } else {
                    self.advance();
                    Ok(self.make_span(start, Token::Dot))
                }
            }

            // Colon (could be `::` operator)
            b':' => {
                if self.peek_next() == Some(b':') {
                    self.advance_n(2);
                    Ok(self.make_span(start, Token::Operator("::".into())))
                } else {
                    self.advance();
                    Ok(self.make_span(start, Token::Colon))
                }
            }

            // Pipe: standalone `|`, or operator starting with `|` (`||`, `|>`, `|=`, `|.`, etc.)
            b'|' => {
                match self.peek_next() {
                    Some(b'.') => {
                        // Special case: `|.` — the `.` is not normally an operator char
                        // but `|.` is a valid Elm operator (used in elm/parser).
                        self.advance_n(2);
                        Ok(self.make_span(start, Token::Operator("|.".into())))
                    }
                    Some(next) if is_operator_char(next) => {
                        // Multi-char operator starting with `|`: `||`, `|>`, `|=`, etc.
                        self.lex_operator(start)
                    }
                    _ => {
                        self.advance();
                        Ok(self.make_span(start, Token::Pipe))
                    }
                }
            }

            // Equals (could be `==`)
            b'=' => {
                if self.peek_next() == Some(b'=') {
                    self.advance_n(2);
                    Ok(self.make_span(start, Token::Operator("==".into())))
                } else {
                    self.advance();
                    Ok(self.make_span(start, Token::Equals))
                }
            }

            // Char literal
            b'\'' => self.lex_char(start),

            // String literal (single-line or multi-line)
            b'"' => self.lex_string(start),

            // Number (digit or 0x for hex)
            b'0'..=b'9' => self.lex_number(start),

            // Underscore (wildcard)
            b'_' => {
                self.advance();
                // If followed by alphanumeric, it's part of an identifier (error in Elm,
                // but we lex it as a lower name for error recovery).
                if self
                    .peek()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    let text_start = start.offset;
                    while self
                        .peek()
                        .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        self.advance();
                    }
                    let name = self.slice_from(text_start).to_string();
                    Ok(self.make_span(start, Token::LowerName(name)))
                } else {
                    Ok(self.make_span(start, Token::Underscore))
                }
            }

            // Lowercase identifier or keyword
            b if b.is_ascii_lowercase() => self.lex_lower(start),

            // Uppercase identifier
            b if b.is_ascii_uppercase() => self.lex_upper(start),

            // Operator characters
            b if is_operator_char(b) => self.lex_operator(start),

            _ => {
                // Skip any UTF-8 continuation bytes we may have landed inside.
                while self.offset < self.source.len() && !self.source.is_char_boundary(self.offset)
                {
                    self.advance();
                }
                if self.offset >= self.source.len() {
                    return Err(self.make_error(start, "unexpected end of input".to_string()));
                }
                let ch = self.source[self.offset..].chars().next().unwrap();
                let ch_len = ch.len_utf8();
                for _ in 0..ch_len {
                    self.advance();
                }
                Err(self.make_error(start, format!("unexpected character: '{ch}'")))
            }
        }
    }

    // ── Identifiers ──────────────────────────────────────────────────

    fn lex_lower(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        let text_start = start.offset;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.advance();
        }
        let text = self.slice_from(text_start);

        let token = Token::keyword(text).unwrap_or_else(|| Token::LowerName(text.to_string()));

        Ok(self.make_span(start, token))
    }

    fn lex_upper(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        let text_start = start.offset;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.advance();
        }
        let text = self.slice_from(text_start).to_string();
        Ok(self.make_span(start, Token::UpperName(text)))
    }

    // ── Operators ────────────────────────────────────────────────────

    fn lex_operator(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        let text_start = start.offset;
        while self.peek().is_some_and(is_operator_char) {
            self.advance();
        }
        let text = self.slice_from(text_start).to_string();
        Ok(self.make_span(start, Token::Operator(text)))
    }

    fn lex_operator_or_minus(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        let text_start = start.offset;
        while self.peek().is_some_and(is_operator_char) {
            self.advance();
        }
        let text = self.slice_from(text_start);

        if text == "-" {
            Ok(self.make_span(start, Token::Minus))
        } else {
            Ok(self.make_span(start, Token::Operator(text.to_string())))
        }
    }

    // ── Numbers ──────────────────────────────────────────────────────

    fn lex_number(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        // Check for hex literal: 0x...
        if self.peek() == Some(b'0') && self.peek_next() == Some(b'x') {
            return self.lex_hex(start);
        }

        let text_start = start.offset;
        let mut is_float = false;

        // Integer part.
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.advance();
        }

        // Fractional part.
        if self.peek() == Some(b'.') && self.peek_next().is_some_and(|b| b.is_ascii_digit()) {
            is_float = true;
            self.advance(); // consume '.'
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.advance();
            }
        }

        // Exponent part.
        if self.peek().is_some_and(|b| b == b'e' || b == b'E') {
            is_float = true;
            self.advance(); // consume 'e'/'E'
            if self.peek().is_some_and(|b| b == b'+' || b == b'-') {
                self.advance();
            }
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.advance();
            }
        }

        let text = self.slice_from(text_start);

        if is_float {
            match text.parse::<f64>() {
                Ok(v) => Ok(self.make_span(
                    start,
                    Token::Literal(Literal::Float(v, Some(text.to_string()))),
                )),
                Err(_) => Err(self.make_error(start, format!("invalid float literal: {text}"))),
            }
        } else {
            match text.parse::<i64>() {
                Ok(v) => Ok(self.make_span(start, Token::Literal(Literal::Int(v)))),
                Err(_) => Err(self.make_error(start, format!("invalid integer literal: {text}"))),
            }
        }
    }

    fn lex_hex(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance_n(2); // skip "0x"
        let hex_start = self.offset;

        while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
            self.advance();
        }

        if self.offset == hex_start {
            return Err(self.make_error(start, "expected hex digits after 0x"));
        }

        let hex_text = &self.source[hex_start..self.offset];
        match i64::from_str_radix(hex_text, 16) {
            Ok(v) => Ok(self.make_span(start, Token::Literal(Literal::Hex(v)))),
            Err(_) => Err(self.make_error(start, format!("invalid hex literal: 0x{hex_text}"))),
        }
    }

    // ── Char literals ────────────────────────────────────────────────

    fn lex_char(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance(); // skip opening '

        let ch = if self.peek() == Some(b'\\') {
            self.lex_escape_char()?
        } else if let Some(b) = self.peek() {
            if b == b'\'' {
                return Err(self.make_error(start, "empty character literal"));
            }
            let ch = self.source[self.offset..].chars().next().unwrap();
            let len = ch.len_utf8();
            for _ in 0..len {
                self.advance();
            }
            ch
        } else {
            return Err(self.make_error(start, "unterminated character literal"));
        };

        if self.peek() == Some(b'\'') {
            self.advance();
            Ok(self.make_span(start, Token::Literal(Literal::Char(ch))))
        } else {
            Err(self.make_error(start, "unterminated character literal"))
        }
    }

    fn lex_escape_char(&mut self) -> Result<char, LexError> {
        let start = self.current_pos();
        self.advance(); // skip '\'

        match self.peek() {
            Some(b'n') => {
                self.advance();
                Ok('\n')
            }
            Some(b'r') => {
                self.advance();
                Ok('\r')
            }
            Some(b't') => {
                self.advance();
                Ok('\t')
            }
            Some(b'\\') => {
                self.advance();
                Ok('\\')
            }
            Some(b'\'') => {
                self.advance();
                Ok('\'')
            }
            Some(b'"') => {
                self.advance();
                Ok('"')
            }
            Some(b'u') => self.lex_unicode_escape(),
            _ => Err(self.make_error(start, "invalid escape sequence")),
        }
    }

    fn lex_unicode_escape(&mut self) -> Result<char, LexError> {
        let start = self.current_pos();
        self.advance(); // skip 'u'

        if self.peek() != Some(b'{') {
            return Err(self.make_error(start, "expected '{' after \\u"));
        }
        self.advance();

        let hex_start = self.offset;
        while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
            self.advance();
        }

        if self.offset == hex_start {
            return Err(self.make_error(start, "expected hex digits in unicode escape"));
        }

        let hex_text = &self.source[hex_start..self.offset];

        if self.peek() != Some(b'}') {
            return Err(self.make_error(start, "expected '}' to close unicode escape"));
        }
        self.advance();

        let code_point = u32::from_str_radix(hex_text, 16)
            .map_err(|_| self.make_error(start, "invalid unicode escape"))?;

        char::from_u32(code_point).ok_or_else(|| {
            self.make_error(start, format!("invalid unicode code point: {hex_text}"))
        })
    }

    // ── String literals ──────────────────────────────────────────────

    fn lex_string(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        // Check for multi-line string: """
        if self.peek_at(self.offset + 1) == Some(b'"')
            && self.peek_at(self.offset + 2) == Some(b'"')
        {
            return self.lex_multiline_string(start);
        }

        self.advance(); // skip opening "
        let mut value = String::new();

        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    return Err(self.make_error(start, "unterminated string literal"));
                }
                Some(b'"') => {
                    self.advance();
                    return Ok(self.make_span(start, Token::Literal(Literal::String(value))));
                }
                Some(b'\\') => {
                    let ch = self.lex_escape_char()?;
                    value.push(ch);
                }
                Some(_) => {
                    let ch = self.source[self.offset..].chars().next().unwrap();
                    let len = ch.len_utf8();
                    value.push(ch);
                    for _ in 0..len {
                        self.advance();
                    }
                }
            }
        }
    }

    fn lex_multiline_string(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance_n(3); // skip opening """
        let mut value = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(self.make_error(start, "unterminated multi-line string"));
                }
                Some(b'"') => {
                    if self.peek_at(self.offset + 1) == Some(b'"')
                        && self.peek_at(self.offset + 2) == Some(b'"')
                    {
                        self.advance_n(3);
                        return Ok(
                            self.make_span(start, Token::Literal(Literal::MultilineString(value)))
                        );
                    } else {
                        value.push('"');
                        self.advance();
                    }
                }
                Some(b'\\') => {
                    let ch = self.lex_escape_char()?;
                    value.push(ch);
                }
                Some(_) => {
                    let ch = self.source[self.offset..].chars().next().unwrap();
                    let len = ch.len_utf8();
                    value.push(ch);
                    for _ in 0..len {
                        self.advance();
                    }
                }
            }
        }
    }

    // ── Comments ─────────────────────────────────────────────────────

    fn lex_line_comment(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance_n(2); // skip "--"
        let text_start = self.offset;

        while self.peek().is_some_and(|b| b != b'\n') {
            self.advance();
        }

        let text = self.slice_from(text_start).to_string();
        Ok(self.make_span(start, Token::LineComment(text)))
    }

    fn lex_block_comment(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance_n(2); // skip "{-"

        // Check for doc comment: {-|
        let is_doc = self.peek() == Some(b'|');

        let text_start = self.offset;
        let mut depth: u32 = 1;

        while depth > 0 {
            match self.peek() {
                None => {
                    return Err(self.make_error(start, "unterminated block comment"));
                }
                Some(b'{') => {
                    self.advance();
                    if self.peek() == Some(b'-') {
                        self.advance();
                        depth += 1;
                    }
                }
                Some(b'-') => {
                    self.advance();
                    if self.peek() == Some(b'}') {
                        self.advance();
                        depth -= 1;
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        // The text is between "{-" and "-}" (excluding the closing "-}").
        let text_end = self.offset - 2;
        let text = self.source[text_start..text_end].to_string();

        if is_doc {
            // Strip the leading '|' from doc comment content.
            let doc_text = text.strip_prefix('|').unwrap_or(&text).to_string();
            Ok(self.make_span(start, Token::DocComment(doc_text)))
        } else {
            Ok(self.make_span(start, Token::BlockComment(text)))
        }
    }

    // ── GLSL ─────────────────────────────────────────────────────────

    fn lex_glsl(&mut self, start: Position) -> Result<Spanned<Token>, LexError> {
        self.advance_n(6); // skip "[glsl|"
        let text_start = self.offset;

        loop {
            match self.peek() {
                None => {
                    return Err(self.make_error(start, "unterminated GLSL block"));
                }
                Some(b'|') => {
                    if self.peek_next() == Some(b']') {
                        let text = self.source[text_start..self.offset].to_string();
                        self.advance_n(2); // skip "|]"
                        return Ok(self.make_span(start, Token::Glsl(text)));
                    }
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ── Utility ──────────────────────────────────────────────────────

    fn matches_ahead(&self, pattern: &[u8]) -> bool {
        self.bytes
            .get(self.offset..self.offset + pattern.len())
            .is_some_and(|slice| slice == pattern)
    }
}

/// Returns `true` if the byte is an Elm operator character.
///
/// Elm operators are composed of: `+-*/<>=!&|^~%?@#$`
/// Note: `.` and `:` are handled specially (they're delimiters that can
/// start operators like `::` but are not general operator chars).
fn is_operator_char(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'%'
            | b'?'
            | b'@'
            | b'#'
            | b'$'
    )
}
