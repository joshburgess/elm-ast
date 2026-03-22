use elm_ast_rs::builder;
use elm_ast_rs::declaration::Declaration;
use elm_ast_rs::expr::Expr;
use elm_ast_rs::file::{associate_comments, extract_comments};
use elm_ast_rs::literal::Literal;
use elm_ast_rs::{parse, parse_recovering, Lexer};
use elm_ast_rs::pattern::Pattern;
use elm_ast_rs::type_annotation::TypeAnnotation;

fn parse_ok(source: &str) -> elm_ast_rs::file::ElmModule {
    match parse(source) {
        Ok(m) => m,
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            panic!("parse failed");
        }
    }
}

fn get_body(m: &elm_ast_rs::file::ElmModule) -> &Expr {
    match &m.declarations[0].value {
        Declaration::FunctionDeclaration(f) => &f.declaration.value.body.value,
        _ => panic!("expected function"),
    }
}

fn round_trip(source: &str) {
    let ast1 = parse(source).unwrap();
    let printed = elm_ast_rs::print::print(&ast1);
    parse(&printed).unwrap_or_else(|e| {
        eprintln!("--- printed ---\n{printed}\n---");
        panic!("round-trip failed: {e:?}");
    });
}

// ── Chained if-else ──────────────────────────────────────────────────

#[test]
fn chained_if_else() {
    let src = "\
module Main exposing (..)

x =
    if a then 1
    else if b then 2
    else if c then 3
    else 4
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::IfElse { branches, else_branch } => {
            // The first branch is a, the else is a nested if.
            // Our parser represents chained if-else as nested IfElse.
            assert_eq!(branches.len(), 1);
            assert!(matches!(&else_branch.value, Expr::IfElse { .. }));
        }
        other => panic!("expected IfElse, got {other:?}"),
    }
    round_trip(src);
}

// ── Deeply nested operators ──────────────────────────────────────────

#[test]
fn deeply_nested_operators() {
    let src = "\
module Main exposing (..)

x = 1 + 2 * 3 - 4 / 5 + 6
";
    let _m = parse_ok(src);
    round_trip(src);
}

#[test]
fn mixed_associativity() {
    let src = "\
module Main exposing (..)

x = a |> b |> c <| d
";
    parse_ok(src);
    round_trip(src);
}

#[test]
fn right_associative_cons() {
    let src = "\
module Main exposing (..)

x = 1 :: 2 :: 3 :: []
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::OperatorApplication { operator, .. } => assert_eq!(operator, "::"),
        other => panic!("expected OperatorApplication, got {other:?}"),
    }
    round_trip(src);
}

#[test]
fn right_associative_append() {
    let src = "\
module Main exposing (..)

x = \"a\" ++ \"b\" ++ \"c\"
";
    parse_ok(src);
    round_trip(src);
}

// ── Negative patterns ────────────────────────────────────────────────

#[test]
fn negative_int_pattern() {
    let src = "\
module Main exposing (..)

f x =
    case x of
        -1 ->
            True
        _ ->
            False
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::CaseOf { branches, .. } => {
            assert!(matches!(
                &branches[0].pattern.value,
                Pattern::Literal(Literal::Int(-1))
            ));
        }
        _ => panic!("expected CaseOf"),
    }
    round_trip(src);
}

// ── Cons pattern ─────────────────────────────────────────────────────

#[test]
fn cons_pattern_in_case() {
    let src = "\
module Main exposing (..)

f xs =
    case xs of
        x :: rest ->
            x
        [] ->
            0
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::CaseOf { branches, .. } => {
            assert!(matches!(&branches[0].pattern.value, Pattern::Cons { .. }));
            assert!(matches!(&branches[1].pattern.value, Pattern::List(elems) if elems.is_empty()));
        }
        _ => panic!("expected CaseOf"),
    }
    round_trip(src);
}

// ── As pattern ───────────────────────────────────────────────────────

#[test]
fn as_pattern() {
    let src = "\
module Main exposing (..)

f x =
    case x of
        (Just val) as maybe ->
            maybe
        Nothing ->
            x
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::CaseOf { branches, .. } => {
            assert!(matches!(&branches[0].pattern.value, Pattern::As { .. }));
        }
        _ => panic!("expected CaseOf"),
    }
    round_trip(src);
}

// ── Hex literal pattern ──────────────────────────────────────────────

#[test]
fn hex_literal() {
    let src = "\
module Main exposing (..)

x = 0xFF
";
    let m = parse_ok(src);
    assert!(matches!(get_body(&m), Expr::Literal(Literal::Hex(255))));
    round_trip(src);
}

// ── Record access chain ──────────────────────────────────────────────

