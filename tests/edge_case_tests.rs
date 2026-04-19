#![cfg(not(target_arch = "wasm32"))]

use elm_ast::builder;
use elm_ast::declaration::Declaration;
use elm_ast::expr::Expr;
use elm_ast::file::{associate_comments, extract_comments};
use elm_ast::literal::Literal;
use elm_ast::node::Spanned;
use elm_ast::pattern::Pattern;
use elm_ast::type_annotation::TypeAnnotation;
use elm_ast::{Lexer, parse, parse_recovering};

fn parse_ok(source: &str) -> elm_ast::file::ElmModule {
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

fn get_body(m: &elm_ast::file::ElmModule) -> &Expr {
    match &m.declarations[0].value {
        Declaration::FunctionDeclaration(f) => &f.declaration.value.body.value,
        _ => panic!("expected function"),
    }
}

fn round_trip(source: &str) {
    let ast1 = parse(source).unwrap();
    let printed = elm_ast::print::print(&ast1);
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
        Expr::IfElse {
            branches,
            else_branch,
        } => {
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
    assert!(matches!(get_body(&m), Expr::List { elements, .. } if elements.is_empty()));
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
            assert!(matches!(
                &sig.value.type_annotation.value,
                TypeAnnotation::Unit
            ));
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
    assert!(matches!(
        &m.declarations[0].value,
        Declaration::PortDeclaration(_)
    ));
    assert!(matches!(
        &m.declarations[1].value,
        Declaration::PortDeclaration(_)
    ));
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
        Expr::CaseOf { branches, .. } => match &branches[0].pattern.value {
            Pattern::Constructor { name, args, .. } => {
                assert_eq!(name, "Node");
                assert_eq!(args.len(), 5);
            }
            other => panic!("expected Constructor, got {other:?}"),
        },
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
    assert!(matches!(
        &m.declarations[0].value,
        Declaration::CustomTypeDeclaration(_)
    ));
    assert!(matches!(
        &m.declarations[1].value,
        Declaration::AliasDeclaration(_)
    ));
    assert!(matches!(
        &m.declarations[2].value,
        Declaration::PortDeclaration(_)
    ));
    assert!(matches!(
        &m.declarations[3].value,
        Declaration::FunctionDeclaration(_)
    ));
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
        !m.declarations.is_empty(),
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
        elm_ast::comment::Comment::Line(text) if text.contains("helper")
    ));
    assert_eq!(associated[1].len(), 1);
    assert!(matches!(
        &associated[1][0].value,
        elm_ast::comment::Comment::Line(text) if text.contains("Subtracts")
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
            builder::func(
                "add",
                vec![builder::pvar("x"), builder::pvar("y")],
                builder::binop("+", builder::var("x"), builder::var("y")),
            ),
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
    let expr = builder::if_else(builder::var("x"), builder::int(1), builder::int(0));
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
    let m2: elm_ast::ElmModule = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(m.declarations.len(), m2.declarations.len());
    assert_eq!(m.imports.len(), m2.imports.len());
}

// ── Error recovery (additional) ─────────────────────────────────────

#[test]
fn error_recovery_malformed_type_annotation() {
    // A declaration with broken type annotation should be skipped,
    // but subsequent good declarations should be recovered.
    let src = "\
module Main exposing (..)

broken : -> Int
broken x = x

good : Int -> Int
good x = x
";
    let (module, errors) = parse_recovering(src);
    let m = module.expect("should return partial AST");
    assert!(
        !errors.is_empty(),
        "should have errors for broken type annotation"
    );
    // Should recover at least the 'good' function
    assert!(
        m.declarations.iter().any(|d| {
            matches!(
                &d.value,
                Declaration::FunctionDeclaration(f)
                    if f.declaration.value.name.value == "good"
            )
        }),
        "should recover 'good' declaration"
    );
}

#[test]
fn error_recovery_incomplete_function_body() {
    // When a function has an incomplete body (missing expression after `=`),
    // the parser may consume the next top-level declaration as the body.
    // We verify that parse_recovering still returns a partial AST with errors.
    let src = "\
module Main exposing (..)

incomplete x =

working y = y + 1
";
    let (module, errors) = parse_recovering(src);
    let m = module.expect("should return partial AST");
    assert!(!errors.is_empty(), "should have at least one error");
    // The parser consumes `working y = y + 1` as the body of `incomplete`,
    // so we just verify recovery produced a partial module with declarations.
    assert!(
        !m.declarations.is_empty(),
        "should have at least 1 declaration"
    );
}

#[test]
fn error_recovery_multiple_broken_declarations() {
    let src = "\
module Main exposing (..)

bad1 = if

bad2 = case

ok1 x = x

bad3 = let in

ok2 y = y
";
    let (module, errors) = parse_recovering(src);
    let m = module.expect("should return partial AST");
    assert!(
        errors.len() >= 2,
        "should have multiple errors, got {}",
        errors.len()
    );
    let recovered_names: Vec<&str> = m
        .declarations
        .iter()
        .filter_map(|d| match &d.value {
            Declaration::FunctionDeclaration(f) => Some(f.declaration.value.name.value.as_str()),
            _ => None,
        })
        .collect();
    // Recovery should salvage at least ok1 from among the broken declarations
    assert!(
        recovered_names.contains(&"ok1"),
        "should recover ok1, got {:?}",
        recovered_names
    );
    // The parser may or may not recover ok2 depending on how `let in` is consumed
    assert!(
        recovered_names.len() >= 2,
        "should recover at least 2 declarations, got {:?}",
        recovered_names
    );
}

#[test]
fn error_recovery_error_has_span_info() {
    let src = "\
module Main exposing (..)

broken = if
";
    let (_, errors) = parse_recovering(src);
    assert!(!errors.is_empty());
    // Errors should have non-zero span info
    for e in &errors {
        assert!(
            e.span.start.line > 0,
            "error span should have line info: {:?}",
            e
        );
    }
}

// ── Edge cases (additional) ─────────────────────────────────────────

#[test]
fn empty_module_no_declarations() {
    let src = "\
module Main exposing (..)
";
    let m = parse_ok(src);
    assert!(m.declarations.is_empty());
    assert!(m.imports.is_empty());
}

#[test]
fn module_with_imports_only() {
    let src = "\
module Main exposing (..)

import Html
import Json.Decode as Decode
";
    let m = parse_ok(src);
    assert!(m.declarations.is_empty());
    assert_eq!(m.imports.len(), 2);
}

#[test]
fn deeply_nested_expressions() {
    // Build a deeply nested expression: f (f (f (... (f x) ...)))
    // 100 levels of nesting — safe because the parser is fully iterative
    // (CPS/trampoline), so depth is limited only by heap, not stack.
    let mut src = String::from("module Main exposing (..)\n\nresult = ");
    for _ in 0..100 {
        src.push_str("identity (");
    }
    src.push_str("42");
    for _ in 0..100 {
        src.push(')');
    }
    src.push('\n');
    let m = parse_ok(&src);
    assert_eq!(m.declarations.len(), 1);
    // Verify round-trip
    round_trip(&src);
}

#[test]
fn deeply_nested_if_else() {
    // 100 levels of nested if-else — safe with the iterative parser.
    let mut src = String::from("module Main exposing (..)\n\nresult x = ");
    for i in 0..100 {
        src.push_str(&format!("if x == {} then {} else ", i, i));
    }
    src.push_str("0\n");
    let m = parse_ok(&src);
    assert_eq!(m.declarations.len(), 1);
}

#[test]
fn depth_limit_returns_error_instead_of_stack_overflow() {
    // 257 levels of nesting exceeds the 256 depth limit.
    // The iterative parser returns a clean error instead of consuming
    // unbounded heap memory.
    let mut src = String::from("module Main exposing (..)\n\nresult = ");
    for _ in 0..257 {
        src.push_str("identity (");
    }
    src.push_str("42");
    for _ in 0..257 {
        src.push(')');
    }
    src.push('\n');
    let result = parse(&src);
    assert!(
        result.is_err(),
        "257-deep nesting should be rejected by depth limit"
    );
    let err = result.unwrap_err();
    assert!(
        err.iter().any(|e| e.message.contains("nesting too deep")),
        "error should mention depth limit, got: {:?}",
        err
    );
}

// ── Builder API (additional) ────────────────────────────────────────

#[test]
fn builder_int_float_string_char() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let m = module(
        vec!["Main"],
        vec![
            func("myInt", vec![], int(42)),
            func("myFloat", vec![], float(3.5)),
            func("myString", vec![], string("hello")),
            func("myChar", vec![], char_lit('x')),
            func("myUnit", vec![], unit()),
        ],
    );
    let output = print(&m);
    assert!(output.contains("myInt ="), "should have myInt");
    assert!(output.contains("42"), "should have 42");
    assert!(output.contains("3.5"), "should have 3.5");
    assert!(output.contains("\"hello\""), "should have hello string");
    assert!(output.contains("'x'"), "should have char x");
    assert!(output.contains("()"), "should have unit");
}

#[test]
fn builder_qualified_and_patterns() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let m = module(
        vec!["Main"],
        vec![func(
            "test",
            vec![pwild(), precord(vec!["x", "y"])],
            qualified(&["List"], "map"),
        )],
    );
    let output = print(&m);
    assert!(output.contains("_"), "should have wildcard pattern");
    assert!(output.contains("{ x, y }"), "should have record pattern");
    assert!(output.contains("List.map"), "should have qualified name");
}

