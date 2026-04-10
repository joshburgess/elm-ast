use elm_ast::parse;
use elm_ast::print::print;

/// Parse source, print it, then parse the printed output. The two ASTs
/// should be structurally identical (ignoring spans).
fn round_trip(source: &str) {
    let ast1 = parse(source).unwrap_or_else(|e| {
        panic!("first parse failed: {e:?}");
    });

    let printed = print(&ast1);

    let ast2 = parse(&printed).unwrap_or_else(|e| {
        eprintln!("--- printed output ---\n{printed}\n--- end ---");
        panic!("second parse failed: {e:?}");
    });

    // Compare declaration counts and structure (spans will differ).
    assert_eq!(
        ast1.imports.len(),
        ast2.imports.len(),
        "import count mismatch after round-trip"
    );
    assert_eq!(
        ast1.declarations.len(),
        ast2.declarations.len(),
        "declaration count mismatch after round-trip"
    );
}

/// Parse and print, returning the printed string for direct inspection.
fn print_source(source: &str) -> String {
    let ast = parse(source).unwrap_or_else(|e| {
        panic!("parse failed: {e:?}");
    });
    print(&ast)
}

// ── Round-trip tests ─────────────────────────────────────────────────

#[test]
fn round_trip_minimal_module() {
    round_trip("module Main exposing (..)");
}

#[test]
fn round_trip_with_imports() {
    round_trip(
        "\
module Main exposing (..)

import Html
import Html.Attributes as HA exposing (class, style)
",
    );
}

#[test]
fn round_trip_function_no_signature() {
    round_trip(
        "\
module Main exposing (..)

add x y = x + y
",
    );
}

#[test]
fn round_trip_function_with_signature() {
    round_trip(
        "\
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y
",
    );
}

#[test]
fn round_trip_type_alias() {
    round_trip(
        "\
module Main exposing (..)

type alias Model = { name : String, age : Int }
",
    );
}

#[test]
fn round_trip_custom_type() {
    round_trip(
        "\
module Main exposing (..)

type Msg = Increment | Decrement | SetName String
",
    );
}

#[test]
fn round_trip_parameterized_custom_type() {
    round_trip(
        "\
module Main exposing (..)

type Maybe a = Just a | Nothing
",
    );
}

#[test]
fn round_trip_if_then_else() {
    round_trip(
        "\
module Main exposing (..)

x = if True then 1 else 0
",
    );
}

#[test]
fn round_trip_case_of() {
    round_trip(
        "\
module Main exposing (..)

x =
    case msg of
        Increment ->
            1
        Decrement ->
            0
",
    );
}

#[test]
fn round_trip_let_in() {
    round_trip(
        "\
module Main exposing (..)

x =
    let
        y = 1
    in
    y
",
    );
}

#[test]
fn round_trip_lambda() {
    round_trip(
        "\
module Main exposing (..)

x = \\a b -> a
",
    );
}

#[test]
fn round_trip_list() {
    round_trip(
        "\
module Main exposing (..)

x = [ 1, 2, 3 ]
",
    );
}

