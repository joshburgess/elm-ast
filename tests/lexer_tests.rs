#![cfg(not(target_arch = "wasm32"))]

use elm_ast::lexer::Lexer;
use elm_ast::literal::Literal;
use elm_ast::token::Token;

/// Helper: lex source and return just the token values (no spans), excluding Eof.
fn lex(source: &str) -> Vec<Token> {
    let (tokens, errors) = Lexer::new(source).tokenize();
    assert!(errors.is_empty(), "unexpected lex errors: {errors:?}");
    tokens
        .into_iter()
        .map(|t| t.value)
        .filter(|t| !matches!(t, Token::Eof))
        .collect()
}

/// Helper: lex and expect errors, returning (tokens, errors).
fn lex_with_errors(source: &str) -> (Vec<Token>, Vec<elm_ast::lexer::LexError>) {
    let (tokens, errors) = Lexer::new(source).tokenize();
    let toks = tokens
        .into_iter()
        .map(|t| t.value)
        .filter(|t| !matches!(t, Token::Eof))
        .collect();
    (toks, errors)
}

// ── Keywords ─────────────────────────────────────────────────────────

#[test]
fn keywords() {
    assert_eq!(lex("module"), vec![Token::Module]);
    assert_eq!(lex("where"), vec![Token::Where]);
    assert_eq!(lex("import"), vec![Token::Import]);
    assert_eq!(lex("as"), vec![Token::As]);
    assert_eq!(lex("exposing"), vec![Token::Exposing]);
    assert_eq!(lex("type"), vec![Token::Type]);
    assert_eq!(lex("alias"), vec![Token::Alias]);
    assert_eq!(lex("port"), vec![Token::Port]);
    assert_eq!(lex("if"), vec![Token::If]);
    assert_eq!(lex("then"), vec![Token::Then]);
    assert_eq!(lex("else"), vec![Token::Else]);
    assert_eq!(lex("case"), vec![Token::Case]);
    assert_eq!(lex("of"), vec![Token::Of]);
    assert_eq!(lex("let"), vec![Token::Let]);
    assert_eq!(lex("in"), vec![Token::In]);
    assert_eq!(lex("infix"), vec![Token::Infix]);
    // `left`, `right`, `non`, `effect` are NOT keywords — they are
    // contextual identifiers only special in specific declarations.
    assert_eq!(lex("left"), vec![Token::LowerName("left".into())]);
    assert_eq!(lex("right"), vec![Token::LowerName("right".into())]);
    assert_eq!(lex("non"), vec![Token::LowerName("non".into())]);
    assert_eq!(lex("effect"), vec![Token::LowerName("effect".into())]);
}

#[test]
fn keyword_prefix_is_identifier() {
    // "importing" is not the keyword "import"
    assert_eq!(lex("importing"), vec![Token::LowerName("importing".into())]);
    assert_eq!(lex("letter"), vec![Token::LowerName("letter".into())]);
    assert_eq!(lex("types"), vec![Token::LowerName("types".into())]);
}

// ── Identifiers ──────────────────────────────────────────────────────

#[test]
fn lower_identifiers() {
    assert_eq!(lex("foo"), vec![Token::LowerName("foo".into())]);
    assert_eq!(lex("myFunc"), vec![Token::LowerName("myFunc".into())]);
    assert_eq!(lex("x1"), vec![Token::LowerName("x1".into())]);
    assert_eq!(lex("foo_bar"), vec![Token::LowerName("foo_bar".into())]);
}

#[test]
fn upper_identifiers() {
    assert_eq!(lex("Maybe"), vec![Token::UpperName("Maybe".into())]);
    assert_eq!(lex("Html"), vec![Token::UpperName("Html".into())]);
    assert_eq!(lex("Cmd"), vec![Token::UpperName("Cmd".into())]);
    assert_eq!(
        lex("MyModule_V2"),
        vec![Token::UpperName("MyModule_V2".into())]
    );
}

// ── Delimiters ───────────────────────────────────────────────────────

#[test]
fn delimiters() {
    assert_eq!(
        lex("( ) [ ] { } , | = : . .. \\ _ ->"),
        vec![
            Token::LeftParen,
            Token::RightParen,
            Token::LeftBracket,
            Token::RightBracket,
            Token::LeftBrace,
            Token::RightBrace,
            Token::Comma,
            Token::Pipe,
            Token::Equals,
            Token::Colon,
            Token::Dot,
            Token::DotDot,
            Token::Backslash,
            Token::Underscore,
            Token::Arrow,
        ]
    );
}

// ── Operators ────────────────────────────────────────────────────────