#[test]
fn record_access_chain() {
    let src = "\
module Main exposing (..)

x = model.user.name
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::RecordAccess { field, record } => {
            assert_eq!(field.value, "name");
            assert!(matches!(
                &record.value,
                Expr::RecordAccess { field: inner_field, .. } if inner_field.value == "user"
            ));
        }
        other => panic!("expected nested RecordAccess, got {other:?}"),
    }
    round_trip(src);
}

// ── Record access function ───────────────────────────────────────────

#[test]
fn record_access_function_in_map() {
    let src = "\
module Main exposing (..)

x = List.map .name users
";
    parse_ok(src);
    round_trip(src);
}

// ── Empty record ─────────────────────────────────────────────────────

#[test]
fn empty_record() {
    let src = "\
module Main exposing (..)

x = {}
";
    let m = parse_ok(src);
    assert!(matches!(get_body(&m), Expr::Record(fields) if fields.is_empty()));
    round_trip(src);
}

// ── Empty list ───────────────────────────────────────────────────────

#[test]
fn empty_list() {
    let src = "\
module Main exposing (..)

x = []
";
    let m = parse_ok(src);
    assert!(matches!(get_body(&m), Expr::List(elems) if elems.is_empty()));
    round_trip(src);
}

// ── Unit type and expression ─────────────────────────────────────────

#[test]
fn unit_type_and_expr() {
    let src = "\
module Main exposing (..)

x : ()
x = ()
";
    let m = parse_ok(src);
    match &m.declarations[0].value {
        Declaration::FunctionDeclaration(f) => {
            let sig = f.signature.as_ref().unwrap();
            assert!(matches!(&sig.value.type_annotation.value, TypeAnnotation::Unit));
            assert!(matches!(&f.declaration.value.body.value, Expr::Unit));
        }
        _ => panic!("expected function"),
    }
    round_trip(src);
}

// ── Qualified constructor ────────────────────────────────────────────

#[test]
fn qualified_constructor_pattern() {
    let src = "\
module Main exposing (..)

f x =
    case x of
        Maybe.Just val ->
            val
        Maybe.Nothing ->
            0
";
    parse_ok(src);
    round_trip(src);
}

// ── Qualified value ──────────────────────────────────────────────────

#[test]
fn deeply_qualified_value() {
    let src = "\
module Main exposing (..)

x = Dict.Dict.empty
";
    parse_ok(src);
    round_trip(src);
}

// ── Tuple type ───────────────────────────────────────────────────────

#[test]
fn triple_tuple_type() {
    let src = "\
module Main exposing (..)

x : ( Int, String, Bool )
x = ( 1, \"hello\", True )
";
    let m = parse_ok(src);
    match &m.declarations[0].value {
        Declaration::FunctionDeclaration(f) => {
            let sig = f.signature.as_ref().unwrap();
            assert!(matches!(
                &sig.value.type_annotation.value,
                TypeAnnotation::Tupled(elems) if elems.len() == 3
            ));
        }
        _ => panic!("expected function"),
    }
    round_trip(src);
}

// ── Generic record type ──────────────────────────────────────────────

#[test]
fn generic_record_type() {
    let src = "\
module Main exposing (..)

type alias WithId a = { a | id : Int }
";
    parse_ok(src);
    round_trip(src);
}

// ── Type with many args ──────────────────────────────────────────────

#[test]
fn type_with_many_args() {
    let src = "\
module Main exposing (..)

x : Result String (List (Maybe Int))
x = Ok []
";
    let m = parse_ok(src);
    match &m.declarations[0].value {
        Declaration::FunctionDeclaration(f) => {
            let sig = f.signature.as_ref().unwrap();
            match &sig.value.type_annotation.value {
                TypeAnnotation::Typed { name, args, .. } => {
                    assert_eq!(name.value, "Result");
                    assert_eq!(args.len(), 2);
                }
                other => panic!("expected Typed, got {other:?}"),
            }
        }
        _ => panic!("expected function"),
    }
    round_trip(src);
}

// ── Function type with parens ────────────────────────────────────────

#[test]
fn function_type_with_parens() {
    let src = "\
module Main exposing (..)

x : (Int -> String) -> List Int -> List String
x = List.map
";
    parse_ok(src);
    round_trip(src);
}

// ── Let with type annotation ─────────────────────────────────────────

#[test]
fn let_with_type_annotation() {
    let src = "\
module Main exposing (..)

x =
    let
        add : Int -> Int -> Int
        add a b = a + b
    in
    add 1 2
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::LetIn { declarations, .. } => {
            assert_eq!(declarations.len(), 1);
        }
        _ => panic!("expected LetIn"),
    }
    round_trip(src);
}

// ── Let with destructuring ───────────────────────────────────────────

#[test]
fn let_with_destructuring() {
    let src = "\
module Main exposing (..)

x =
    let
        ( a, b ) = ( 1, 2 )
    in
    a + b
";
    parse_ok(src);
    round_trip(src);
}