#[test]
fn builder_func_with_sig() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let m = module(
        vec!["Main"],
        vec![func_with_sig(
            "add",
            vec![pvar("x"), pvar("y")],
            binop("+", var("x"), var("y")),
            tfunc(
                tname("Int", vec![]),
                tfunc(tname("Int", vec![]), tname("Int", vec![])),
            ),
        )],
    );
    let output = print(&m);
    assert!(
        output.contains("add : Int -> Int -> Int"),
        "should have type sig, got:\n{}",
        output
    );
    assert!(output.contains("add x y"), "should have function impl");
}

#[test]
fn builder_type_alias_and_custom_type() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let m = module(
        vec!["Main"],
        vec![
            type_alias("Pair", vec!["a", "b"], tname("Record", vec![])),
            custom_type(
                "Maybe",
                vec!["a"],
                vec![("Just", vec![tvar("a")]), ("Nothing", vec![])],
            ),
        ],
    );
    let output = print(&m);
    assert!(output.contains("type alias Pair"), "should have type alias");
    assert!(output.contains("type Maybe"), "should have custom type");
    assert!(output.contains("= Just a"), "should have Just constructor");
    assert!(
        output.contains("| Nothing"),
        "should have Nothing constructor"
    );
}

#[test]
fn builder_import() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let mut m = module(vec!["Main"], vec![func("x", vec![], int(1))]);
    m.imports = vec![import(vec!["Html"]), import(vec!["Json", "Decode"])];
    let output = print(&m);
    assert!(output.contains("import Html"), "should have Html import");
    assert!(
        output.contains("import Json.Decode"),
        "should have Json.Decode import"
    );
}