#[test]
fn operators() {
    assert_eq!(lex("+"), vec![Token::Operator("+".into())]);
    assert_eq!(lex("++"), vec![Token::Operator("++".into())]);
    assert_eq!(lex("<|"), vec![Token::Operator("<|".into())]);
    assert_eq!(lex("|>"), vec![Token::Operator("|>".into())]);
    assert_eq!(lex(">>"), vec![Token::Operator(">>".into())]);
    assert_eq!(lex("<<"), vec![Token::Operator("<<".into())]);
    assert_eq!(lex("=="), vec![Token::Operator("==".into())]);
    assert_eq!(lex("/="), vec![Token::Operator("/=".into())]);
    assert_eq!(lex("<="), vec![Token::Operator("<=".into())]);
    assert_eq!(lex(">="), vec![Token::Operator(">=".into())]);
    assert_eq!(lex("&&"), vec![Token::Operator("&&".into())]);
    assert_eq!(lex("||"), vec![Token::Operator("||".into())]);
    assert_eq!(lex("::"), vec![Token::Operator("::".into())]);
    assert_eq!(lex("//"), vec![Token::Operator("//".into())]);
}

#[test]
fn minus_is_special() {
    // Standalone minus
    assert_eq!(lex("-"), vec![Token::Minus]);
    // Arrow
    assert_eq!(lex("->"), vec![Token::Arrow]);
}

// ── Integer literals ─────────────────────────────────────────────────

#[test]
fn integer_literals() {
    assert_eq!(lex("0"), vec![Token::Literal(Literal::Int(0))]);
    assert_eq!(lex("42"), vec![Token::Literal(Literal::Int(42))]);
    assert_eq!(lex("1000000"), vec![Token::Literal(Literal::Int(1000000))]);
}

#[test]
fn hex_literals() {
    assert_eq!(lex("0xFF"), vec![Token::Literal(Literal::Hex(255))]);
    assert_eq!(lex("0x1A"), vec![Token::Literal(Literal::Hex(26))]);
    assert_eq!(lex("0x00"), vec![Token::Literal(Literal::Hex(0))]);
}

// ── Float literals ───────────────────────────────────────────────────

#[test]
fn float_literals() {
    // PartialEq for Literal ignores the Float lexeme, so these still compare
    // equal to the parser-produced `Some(text)` forms.
    #[allow(clippy::approx_constant)]
    {
        assert_eq!(
            lex("3.14"),
            vec![Token::Literal(Literal::Float(3.14, None))]
        );
    }
    assert_eq!(lex("0.0"), vec![Token::Literal(Literal::Float(0.0, None))]);
    assert_eq!(
        lex("1.0e10"),
        vec![Token::Literal(Literal::Float(1.0e10, None))]
    );
    assert_eq!(
        lex("2.5E-3"),
        vec![Token::Literal(Literal::Float(2.5e-3, None))]
    );
}

// ── Char literals ────────────────────────────────────────────────────

#[test]
fn char_literals() {
    assert_eq!(lex("'a'"), vec![Token::Literal(Literal::Char('a'))]);
    assert_eq!(lex("'Z'"), vec![Token::Literal(Literal::Char('Z'))]);
    assert_eq!(lex("'0'"), vec![Token::Literal(Literal::Char('0'))]);
}

#[test]
fn char_escape_sequences() {
    assert_eq!(lex("'\\n'"), vec![Token::Literal(Literal::Char('\n'))]);
    assert_eq!(lex("'\\t'"), vec![Token::Literal(Literal::Char('\t'))]);
    assert_eq!(lex("'\\r'"), vec![Token::Literal(Literal::Char('\r'))]);
    assert_eq!(lex("'\\\\'"), vec![Token::Literal(Literal::Char('\\'))]);
    assert_eq!(lex("'\\''"), vec![Token::Literal(Literal::Char('\''))]);
}

#[test]
fn char_unicode_escape() {
    assert_eq!(lex("'\\u{0041}'"), vec![Token::Literal(Literal::Char('A'))]);
    assert_eq!(lex("'\\u{03BB}'"), vec![Token::Literal(Literal::Char('λ'))]);
}

// ── String literals ──────────────────────────────────────────────────