// ── Multiline string ─────────────────────────────────────────────────

#[test]
fn multiline_string_expression() {
    let src = "module Main exposing (..)\n\nx = \"\"\"hello\nworld\"\"\"";
    let m = parse_ok(src);
    assert!(matches!(
        get_body(&m),
        Expr::Literal(Literal::MultilineString(s)) if s == "hello\nworld"
    ));
    round_trip(src);
}

// ── Custom operators in exposing ─────────────────────────────────────

#[test]
fn custom_operator_in_exposing() {
    let src = "\
module Main exposing ((|=), (|.))

x = 1
";
    parse_ok(src);
    round_trip(src);
}

// ── Port module with port declaration ────────────────────────────────

#[test]
fn port_incoming_and_outgoing() {
    let src = "\
port module Ports exposing (..)

port sendMessage : String -> Cmd msg

port messageReceiver : (String -> msg) -> Sub msg
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 2);
    assert!(matches!(&m.declarations[0].value, Declaration::PortDeclaration(_)));
    assert!(matches!(&m.declarations[1].value, Declaration::PortDeclaration(_)));
    round_trip(src);
}

// ── Infix declarations ──────────────────────────────────────────────

#[test]
fn all_infix_directions() {
    let src = "\
module Main exposing (..)

infix left 6 (+) = add
infix right 5 (::) = cons
infix non 4 (==) = eq
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 3);
    for d in &m.declarations {
        assert!(matches!(&d.value, Declaration::InfixDeclaration(_)));
    }
    round_trip(src);
}

// ── Nested lambda ────────────────────────────────────────────────────

#[test]
fn nested_lambda() {
    let src = "\
module Main exposing (..)

x = \\a -> \\b -> \\c -> a + b + c
";
    parse_ok(src);
    round_trip(src);
}

// ── Case with multiple constructor args ──────────────────────────────

#[test]
fn case_constructor_multiple_args() {
    let src = "\
module Main exposing (..)

f x =
    case x of
        Node color left key value right ->
            key
";
    let m = parse_ok(src);
    match get_body(&m) {
        Expr::CaseOf { branches, .. } => {
            match &branches[0].pattern.value {
                Pattern::Constructor { name, args, .. } => {
                    assert_eq!(name, "Node");
                    assert_eq!(args.len(), 5);
                }
                other => panic!("expected Constructor, got {other:?}"),
            }
        }
        _ => panic!("expected CaseOf"),
    }
    round_trip(src);
}

// ── Application with record ──────────────────────────────────────────

#[test]
fn application_with_record_arg() {
    let src = r#"
module Main exposing (..)

x = foo { name = "bar", age = 1 }
"#;
    parse_ok(src);
    round_trip(src);
}

// ── Backward pipe ────────────────────────────────────────────────────

#[test]
fn backward_pipe() {
    let src = "\
module Main exposing (..)

x = f <| g <| h 1
";
    parse_ok(src);
    round_trip(src);
}

// ── Composition operators ────────────────────────────────────────────

#[test]
fn composition_operators() {
    let src = "\
module Main exposing (..)

x = f >> g >> h

y = f << g << h
";
    parse_ok(src);
    round_trip(src);
}

// ── Negation in expression ───────────────────────────────────────────

#[test]
fn negation_of_expression() {
    let src = "\
module Main exposing (..)

x = -(a + b)
";
    parse_ok(src);
    round_trip(src);
}

// ── Multiple declarations of different types ─────────────────────────

#[test]
fn mixed_declarations() {
    let src = "\
module Main exposing (..)

type Msg = Go | Stop

type alias Config = { speed : Int }

port notify : String -> Cmd msg

run : Config -> Msg -> Int
run config msg =
    case msg of
        Go ->
            config.speed
        Stop ->
            0
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 4);
    assert!(matches!(&m.declarations[0].value, Declaration::CustomTypeDeclaration(_)));
    assert!(matches!(&m.declarations[1].value, Declaration::AliasDeclaration(_)));
    assert!(matches!(&m.declarations[2].value, Declaration::PortDeclaration(_)));
    assert!(matches!(&m.declarations[3].value, Declaration::FunctionDeclaration(_)));
    round_trip(src);
}

// ── Error recovery ───────────────────────────────────────────────────

#[test]
fn error_recovery_skips_bad_declaration() {
    let src = "\
module Main exposing (..)

good1 = 1

bad = @#$%

good2 = 2
";
    // parse should fail
    assert!(parse(src).is_err());

    // parse_recovering should return partial AST with the good declarations
    let (module, errors) = parse_recovering(src);
    let m = module.expect("should return partial AST");
    assert!(!errors.is_empty(), "should have errors");
    // Should have recovered at least one good declaration
    assert!(
        m.declarations.len() >= 1,
        "should have at least 1 declaration, got {}",
        m.declarations.len()
    );
}