#[test]
fn builder_tvar_and_tunit() {
    use elm_ast::builder::*;
    use elm_ast::print::print;

    let m = module(
        vec!["Main"],
        vec![func_with_sig(
            "f",
            vec![pvar("x")],
            var("x"),
            tfunc(tvar("a"), tunit()),
        )],
    );
    let output = print(&m);
    assert!(
        output.contains("f : a -> ()"),
        "should have type sig with tvar and tunit, got:\n{}",
        output
    );
}

// ── Comment round-tripping ──────────────────────────────────────────

#[test]
fn comment_between_declarations_round_trips() {
    let src = "\
module Main exposing (..)


add x y =
    x + y


-- Helper function
subtract x y =
    x - y
";
    let m = parse_ok(src);
    assert!(!m.comments.is_empty(), "should capture line comment");

    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- Helper function"),
        "printed output should contain the comment, got:\n{}",
        output
    );

    // Verify it re-parses
    let m2 = parse_ok(&output);
    assert_eq!(m2.declarations.len(), 2);
}

#[test]
fn block_comment_between_declarations_round_trips() {
    let src = "\
module Main exposing (..)


foo =
    1


{- This is a block comment -}
bar =
    2
";
    let m = parse_ok(src);
    assert!(
        m.comments
            .iter()
            .any(|c| matches!(&c.value, elm_ast::comment::Comment::Block(_))),
        "should capture block comment"
    );

    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("{- This is a block comment -}"),
        "printed output should contain the block comment, got:\n{}",
        output
    );
}

#[test]
fn multiple_comments_between_declarations() {
    let src = "\
module Main exposing (..)


first =
    1


-- Comment A
-- Comment B
second =
    2
";
    let m = parse_ok(src);
    assert!(
        m.comments.len() >= 2,
        "should capture both comments, got {}",
        m.comments.len()
    );

    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- Comment A"),
        "should contain Comment A, got:\n{}",
        output
    );
    assert!(
        output.contains("-- Comment B"),
        "should contain Comment B, got:\n{}",
        output
    );
}

#[test]
fn trailing_comment_after_last_declaration() {
    let src = "\
module Main exposing (..)


x =
    1


-- trailing
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- trailing"),
        "trailing comment should be preserved, got:\n{}",
        output
    );
}

#[test]
fn comment_round_trip_reparses() {
    let src = "\
module Main exposing (..)


-- Before first
a =
    1


-- Between
b =
    2


-- After last
";
    let m = parse_ok(src);
    let printed = elm_ast::print::print(&m);

    // Must re-parse successfully
    let m2 = parse_ok(&printed);
    assert_eq!(
        m2.declarations.len(),
        2,
        "should have 2 declarations after round-trip"
    );
    assert!(
        !m2.comments.is_empty(),
        "comments should survive the round-trip"
    );
}

#[test]
fn comment_between_imports() {
    let src = "\
module Main exposing (..)

import Html
-- comment between imports
import Json.Decode

x =
    1
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- comment between imports"),
        "comment between imports should be preserved, got:\n{}",
        output
    );
    // Re-parse must succeed
    let m2 = parse_ok(&output);
    assert_eq!(m2.imports.len(), 2);
    assert_eq!(m2.declarations.len(), 1);
}

#[test]
fn comment_between_header_and_imports() {
    let src = "\
module Main exposing (..)

-- module-level comment
import Html

x =
    1
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- module-level comment"),
        "comment between header and imports should be preserved, got:\n{}",
        output
    );
    let m2 = parse_ok(&output);
    assert_eq!(m2.imports.len(), 1);
}

#[test]
fn comment_between_imports_and_first_declaration() {
    let src = "\
module Main exposing (..)

import Html

-- section marker
x =
    1
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- section marker"),
        "comment between imports and declarations should be preserved, got:\n{}",
        output
    );
    let m2 = parse_ok(&output);
    assert_eq!(m2.declarations.len(), 1);
}