#[test]
fn string_literals() {
    assert_eq!(
        lex(r#""hello""#),
        vec![Token::Literal(Literal::String("hello".into()))]
    );
    assert_eq!(
        lex(r#""""#),
        vec![Token::Literal(Literal::String("".into()))]
    );
}

#[test]
fn string_escape_sequences() {
    assert_eq!(
        lex(r#""hello\nworld""#),
        vec![Token::Literal(Literal::String("hello\nworld".into()))]
    );
    assert_eq!(
        lex(r#""tab\there""#),
        vec![Token::Literal(Literal::String("tab\there".into()))]
    );
    assert_eq!(
        lex(r#""quote\"inside""#),
        vec![Token::Literal(Literal::String("quote\"inside".into()))]
    );
}

#[test]
fn multiline_string() {
    assert_eq!(
        lex(r#""""hello world""""#),
        vec![Token::Literal(Literal::MultilineString(
            "hello world".into()
        ))]
    );
}

#[test]
fn multiline_string_with_newlines() {
    let src = "\"\"\"\nline1\nline2\n\"\"\"";
    assert_eq!(
        lex(src),
        vec![Token::Literal(Literal::MultilineString(
            "\nline1\nline2\n".into()
        ))]
    );
}

#[test]
fn multiline_string_with_quotes_inside() {
    let src = r#""""She said "hi""""#;
    assert_eq!(
        lex(src),
        vec![Token::Literal(Literal::MultilineString(
            r#"She said "hi"#.into()
        ))]
    );
}

// ── Comments ─────────────────────────────────────────────────────────

#[test]
fn line_comment() {
    assert_eq!(
        lex("-- this is a comment"),
        vec![Token::LineComment(" this is a comment".into())]
    );
}

#[test]
fn line_comment_preserves_content() {
    assert_eq!(
        lex("-- TODO: fix this"),
        vec![Token::LineComment(" TODO: fix this".into())]
    );
}

#[test]
fn block_comment() {
    assert_eq!(
        lex("{- hello -}"),
        vec![Token::BlockComment(" hello ".into())]
    );
}

#[test]
fn nested_block_comment() {
    assert_eq!(
        lex("{- outer {- inner -} still outer -}"),
        vec![Token::BlockComment(
            " outer {- inner -} still outer ".into()
        )]
    );
}

#[test]
fn deeply_nested_block_comment() {
    assert_eq!(
        lex("{- a {- b {- c -} b -} a -}"),
        vec![Token::BlockComment(" a {- b {- c -} b -} a ".into())]
    );
}

#[test]
fn doc_comment() {
    assert_eq!(
        lex("{-| This is documentation -}"),
        vec![Token::DocComment(" This is documentation ".into())]
    );
}

// ── GLSL ─────────────────────────────────────────────────────────────

#[test]
fn glsl_block() {
    let src = "[glsl|void main() { gl_FragColor = vec4(1.0); }|]";
    assert_eq!(
        lex(src),
        vec![Token::Glsl(
            "void main() { gl_FragColor = vec4(1.0); }".into()
        )]
    );
}

#[test]
fn glsl_block_multiline() {
    let src = "[glsl|\nprecision mediump float;\nvoid main() {}\n|]";
    assert_eq!(
        lex(src),
        vec![Token::Glsl(
            "\nprecision mediump float;\nvoid main() {}\n".into()
        )]
    );
}

// ── Newlines and layout ──────────────────────────────────────────────

#[test]
fn newlines_are_emitted() {
    let tokens = lex("foo\nbar");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("foo".into()),
            Token::Newline,
            Token::LowerName("bar".into()),
        ]
    );
}

#[test]
fn blank_lines() {
    let tokens = lex("foo\n\nbar");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("foo".into()),
            Token::Newline,
            Token::Newline,
            Token::LowerName("bar".into()),
        ]
    );
}

// ── Span tracking ────────────────────────────────────────────────────

#[test]
fn spans_are_accurate() {
    let (tokens, errors) = Lexer::new("foo bar").tokenize();
    assert!(errors.is_empty());

    // "foo" at line 1, columns 1-4
    let foo = &tokens[0];
    assert_eq!(foo.span.start.line, 1);
    assert_eq!(foo.span.start.column, 1);
    assert_eq!(foo.span.end.column, 4);

    // "bar" at line 1, columns 5-8
    let bar = &tokens[1];
    assert_eq!(bar.span.start.line, 1);
    assert_eq!(bar.span.start.column, 5);
    assert_eq!(bar.span.end.column, 8);
}

#[test]
fn spans_across_lines() {
    let (tokens, errors) = Lexer::new("foo\nbar").tokenize();
    assert!(errors.is_empty());

    let foo = &tokens[0];
    assert_eq!(foo.span.start.line, 1);

    let bar = &tokens[2]; // tokens[1] is Newline
    assert_eq!(bar.span.start.line, 2);
    assert_eq!(bar.span.start.column, 1);
}

// ── Compound expressions ─────────────────────────────────────────────

#[test]
fn module_header() {
    let tokens = lex("module Main exposing (..)");
    assert_eq!(
        tokens,
        vec![
            Token::Module,
            Token::UpperName("Main".into()),
            Token::Exposing,
            Token::LeftParen,
            Token::DotDot,
            Token::RightParen,
        ]
    );
}

#[test]
fn import_statement() {
    let tokens = lex("import Html.Attributes as HA exposing (class, style)");
    assert_eq!(
        tokens,
        vec![
            Token::Import,
            Token::UpperName("Html".into()),
            Token::Dot,
            Token::UpperName("Attributes".into()),
            Token::As,
            Token::UpperName("HA".into()),
            Token::Exposing,
            Token::LeftParen,
            Token::LowerName("class".into()),
            Token::Comma,
            Token::LowerName("style".into()),
            Token::RightParen,
        ]
    );
}

#[test]
fn function_definition() {
    let tokens = lex("add x y = x + y");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("add".into()),
            Token::LowerName("x".into()),
            Token::LowerName("y".into()),
            Token::Equals,
            Token::LowerName("x".into()),
            Token::Operator("+".into()),
            Token::LowerName("y".into()),
        ]
    );
}