#[test]
fn error_recovery_preserves_good_declarations() {
    let src = "\
module Main exposing (..)

add x y = x + y

broken = {{{

sub x y = x - y
";
    let (module, errors) = parse_recovering(src);
    let m = module.expect("should return partial AST");
    assert!(!errors.is_empty());

    // Should have parsed at least `add` and `sub`
    let names: Vec<&str> = m
        .declarations
        .iter()
        .filter_map(|d| match &d.value {
            Declaration::FunctionDeclaration(f) => Some(f.declaration.value.name.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"add"), "should have 'add', got {names:?}");
    assert!(names.contains(&"sub"), "should have 'sub', got {names:?}");
}

#[test]
fn error_recovery_bad_module_header() {
    let src = "not a module header at all";
    let (module, errors) = parse_recovering(src);
    assert!(module.is_none());
    assert!(!errors.is_empty());
}

// ── Comment attachment ───────────────────────────────────────────────

#[test]
fn leading_comments_via_token_extraction() {
    let src = "\
module Main exposing (..)

-- This is a helper function
add x y = x + y

-- Subtracts two numbers
sub x y = x - y
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 2);

    // Extract comments from the token stream (complete coverage).
    let (tokens, _) = Lexer::new(src).tokenize();
    let all_comments = extract_comments(&tokens);
    let associated = associate_comments(&m, &all_comments);

    assert_eq!(associated.len(), 2);
    assert_eq!(associated[0].len(), 1);
    assert!(matches!(
        &associated[0][0].value,
        elm_ast_rs::comment::Comment::Line(text) if text.contains("helper")
    ));
    assert_eq!(associated[1].len(), 1);
    assert!(matches!(
        &associated[1][0].value,
        elm_ast_rs::comment::Comment::Line(text) if text.contains("Subtracts")
    ));
}

#[test]
fn multiple_leading_comments() {
    let src = "\
module Main exposing (..)

-- Comment 1
-- Comment 2
add x y = x + y
";
    let m = parse_ok(src);
    let (tokens, _) = Lexer::new(src).tokenize();
    let all_comments = extract_comments(&tokens);
    let associated = associate_comments(&m, &all_comments);

    assert_eq!(associated[0].len(), 2);
}

#[test]
fn no_leading_comments() {
    let src = "\
module Main exposing (..)

add x y = x + y
";
    let m = parse_ok(src);
    let (tokens, _) = Lexer::new(src).tokenize();
    let all_comments = extract_comments(&tokens);
    let associated = associate_comments(&m, &all_comments);

    assert!(associated[0].is_empty());
}

// ── Builder API ──────────────────────────────────────────────────────

#[test]
fn builder_constructs_valid_module() {
    let m = builder::module(
        vec!["Main"],
        vec![
            builder::func("add", vec![builder::pvar("x"), builder::pvar("y")],
                builder::binop("+", builder::var("x"), builder::var("y"))),
            builder::custom_type(
                "Msg",
                Vec::<String>::new(),
                vec![
                    ("Increment", vec![]),
                    ("SetValue", vec![builder::tname("Int", vec![])]),
                ],
            ),
        ],
    );

    // Should be printable and re-parseable.
    let output = format!("{m}");
    assert!(output.contains("add x y"));
    assert!(output.contains("x + y"));
    assert!(output.contains("type Msg"));

    let reparsed = parse(&output).unwrap_or_else(|e| {
        eprintln!("--- output ---\n{output}\n---");
        panic!("failed to reparse builder output: {e:?}");
    });
    assert_eq!(reparsed.declarations.len(), 2);
}

#[test]
fn builder_expressions() {
    let expr = builder::if_else(
        builder::var("x"),
        builder::int(1),
        builder::int(0),
    );
    let output = format!("{}", expr.value);
    assert!(output.contains("if x then 1 else 0"));
}

#[test]
fn display_impl_for_pattern() {
    let pat = builder::pctor("Just", vec![builder::pvar("x")]);
    assert_eq!(format!("{}", pat.value), "Just x");
}

#[test]
fn display_impl_for_type() {
    let ty = builder::tfunc(
        builder::tname("Int", vec![]),
        builder::tname("String", vec![]),
    );
    assert_eq!(format!("{}", ty.value), "Int -> String");
}

// ── Serde round-trip ─────────────────────────────────────────────────

#[test]
#[cfg(feature = "serde")]
fn serde_json_round_trip() {
    let src = "\
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y
";
    let m = parse_ok(src);
    let json = serde_json::to_string(&m).expect("serialize");
    let m2: elm_ast_rs::ElmModule = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(m.declarations.len(), m2.declarations.len());
    assert_eq!(m.imports.len(), m2.imports.len());
}