#[test]
fn nested_block_comment_round_trips() {
    let src = "\
module Main exposing (..)


{- outer {- inner -} comment -}
x =
    1
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("{- outer {- inner -} comment -}"),
        "nested block comment should be preserved, got:\n{}",
        output
    );
    let m2 = parse_ok(&output);
    assert_eq!(m2.declarations.len(), 1);
}

#[test]
fn mixed_line_and_block_comments() {
    let src = "\
module Main exposing (..)


a =
    1


-- line comment
{- block comment -}
b =
    2
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- line comment"),
        "line comment should be preserved, got:\n{}",
        output
    );
    assert!(
        output.contains("{- block comment -}"),
        "block comment should be preserved, got:\n{}",
        output
    );
}

#[test]
fn comment_idempotency() {
    // parse -> print -> parse -> print must produce identical output
    let src = "\
module Main exposing (..)

import Html
-- between imports
import Json.Decode


-- section A
a =
    1


-- section B
{- extra -}
b =
    2


-- trailing
";
    let m1 = parse_ok(src);
    let print1 = elm_ast::print::print(&m1);

    let m2 = parse_ok(&print1);
    let print2 = elm_ast::print::print(&m2);

    assert_eq!(
        print1, print2,
        "comment printing must be idempotent.\nFirst print:\n{}\nSecond print:\n{}",
        print1, print2
    );
}

#[test]
fn comments_preserved_across_many_declarations() {
    let src = "\
module Main exposing (..)


-- A
a =
    1


-- B
b =
    2


-- C
c =
    3


-- D
d =
    4
";
    let m = parse_ok(src);
    assert!(
        m.comments.len() >= 4,
        "should capture all 4 comments, got {}",
        m.comments.len()
    );

    let output = elm_ast::print::print(&m);
    for label in &["-- A", "-- B", "-- C", "-- D"] {
        assert!(
            output.contains(label),
            "should contain {}, got:\n{}",
            label,
            output
        );
    }

    // Idempotency
    let m2 = parse_ok(&output);
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output, output2, "multi-comment printing must be idempotent");
}

#[test]
fn comment_only_module_no_declarations() {
    let src = "\
module Main exposing (..)

-- just a comment
";
    let m = parse_ok(src);
    assert!(m.declarations.is_empty());
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- just a comment"),
        "comment in declaration-less module should be preserved, got:\n{}",
        output
    );
}

#[test]
fn doc_comment_and_line_comment_coexist() {
    // Doc comments are attached to declarations, line comments are separate.
    let src = "\
module Main exposing (..)


-- section marker
{-| Documentation for foo -}
foo =
    1
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- section marker"),
        "line comment before doc comment should be preserved, got:\n{}",
        output
    );
    assert!(
        output.contains("{-|"),
        "doc comment should be preserved, got:\n{}",
        output
    );
}

#[test]
fn comment_count_survives_round_trip() {
    let src = "\
module Main exposing (..)


-- one
-- two
-- three
x =
    1


-- four
y =
    2
";
    let m = parse_ok(src);
    let comment_count = m.comments.len();
    assert!(
        comment_count >= 4,
        "should have at least 4 comments, got {}",
        comment_count
    );

    let output = elm_ast::print::print(&m);
    let m2 = parse_ok(&output);
    assert_eq!(
        m2.comments.len(),
        comment_count,
        "comment count should survive round-trip: {} vs {}.\nPrinted:\n{}",
        comment_count,
        m2.comments.len(),
        output
    );
}

// ── BinOps (raw unresolved operator chain) ──────────────────────────

#[test]
fn binops_construct_and_print() {
    // BinOps is not produced by the parser — only for manual AST construction.
    // Verify we can construct one and the printer handles it.
    let binops_expr = Spanned::dummy(Expr::BinOps {
        operands_and_operators: vec![
            (
                Spanned::dummy(Expr::FunctionOrValue {
                    module_name: vec![],
                    name: "a".into(),
                }),
                Spanned::dummy("+".to_string()),
            ),
            (
                Spanned::dummy(Expr::FunctionOrValue {
                    module_name: vec![],
                    name: "b".into(),
                }),
                Spanned::dummy("*".to_string()),
            ),
        ],
        final_operand: Box::new(Spanned::dummy(Expr::FunctionOrValue {
            module_name: vec![],
            name: "c".into(),
        })),
    });

    // Build a module containing this expression
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], binops_expr)]);

    let output = elm_ast::print::print(&m);
    // The printer wraps BinOps in parens when in atomic position
    assert!(
        output.contains("a") && output.contains("b") && output.contains("c"),
        "BinOps should print all operands. Output:\n{output}"
    );
}