#[test]
fn type_annotation() {
    let tokens = lex("add : Int -> Int -> Int");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("add".into()),
            Token::Colon,
            Token::UpperName("Int".into()),
            Token::Arrow,
            Token::UpperName("Int".into()),
            Token::Arrow,
            Token::UpperName("Int".into()),
        ]
    );
}

#[test]
fn custom_type() {
    let tokens = lex("type Msg = Increment | Decrement");
    assert_eq!(
        tokens,
        vec![
            Token::Type,
            Token::UpperName("Msg".into()),
            Token::Equals,
            Token::UpperName("Increment".into()),
            Token::Pipe,
            Token::UpperName("Decrement".into()),
        ]
    );
}

#[test]
fn lambda_expression() {
    let tokens = lex("\\x -> x + 1");
    assert_eq!(
        tokens,
        vec![
            Token::Backslash,
            Token::LowerName("x".into()),
            Token::Arrow,
            Token::LowerName("x".into()),
            Token::Operator("+".into()),
            Token::Literal(Literal::Int(1)),
        ]
    );
}

#[test]
fn record_expression() {
    let tokens = lex("{ name = \"Alice\", age = 30 }");
    assert_eq!(
        tokens,
        vec![
            Token::LeftBrace,
            Token::LowerName("name".into()),
            Token::Equals,
            Token::Literal(Literal::String("Alice".into())),
            Token::Comma,
            Token::LowerName("age".into()),
            Token::Equals,
            Token::Literal(Literal::Int(30)),
            Token::RightBrace,
        ]
    );
}

#[test]
fn record_update() {
    let tokens = lex("{ model | count = 0 }");
    assert_eq!(
        tokens,
        vec![
            Token::LeftBrace,
            Token::LowerName("model".into()),
            Token::Pipe,
            Token::LowerName("count".into()),
            Token::Equals,
            Token::Literal(Literal::Int(0)),
            Token::RightBrace,
        ]
    );
}

#[test]
fn case_expression() {
    let tokens = lex("case msg of\n    Increment -> model + 1");
    assert_eq!(
        tokens,
        vec![
            Token::Case,
            Token::LowerName("msg".into()),
            Token::Of,
            Token::Newline,
            Token::UpperName("Increment".into()),
            Token::Arrow,
            Token::LowerName("model".into()),
            Token::Operator("+".into()),
            Token::Literal(Literal::Int(1)),
        ]
    );
}

#[test]
fn list_expression() {
    let tokens = lex("[ 1, 2, 3 ]");
    assert_eq!(
        tokens,
        vec![
            Token::LeftBracket,
            Token::Literal(Literal::Int(1)),
            Token::Comma,
            Token::Literal(Literal::Int(2)),
            Token::Comma,
            Token::Literal(Literal::Int(3)),
            Token::RightBracket,
        ]
    );
}

#[test]
fn pipeline_expression() {
    let tokens = lex("list |> List.map f |> List.filter g");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("list".into()),
            Token::Operator("|>".into()),
            Token::UpperName("List".into()),
            Token::Dot,
            Token::LowerName("map".into()),
            Token::LowerName("f".into()),
            Token::Operator("|>".into()),
            Token::UpperName("List".into()),
            Token::Dot,
            Token::LowerName("filter".into()),
            Token::LowerName("g".into()),
        ]
    );
}