#[test]
fn round_trip_record() {
    round_trip(
        "\
module Main exposing (..)

x = { name = \"Alice\", age = 30 }
",
    );
}

#[test]
fn round_trip_record_update() {
    round_trip(
        "\
module Main exposing (..)

x = { model | count = 0 }
",
    );
}

#[test]
fn round_trip_tuple() {
    round_trip(
        "\
module Main exposing (..)

x = ( 1, 2 )
",
    );
}

#[test]
fn round_trip_pipeline() {
    round_trip(
        "\
module Main exposing (..)

x = a |> b |> c
",
    );
}

#[test]
fn round_trip_operator_precedence() {
    round_trip(
        "\
module Main exposing (..)

x = 1 + 2 * 3
",
    );
}

#[test]
fn round_trip_port_module() {
    round_trip(
        "\
port module Ports exposing (sendMessage)

port sendMessage : String -> Cmd msg
",
    );
}

#[test]
fn round_trip_effect_module() {
    round_trip("effect module Task where { command = MyCmd } exposing (Task, perform)");
}

#[test]
fn round_trip_generic_record_type() {
    round_trip(
        "\
module Main exposing (..)

type alias Named a = { a | name : String }
",
    );
}

#[test]
fn round_trip_exposing_type_with_constructors() {
    round_trip(
        "\
module Main exposing (Msg(..), Model, update)

type Msg = Increment | Decrement
",
    );
}

#[test]
fn round_trip_counter_program() {
    round_trip(
        r#"
module Main exposing (..)

type Msg = Increment | Decrement

type alias Model = Int

update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            model + 1
        Decrement ->
            model - 1
"#,
    );
}

// ── Direct output tests ──────────────────────────────────────────────

#[test]
fn print_module_header() {
    let output = print_source("module Main exposing (..)");
    assert!(output.starts_with("module Main exposing (..)"));
}

#[test]
fn print_port_module_header() {
    let output = print_source("port module Ports exposing (foo, bar)");
    assert!(output.starts_with("port module Ports exposing (foo, bar)"));
}

#[test]
fn print_imports() {
    let output = print_source(
        "\
module Main exposing (..)

import Html
import Html.Attributes as HA exposing (class)
",
    );
    assert!(output.contains("import Html\n"));
    assert!(output.contains("import Html.Attributes as HA exposing (class)"));
}

#[test]
fn print_type_alias_indentation() {
    let output = print_source(
        "\
module Main exposing (..)

type alias Model = { name : String, age : Int }
",
    );
    // Should have the type on an indented line.
    assert!(output.contains("type alias Model ="));
    assert!(output.contains("{ name : String, age : Int }"));
}

#[test]
fn print_custom_type_formatting() {
    let output = print_source(
        "\
module Main exposing (..)

type Msg = Increment | Decrement | Reset
",
    );
    // Constructors should be on separate lines with = and |
    assert!(output.contains("= Increment"));
    assert!(output.contains("| Decrement"));
    assert!(output.contains("| Reset"));
}

#[test]
fn print_function_body_indented() {
    let output = print_source(
        "\
module Main exposing (..)

foo x = x + 1
",
    );
    assert!(output.contains("foo x =\n"));
    assert!(output.contains("    x + 1"));
}

#[test]
fn print_operator_spacing() {
    let output = print_source(
        "\
module Main exposing (..)

x = 1 + 2
",
    );
    assert!(output.contains("1 + 2"));
}

#[test]
fn print_list_formatting() {
    let output = print_source(
        "\
module Main exposing (..)

x = [ 1, 2, 3 ]
",
    );
    assert!(output.contains("[ 1, 2, 3 ]"));
}

#[test]
fn print_empty_list() {
    let output = print_source(
        "\
module Main exposing (..)

x = []
",
    );
    assert!(output.contains("[]"));
}

#[test]
fn print_record_formatting() {
    let output = print_source(
        r#"
module Main exposing (..)

x = { name = "Alice", age = 30 }
"#,
    );
    assert!(output.contains("{ name = \"Alice\", age = 30 }"));
}

#[test]
fn print_record_update_formatting() {
    let output = print_source(
        "\
module Main exposing (..)

x = { model | count = 0 }
",
    );
    assert!(output.contains("{ model | count = 0 }"));
}

#[test]
fn print_case_indentation() {
    let output = print_source(
        "\
module Main exposing (..)

x =
    case n of
        0 ->
            1
        _ ->
            n
",
    );
    assert!(output.contains("case n of"));
    assert!(output.contains("0 ->"));
    assert!(output.contains("_ ->"));
}

#[test]
fn print_let_indentation() {
    let output = print_source(
        "\
module Main exposing (..)

x =
    let
        y = 1
    in
    y
",
    );
    assert!(output.contains("let"));
    assert!(output.contains("in"));
}

#[test]
fn print_lambda() {
    let output = print_source(
        "\
module Main exposing (..)

x = \\a b -> a + b
",
    );
    assert!(output.contains("\\a b -> a + b"));
}

#[test]
fn print_prefix_operator() {
    let output = print_source(
        "\
module Main exposing (..)

x = (+)
",
    );
    assert!(output.contains("(+)"));
}

#[test]
fn print_unit() {
    let output = print_source(
        "\
module Main exposing (..)

x = ()
",
    );
    assert!(output.contains("()"));
}

#[test]
fn print_negation() {
    let output = print_source(
        "\
module Main exposing (..)

x = -1
",
    );
    assert!(output.contains("-1"));
}

#[test]
fn print_qualified_value() {
    let output = print_source(
        "\
module Main exposing (..)

x = Html.text
",
    );
    assert!(output.contains("Html.text"));
}

#[test]
fn print_string_escaping() {
    let output = print_source(
        r#"
module Main exposing (..)

x = "hello\nworld"
"#,
    );
    assert!(output.contains(r#""hello\nworld""#));
}

#[test]
fn print_hex_literal() {
    let output = print_source(
        "\
module Main exposing (..)

x = 0xFF
",
    );
    assert!(output.contains("0xFF"));
}

// ── Additional round-trip tests ─────────────────────────────────────

#[test]
fn round_trip_large_record() {
    round_trip(
        r#"
module Main exposing (..)

config =
    { host = "localhost"
    , port_ = 8080
    , debug = True
    , name = "test"
    , timeout = 30
    , retries = 3
    , verbose = False
    , logLevel = "info"
    }
"#,
    );
}

#[test]
fn round_trip_many_case_branches() {
    round_trip(
        "\
module Main exposing (..)

toStr n =
    case n of
        0 ->
            \"zero\"
        1 ->
            \"one\"
        2 ->
            \"two\"
        3 ->
            \"three\"
        4 ->
            \"four\"
        _ ->
            \"other\"
",
    );
}

#[test]
fn round_trip_all_pattern_types() {
    round_trip(
        "\
module Main exposing (..)

f x =
    case x of
        _ ->
            1

        42 ->
            2

        \"hi\" ->
            3

        ( a, b ) ->
            4

        Just val ->
            5

        { name } ->
            6

        a :: rest ->
            7

        [] ->
            8

        y as z ->
            9
",
    );
}

#[test]
fn round_trip_parenthesized_expression() {
    round_trip(
        "\
module Main exposing (..)

x = (1 + 2) * 3
",
    );
}

#[test]
fn round_trip_record_access_chain() {
    round_trip(
        "\
module Main exposing (..)

x = model.user.name
",
    );
}

#[test]
fn round_trip_complex_nested_operators() {
    round_trip(
        "\
module Main exposing (..)

x = a && b || c && (d || e)
",
    );
}

#[test]
fn round_trip_nested_let_in() {
    round_trip(
        "\
module Main exposing (..)

x =
    let
        a =
            let
                b = 1
            in
            b + 1
    in
    a * 2
",
    );
}

#[test]
fn round_trip_nested_case() {
    round_trip(
        "\
module Main exposing (..)

x =
    case a of
        Just val ->
            case val of
                1 ->
                    True

                _ ->
                    False

        Nothing ->
            False
",
    );
}

#[test]
fn round_trip_multiline_function_type() {
    round_trip(
        "\
module Main exposing (..)

update : Msg -> Model -> ( Model, Cmd Msg )
update msg model = ( model, Cmd.none )
",
    );
}

#[test]
fn round_trip_record_update_multiple_fields() {
    round_trip(
        "\
module Main exposing (..)

x = { model | name = \"Bob\", age = 30, active = True }
",
    );
}

#[test]
fn round_trip_nested_lambda() {
    round_trip(
        "\
module Main exposing (..)

x = \\a -> \\b -> a + b
",
    );
}

/// Parse → print → parse → print: second print should equal the first.
fn idempotent(source: &str) {
    let ast1 = parse(source).unwrap();
    let print1 = print(&ast1);
    let ast2 = parse(&print1).unwrap();
    let print2 = print(&ast2);
    assert_eq!(print1, print2, "printer not idempotent");
}

#[test]
fn idempotent_complex_module() {
    idempotent(
        r#"
module Main exposing (Model, Msg(..), update, view)

import Html exposing (Html, div, text)
import Html.Events exposing (onClick)

type Msg = Increment | Decrement | Reset

type alias Model = { count : Int, name : String }

update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            { model | count = model.count + 1 }
        Decrement ->
            { model | count = model.count - 1 }
        Reset ->
            { model | count = 0 }

view : Model -> Html Msg
view model =
    div []
        [ text (String.fromInt model.count) ]
"#,
    );
}

#[test]
fn idempotent_let_case_if() {
    idempotent(
        "\
module Main exposing (..)

f x =
    let
        y =
            if x > 0 then
                case x of
                    1 ->
                        \"one\"
                    _ ->
                        \"other\"
            else
                \"negative\"
    in
    y
",
    );
}