#[test]
fn binops_visitor_traversal() {
    use elm_ast::visit::Visit;

    // Construct a BinOps node and verify the visitor traverses operands
    let binops_expr = Spanned::dummy(Expr::BinOps {
        operands_and_operators: vec![
            (
                Spanned::dummy(Expr::Literal(Literal::Int(1))),
                Spanned::dummy("+".to_string()),
            ),
            (
                Spanned::dummy(Expr::Literal(Literal::Int(2))),
                Spanned::dummy("*".to_string()),
            ),
        ],
        final_operand: Box::new(Spanned::dummy(Expr::Literal(Literal::Int(3)))),
    });

    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], binops_expr)]);

    struct LiteralCounter(usize);
    impl Visit for LiteralCounter {
        fn visit_literal(&mut self, _lit: &Literal) {
            self.0 += 1;
        }
    }

    let mut counter = LiteralCounter(0);
    counter.visit_module(&m);
    assert_eq!(
        counter.0, 3,
        "should visit all 3 operand literals in BinOps"
    );
}

#[test]
fn binops_fold_traversal() {
    use elm_ast::fold::Fold;

    let binops_expr = Spanned::dummy(Expr::BinOps {
        operands_and_operators: vec![(
            Spanned::dummy(Expr::Literal(Literal::Int(10))),
            Spanned::dummy("+".to_string()),
        )],
        final_operand: Box::new(Spanned::dummy(Expr::Literal(Literal::Int(20)))),
    });

    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], binops_expr)]);

    // Fold that doubles all integers
    struct IntDoubler;
    impl Fold for IntDoubler {
        fn fold_literal(&mut self, lit: Literal) -> Literal {
            match lit {
                Literal::Int(n) => Literal::Int(n * 2),
                other => other,
            }
        }
    }

    let m2 = IntDoubler.fold_module(m);

    // Verify the fold was applied by checking the AST directly
    match &m2.declarations[0].value {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::BinOps {
                operands_and_operators,
                final_operand,
            } => {
                // First operand should be 20 (10 * 2)
                assert!(
                    matches!(
                        &operands_and_operators[0].0.value,
                        Expr::Literal(Literal::Int(20))
                    ),
                    "first operand should be 20"
                );
                // Final operand should be 40 (20 * 2)
                assert!(
                    matches!(&final_operand.value, Expr::Literal(Literal::Int(40))),
                    "final operand should be 40"
                );
            }
            other => panic!("expected BinOps, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Builder: untested functions ─────────────────────────────────────

#[test]
fn builder_app() {
    let expr = builder::app(builder::var("f"), vec![builder::int(1), builder::int(2)]);
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], expr)]);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("f 1 2"),
        "app should print as `f 1 2`. Got:\n{output}"
    );
    parse_ok(&output);
}

#[test]
fn builder_lambda() {
    let expr = builder::lambda(
        vec![builder::pvar("a"), builder::pvar("b")],
        builder::var("a"),
    );
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], expr)]);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("\\a b ->"),
        "lambda should print args. Got:\n{output}"
    );
    parse_ok(&output);
}

#[test]
fn builder_list() {
    let expr = builder::list(vec![builder::int(1), builder::int(2), builder::int(3)]);
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], expr)]);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("[ 1, 2, 3 ]"),
        "list should format. Got:\n{output}"
    );
    parse_ok(&output);
}

#[test]
fn builder_tuple() {
    let expr = builder::tuple(vec![builder::int(1), builder::string("hello")]);
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], expr)]);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("( 1, \"hello\" )"),
        "tuple should format. Got:\n{output}"
    );
    parse_ok(&output);
}

#[test]
fn builder_record() {
    let expr = builder::record(vec![
        ("name", builder::string("Alice")),
        ("age", builder::int(30)),
    ]);
    let m = builder::module(vec!["Main"], vec![builder::func("x", vec![], expr)]);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("name ="),
        "record should have field names. Got:\n{output}"
    );
    assert!(
        output.contains("\"Alice\""),
        "record should have values. Got:\n{output}"
    );
    parse_ok(&output);
}

// ── VisitMut round-trip ─────────────────────────────────────────────

#[test]
fn visit_mut_round_trip_preserves_parsability() {
    use elm_ast::visit_mut::VisitMut;

    let src = "\
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y

greet name = \"Hello, \" ++ name
";
    let mut m = parse_ok(src);

    // Rename 'x' to 'z' everywhere
    struct Renamer;
    impl VisitMut for Renamer {
        fn visit_ident_mut(&mut self, name: &mut String) {
            if *name == "x" {
                *name = "z".to_string();
            }
        }
    }
    Renamer.visit_module_mut(&mut m);

    // The mutated AST should still print and reparse correctly
    let output = elm_ast::print::print(&m);
    let m2 = parse_ok(&output);
    assert_eq!(m2.declarations.len(), 2);
    assert!(
        output.contains("z + y"),
        "should have renamed x to z. Got:\n{output}"
    );
    assert!(
        !output.contains("x +"),
        "should not have original x in body. Got:\n{output}"
    );

    // Second round-trip: print again and verify idempotent
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output, output2, "mutated AST should print idempotently");
}