#[test]
fn record_access_function() {
    let tokens = lex(".name");
    assert_eq!(tokens, vec![Token::Dot, Token::LowerName("name".into())]);
}

// ── Error recovery ───────────────────────────────────────────────────

#[test]
fn unterminated_string() {
    let (_, errors) = lex_with_errors("\"unterminated");
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("unterminated"));
}

#[test]
fn unterminated_block_comment() {
    let (_, errors) = lex_with_errors("{- never closed");
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("unterminated"));
}

#[test]
fn empty_hex_literal() {
    let (_, errors) = lex_with_errors("0x ");
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("hex"));
}

// ── Edge cases ───────────────────────────────────────────────────────

#[test]
fn empty_source() {
    assert_eq!(lex(""), vec![]);
}

#[test]
fn only_whitespace() {
    assert_eq!(lex("   "), vec![]);
}

#[test]
fn only_newlines() {
    assert_eq!(lex("\n\n"), vec![Token::Newline, Token::Newline]);
}

#[test]
fn underscore_standalone() {
    assert_eq!(lex("_"), vec![Token::Underscore]);
}

#[test]
fn consecutive_operators() {
    // Two separate operators separated by space
    let tokens = lex("+ -");
    assert_eq!(tokens, vec![Token::Operator("+".into()), Token::Minus]);
}

#[test]
fn number_followed_by_dot_identifier() {
    // `model.count` — not a float
    let tokens = lex("model.count");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("model".into()),
            Token::Dot,
            Token::LowerName("count".into()),
        ]
    );
}

#[test]
fn let_in_block() {
    let src = "let\n    x = 1\nin\n    x";
    let tokens = lex(src);
    assert_eq!(
        tokens,
        vec![
            Token::Let,
            Token::Newline,
            Token::LowerName("x".into()),
            Token::Equals,
            Token::Literal(Literal::Int(1)),
            Token::Newline,
            Token::In,
            Token::Newline,
            Token::LowerName("x".into()),
        ]
    );
}

#[test]
fn if_then_else() {
    let tokens = lex("if True then 1 else 0");
    assert_eq!(
        tokens,
        vec![
            Token::If,
            Token::UpperName("True".into()),
            Token::Then,
            Token::Literal(Literal::Int(1)),
            Token::Else,
            Token::Literal(Literal::Int(0)),
        ]
    );
}

#[test]
fn port_declaration() {
    let tokens = lex("port sendMessage : String -> Cmd msg");
    assert_eq!(
        tokens,
        vec![
            Token::Port,
            Token::LowerName("sendMessage".into()),
            Token::Colon,
            Token::UpperName("String".into()),
            Token::Arrow,
            Token::UpperName("Cmd".into()),
            Token::LowerName("msg".into()),
        ]
    );
}

#[test]
fn infix_declaration() {
    let tokens = lex("infix left 6 (+) = add");
    assert_eq!(
        tokens,
        vec![
            Token::Infix,
            Token::LowerName("left".into()),
            Token::Literal(Literal::Int(6)),
            Token::LeftParen,
            Token::Operator("+".into()),
            Token::RightParen,
            Token::Equals,
            Token::LowerName("add".into()),
        ]
    );
}

#[test]
fn cons_operator() {
    let tokens = lex("x :: xs");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("x".into()),
            Token::Operator("::".into()),
            Token::LowerName("xs".into()),
        ]
    );
}

#[test]
fn tuple_type() {
    let tokens = lex("( Int, String )");
    assert_eq!(
        tokens,
        vec![
            Token::LeftParen,
            Token::UpperName("Int".into()),
            Token::Comma,
            Token::UpperName("String".into()),
            Token::RightParen,
        ]
    );
}

#[test]
fn negative_number() {
    // In Elm, `-42` is parsed as negation of 42, not a negative literal.
    // The lexer should emit Minus + Int.
    let tokens = lex("-42");
    assert_eq!(tokens, vec![Token::Minus, Token::Literal(Literal::Int(42))]);
}

#[test]
fn effect_module() {
    let tokens = lex("effect module Task where { command = MyCmd } exposing (..)");
    assert_eq!(
        tokens,
        vec![
            Token::LowerName("effect".into()),
            Token::Module,
            Token::UpperName("Task".into()),
            Token::Where,
            Token::LeftBrace,
            Token::LowerName("command".into()),
            Token::Equals,
            Token::UpperName("MyCmd".into()),
            Token::RightBrace,
            Token::Exposing,
            Token::LeftParen,
            Token::DotDot,
            Token::RightParen,
        ]
    );
}