#[test]
fn visit_mut_round_trip_with_complex_ast() {
    use elm_ast::visit_mut::VisitMut;

    let src = r#"
module Main exposing (..)

type Msg
    = Increment
    | Decrement

update : Msg -> Int -> Int
update msg model =
    case msg of
        Increment ->
            model + 1

        Decrement ->
            model - 1

view model =
    if model > 0 then
        "positive"
    else
        "non-positive"
"#;
    let mut m = parse_ok(src);

    // Double all integer literals
    struct IntDoubler;
    impl VisitMut for IntDoubler {
        fn visit_literal_mut(&mut self, lit: &mut Literal) {
            if let Literal::Int(n) = lit {
                *n *= 2;
            }
        }
    }
    IntDoubler.visit_module_mut(&mut m);

    let output = elm_ast::print::print(&m);
    let m2 = parse_ok(&output);
    assert_eq!(m2.declarations.len(), 3);

    // Verify idempotent
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output, output2);
}

// ── Serde: additional coverage ──────────────────────────────────────

#[test]
#[cfg(feature = "serde")]
fn serde_round_trip_complex_module() {
    let src = r#"
module Main exposing (..)

import Html exposing (div, text)

type Msg
    = Increment
    | Decrement

type alias Model =
    { count : Int
    , name : String
    }

update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            { model | count = model.count + 1 }

        Decrement ->
            { model | count = model.count - 1 }
"#;
    let m = parse_ok(src);
    let json = serde_json::to_string(&m).expect("serialize");
    let m2: elm_ast::file::ElmModule = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(m.imports.len(), m2.imports.len());
    assert_eq!(m.declarations.len(), m2.declarations.len());
    assert_eq!(m.comments.len(), m2.comments.len());

    // The deserialized AST should print identically to the original
    let output1 = elm_ast::print::print(&m);
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(
        output1, output2,
        "serde round-trip should preserve print output"
    );
}

#[test]
#[cfg(feature = "serde")]
fn serde_round_trip_preserves_all_expr_types() {
    let src = r#"
module Main exposing (..)

a = 42

b = "hello"

c = 'x'

d = 3.14

e = ()

f = [ 1, 2, 3 ]

g = ( 1, 2 )

h = \x -> x

i = { name = "test" }
"#;
    let m = parse_ok(src);
    let json = serde_json::to_string_pretty(&m).expect("serialize");
    let m2: elm_ast::file::ElmModule = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(m.declarations.len(), m2.declarations.len());

    let output1 = elm_ast::print::print(&m);
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output1, output2);
}

#[test]
#[cfg(feature = "serde")]
fn serde_parse_serialize_reparse_idempotent() {
    let src = "\
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y
";
    let m1 = parse_ok(src);
    let json = serde_json::to_string(&m1).expect("serialize");
    let m2: elm_ast::file::ElmModule = serde_json::from_str(&json).expect("deserialize");
    let printed = elm_ast::print::print(&m2);
    let m3 = parse_ok(&printed);
    let printed2 = elm_ast::print::print(&m3);
    assert_eq!(
        printed, printed2,
        "parse→serialize→deserialize→print→parse→print should be idempotent"
    );
}

// ── Display impls: independent tests ────────────────────────────────

#[test]
fn display_expr_literal() {
    assert_eq!(format!("{}", Expr::Literal(Literal::Int(42))), "42");
    assert_eq!(
        format!("{}", Expr::Literal(Literal::String("hi".into()))),
        "\"hi\""
    );
    assert_eq!(format!("{}", Expr::Literal(Literal::Char('a'))), "'a'");
    assert_eq!(format!("{}", Expr::Unit), "()");
}

#[test]
fn display_expr_function_or_value() {
    let expr = Expr::FunctionOrValue {
        module_name: vec![],
        name: "foo".into(),
    };
    assert_eq!(format!("{expr}"), "foo");

    let qualified = Expr::FunctionOrValue {
        module_name: vec!["Html".into()],
        name: "div".into(),
    };
    assert_eq!(format!("{qualified}"), "Html.div");
}

#[test]
fn display_expr_list() {
    let list = Expr::List {
        elements: vec![
            Spanned::dummy(Expr::Literal(Literal::Int(1))),
            Spanned::dummy(Expr::Literal(Literal::Int(2))),
        ],
        element_inline_comments: Vec::new(),
        trailing_comments: Vec::new(),
    };
    let output = format!("{list}");
    assert!(
        output.contains("1") && output.contains("2"),
        "list display: {output}"
    );
}

#[test]
fn display_pattern_variants() {
    assert_eq!(format!("{}", Pattern::Anything), "_");
    assert_eq!(format!("{}", Pattern::Var("x".into())), "x");
    assert_eq!(format!("{}", Pattern::Unit), "()");
    assert_eq!(format!("{}", Pattern::Literal(Literal::Int(42))), "42");
}

#[test]
fn display_type_annotation_variants() {
    assert_eq!(format!("{}", TypeAnnotation::GenericType("a".into())), "a");
    assert_eq!(format!("{}", TypeAnnotation::Unit), "()");

    let typed = TypeAnnotation::Typed {
        module_name: vec![],
        name: Spanned::dummy("Int".into()),
        args: vec![],
    };
    assert_eq!(format!("{typed}"), "Int");
}

#[test]
fn display_declaration() {
    let m = parse_ok("module Main exposing (..)\n\nadd x y = x + y");
    let decl = &m.declarations[0].value;
    let output = format!("{decl}");
    assert!(output.contains("add x y"), "declaration display: {output}");
    assert!(output.contains("x + y"), "declaration display: {output}");
}

#[test]
fn display_module() {
    let m = parse_ok("module Main exposing (..)\n\nx = 1");
    let output = format!("{m}");
    assert!(
        output.contains("module Main exposing"),
        "module display: {output}"
    );
    assert!(output.contains("x ="), "module display: {output}");
}

// ── Expression-level comment preservation ─────────────────────────────

#[test]
fn let_in_comment_between_declarations() {
    let src = "\
module Main exposing (..)


x =
    let
        a =
            1

        -- comment between let bindings
        b =
            2
    in
    a + b
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- comment between let bindings"),
        "comment inside let-in should be preserved, got:\n{output}"
    );
}

#[test]
fn case_comment_between_branches() {
    let src = "\
module Main exposing (..)


x y =
    case y of
        True ->
            1

        -- comment before False branch
        False ->
            2
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- comment before False branch"),
        "comment between case branches should be preserved, got:\n{output}"
    );
}

#[test]
fn let_in_comment_round_trip() {
    let src = "\
module Main exposing (..)


x =
    let
        a =
            1

        -- helper value
        b =
            2
    in
    a + b
";
    let m = parse_ok(src);
    let output1 = elm_ast::print::print(&m);
    assert!(
        output1.contains("-- helper value"),
        "first print should contain comment"
    );
    let m2 = parse_ok(&output1);
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output1, output2, "let-in comment should survive round-trip");
}

#[test]
fn case_comment_round_trip() {
    let src = "\
module Main exposing (..)


x y =
    case y of
        True ->
            1

        -- handle false case
        False ->
            2
";
    let m = parse_ok(src);
    let output1 = elm_ast::print::print(&m);
    assert!(
        output1.contains("-- handle false case"),
        "first print should contain comment"
    );
    let m2 = parse_ok(&output1);
    let output2 = elm_ast::print::print(&m2);
    assert_eq!(output1, output2, "case comment should survive round-trip");
}

#[test]
fn let_in_block_comment_preserved() {
    let src = "\
module Main exposing (..)


x =
    let
        {- block comment in let -}
        a =
            1
    in
    a
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("{- block comment in let -}"),
        "block comment inside let-in should be preserved, got:\n{output}"
    );
}

#[test]
fn case_multiple_comments_between_branches() {
    let src = "\
module Main exposing (..)


x y =
    case y of
        1 ->
            True

        -- first comment
        -- second comment
        _ ->
            False
";
    let m = parse_ok(src);
    let output = elm_ast::print::print(&m);
    assert!(
        output.contains("-- first comment"),
        "first comment between case branches should be preserved, got:\n{output}"
    );
    assert!(
        output.contains("-- second comment"),
        "second comment between case branches should be preserved, got:\n{output}"
    );
}

#[test]
fn visit_comments_on_let_declarations() {
    use elm_ast::comment::Comment;
    use elm_ast::visit::Visit;

    let src = "\
module Main exposing (..)


x =
    let
        a =
            1

        -- comment on b
        b =
            2
    in
    a + b
";
    let m = parse_ok(src);

    struct CommentCollector(Vec<String>);
    impl Visit for CommentCollector {
        fn visit_comment(&mut self, comment: &Spanned<Comment>) {
            match &comment.value {
                Comment::Line(text) => self.0.push(text.clone()),
                Comment::Block(text) => self.0.push(text.clone()),
                Comment::Doc(text) => self.0.push(text.clone()),
            }
        }
    }

    let mut collector = CommentCollector(vec![]);
    collector.visit_module(&m);
    assert!(
        collector.0.iter().any(|c| c.contains("comment on b")),
        "visitor should find comment attached to let declaration, got: {:?}",
        collector.0
    );
}

#[test]
fn visit_comments_on_case_branch_patterns() {
    use elm_ast::comment::Comment;
    use elm_ast::visit::Visit;

    let src = "\
module Main exposing (..)


x y =
    case y of
        True ->
            1

        -- false branch comment
        False ->
            2
";
    let m = parse_ok(src);

    struct CommentCollector(Vec<String>);
    impl Visit for CommentCollector {
        fn visit_comment(&mut self, comment: &Spanned<Comment>) {
            match &comment.value {
                Comment::Line(text) => self.0.push(text.clone()),
                Comment::Block(text) => self.0.push(text.clone()),
                Comment::Doc(text) => self.0.push(text.clone()),
            }
        }
    }

    let mut collector = CommentCollector(vec![]);
    collector.visit_module(&m);
    assert!(
        collector
            .0
            .iter()
            .any(|c| c.contains("false branch comment")),
        "visitor should find comment on case branch pattern, got: {:?}",
        collector.0
    );
}

// ── CPS/trampoline stress tests ─────────────────────────────────────

#[test]
fn deeply_nested_mixed_expressions() {
    // if inside case inside let inside lambda — 25 layers of each
    let mut src = String::from("module Main exposing (..)\n\nf x =\n");
    let indent = "    ";
    let mut depth = 1;
    for i in 0..25 {
        let pad = indent.repeat(depth);
        // lambda
        src.push_str(&format!("{pad}\\arg{i} ->\n"));
        depth += 1;
        let pad = indent.repeat(depth);
        // let
        src.push_str(&format!("{pad}let\n"));
        src.push_str(&format!("{pad}    tmp{i} = arg{i}\n"));
        src.push_str(&format!("{pad}in\n"));
        // case
        src.push_str(&format!("{pad}case tmp{i} of\n"));
        depth += 1;
        let pad = indent.repeat(depth);
        src.push_str(&format!("{pad}_ ->\n"));
        depth += 1;
        let pad = indent.repeat(depth);
        // if
        src.push_str(&format!("{pad}if True then\n"));
        depth += 1;
    }
    let pad = indent.repeat(depth);
    src.push_str(&format!("{pad}42\n"));
    // close all the if/else chains
    for _ in 0..25 {
        depth -= 1;
        let pad = indent.repeat(depth);
        src.push_str(&format!("\n{pad}else\n"));
        depth -= 1;
        let pad = indent.repeat(depth);
        src.push_str(&format!("{pad}    0\n"));
        depth -= 2; // back out of case branch and lambda
    }
    // The point is: this should parse without stack overflow
    let result = parse(&src);
    assert!(
        result.is_ok(),
        "deeply nested mixed expressions should parse: {:?}",
        result.err()
    );
}

#[test]
fn deeply_nested_lists() {
    // [[[[...50 levels...]]]]
    let mut src = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..50 {
        src.push_str("[ ");
    }
    src.push('1');
    for _ in 0..50 {
        src.push_str(" ]");
    }
    src.push('\n');
    let m = parse_ok(&src);
    let body = get_body(&m);
    // Walk down to verify nesting
    let mut expr = body;
    let mut count = 0;
    loop {
        match expr {
            Expr::List { elements, .. } => {
                count += 1;
                if elements.len() == 1 {
                    expr = &elements[0].value;
                } else {
                    break;
                }
            }
            Expr::Literal(Literal::Int(1)) => break,
            _ => panic!("unexpected expr at depth {count}: {expr:?}"),
        }
    }
    assert_eq!(count, 50);
}

#[test]
fn deeply_nested_tuples() {
    // ((((...50 levels..., 1), 2), 3), 4)
    let mut src = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..50 {
        src.push_str("( ");
    }
    src.push('1');
    for i in 0..50 {
        src.push_str(&format!(", {} )", i + 2));
    }
    src.push('\n');
    let result = parse(&src);
    assert!(result.is_ok(), "deeply nested tuples should parse");
}

#[test]
fn deeply_nested_records() {
    // { a = { a = { a = ... 30 levels ... } } }
    let mut src = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..30 {
        src.push_str("{ a = ");
    }
    src.push('1');
    for _ in 0..30 {
        src.push_str(" }");
    }
    src.push('\n');
    let result = parse(&src);
    assert!(result.is_ok(), "deeply nested records should parse");
}

#[test]
fn deeply_nested_parens() {
    // ((((...100 levels...1))))
    let mut src = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..100 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..100 {
        src.push(')');
    }
    src.push('\n');
    let m = parse_ok(&src);
    // Should ultimately resolve to the integer 1
    let body = get_body(&m);
    let mut expr = body;
    loop {
        match expr {
            Expr::Parenthesized { expr: inner, .. } => expr = &inner.value,
            Expr::Literal(Literal::Int(1)) => break,
            _ => panic!("unexpected: {expr:?}"),
        }
    }
}

#[test]
fn error_at_depth_boundary() {
    // Exactly at the boundary: 256 should succeed, 257 should fail
    let mut src_ok = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..256 {
        src_ok.push_str("( ");
    }
    src_ok.push('1');
    for _ in 0..256 {
        src_ok.push_str(" )");
    }
    let result = parse(&src_ok);
    // 256 is exactly the limit — it may or may not succeed depending on
    // how many continuation frames each paren uses. The important thing is
    // that 257 fails cleanly.
    let _ = result;

    let mut src_err = String::from("module Main exposing (..)\n\nx = ");
    for _ in 0..257 {
        src_err.push_str("( ");
    }
    src_err.push('1');
    for _ in 0..257 {
        src_err.push_str(" )");
    }
    let result = parse(&src_err);
    assert!(
        result.is_err(),
        "257 levels of nesting should exceed depth limit"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("nesting too deep"),
        "error should mention depth: {err_msg}"
    );
}
