use elm_ast_rs::declaration::Declaration;
use elm_ast_rs::exposing::{ExposedItem, Exposing};
use elm_ast_rs::expr::Expr;
use elm_ast_rs::file::ElmModule;
use elm_ast_rs::literal::Literal;
use elm_ast_rs::module_header::ModuleHeader;
use elm_ast_rs::parse;
use elm_ast_rs::pattern::Pattern;
use elm_ast_rs::type_annotation::TypeAnnotation;

/// Parse and unwrap, panicking on error.
fn parse_ok(source: &str) -> ElmModule {
    match parse(source) {
        Ok(m) => m,
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            panic!("parse failed with {} error(s)", errors.len());
        }
    }
}

/// Helper to extract the inner value from a Spanned.
fn v<T>(spanned: &elm_ast_rs::Spanned<T>) -> &T {
    &spanned.value
}

// ── Module headers ───────────────────────────────────────────────────

#[test]
fn normal_module_exposing_all() {
    let m = parse_ok("module Main exposing (..)");
    match v(&m.header) {
        ModuleHeader::Normal { name, exposing } => {
            assert_eq!(v(name), &vec!["Main".to_string()]);
            assert!(matches!(v(exposing), Exposing::All(_)));
        }
        _ => panic!("expected Normal module"),
    }
}

#[test]
fn normal_module_exposing_explicit() {
    let m = parse_ok("module Main exposing (main, view, Msg(..))");
    match v(&m.header) {
        ModuleHeader::Normal { exposing, .. } => match v(exposing) {
            Exposing::Explicit(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(v(&items[0]), ExposedItem::Function(n) if n == "main"));
                assert!(matches!(v(&items[1]), ExposedItem::Function(n) if n == "view"));
                assert!(matches!(v(&items[2]), ExposedItem::TypeExpose { name, open } if name == "Msg" && open.is_some()));
            }
            _ => panic!("expected Explicit exposing"),
        },
        _ => panic!("expected Normal module"),
    }
}

#[test]
fn dotted_module_name() {
    let m = parse_ok("module Html.Attributes exposing (..)");
    match v(&m.header) {
        ModuleHeader::Normal { name, .. } => {
            assert_eq!(v(name), &vec!["Html".to_string(), "Attributes".to_string()]);
        }
        _ => panic!("expected Normal module"),
    }
}

#[test]
fn port_module() {
    let m = parse_ok("port module Ports exposing (sendMessage)");
    assert!(matches!(v(&m.header), ModuleHeader::Port { .. }));
}

#[test]
fn effect_module() {
    let m = parse_ok(
        "effect module Task where { command = MyCmd } exposing (Task, perform)",
    );
    match v(&m.header) {
        ModuleHeader::Effect {
            command,
            subscription,
            ..
        } => {
            assert_eq!(command.as_ref().unwrap().value, "MyCmd");
            assert!(subscription.is_none());
        }
        _ => panic!("expected Effect module"),
    }
}

// ── Imports ──────────────────────────────────────────────────────────

#[test]
fn simple_import() {
    let m = parse_ok("module Main exposing (..)\n\nimport Html");
    assert_eq!(m.imports.len(), 1);
    assert_eq!(v(&m.imports[0]).module_name.value, vec!["Html".to_string()]);
    assert!(v(&m.imports[0]).alias.is_none());
    assert!(v(&m.imports[0]).exposing.is_none());
}

#[test]
fn import_with_alias() {
    let m = parse_ok("module Main exposing (..)\n\nimport Json.Decode as Decode");
    let imp = v(&m.imports[0]);
    assert_eq!(imp.module_name.value, vec!["Json".to_string(), "Decode".to_string()]);
    assert_eq!(imp.alias.as_ref().unwrap().value, vec!["Decode".to_string()]);
}

#[test]
fn import_with_exposing() {
    let m = parse_ok("module Main exposing (..)\n\nimport Html exposing (div, text)");
    let imp = v(&m.imports[0]);
    match v(imp.exposing.as_ref().unwrap()) {
        Exposing::Explicit(items) => {
            assert_eq!(items.len(), 2);
        }
        _ => panic!("expected Explicit"),
    }
}

#[test]
fn import_with_alias_and_exposing() {
    let m = parse_ok(
        "module Main exposing (..)\n\nimport Html.Attributes as HA exposing (class, style)",
    );
    let imp = v(&m.imports[0]);
    assert!(imp.alias.is_some());
    assert!(imp.exposing.is_some());
}

#[test]
fn multiple_imports() {
    let src = "\
module Main exposing (..)

import Html
import Html.Attributes
import Html.Events exposing (onClick)
";
    let m = parse_ok(src);
    assert_eq!(m.imports.len(), 3);
}

// ── Type annotations ─────────────────────────────────────────────────

#[test]
fn simple_type_annotation() {
    let src = "\
module Main exposing (..)

foo : Int
foo = 42
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 1);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(func.signature.is_some());
            let sig = func.signature.as_ref().unwrap();
            assert_eq!(v(&sig.value.name), "foo");
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn function_type_annotation() {
    let src = "\
module Main exposing (..)

add : Int -> Int -> Int
add x y = x
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            // Int -> Int -> Int  =  Int -> (Int -> Int)
            match &sig.value.type_annotation.value {
                TypeAnnotation::FunctionType { from, to } => {
                    assert!(matches!(&from.value, TypeAnnotation::Typed { name, .. } if name.value == "Int"));
                    assert!(matches!(&to.value, TypeAnnotation::FunctionType { .. }));
                }
                _ => panic!("expected FunctionType"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_type() {
    let src = "\
module Main exposing (..)

type alias Model = { name : String, age : Int }
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::AliasDeclaration(alias) => {
            assert_eq!(v(&alias.name), "Model");
            match &alias.type_annotation.value {
                TypeAnnotation::Record(fields) => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(v(&fields[0].value.name), "name");
                    assert_eq!(v(&fields[1].value.name), "age");
                }
                _ => panic!("expected Record type"),
            }
        }
        _ => panic!("expected AliasDeclaration"),
    }
}

#[test]
fn generic_record_type() {
    let src = "\
module Main exposing (..)

type alias Named a = { a | name : String }
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::AliasDeclaration(alias) => {
            assert_eq!(alias.generics.len(), 1);
            match &alias.type_annotation.value {
                TypeAnnotation::GenericRecord { base, fields } => {
                    assert_eq!(v(base), "a");
                    assert_eq!(fields.len(), 1);
                }
                _ => panic!("expected GenericRecord type"),
            }
        }
        _ => panic!("expected AliasDeclaration"),
    }
}

#[test]
fn tuple_type() {
    let src = "\
module Main exposing (..)

pair : ( Int, String )
pair = ( 1, \"hello\" )
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            assert!(matches!(&sig.value.type_annotation.value, TypeAnnotation::Tupled(elems) if elems.len() == 2));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn parameterized_type() {
    let src = "\
module Main exposing (..)

foo : Maybe Int
foo = Nothing
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            match &sig.value.type_annotation.value {
                TypeAnnotation::Typed { name, args, .. } => {
                    assert_eq!(v(name), "Maybe");
                    assert_eq!(args.len(), 1);
                }
                _ => panic!("expected Typed"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Custom types ─────────────────────────────────────────────────────

#[test]
fn simple_custom_type() {
    let src = "\
module Main exposing (..)

type Msg = Increment | Decrement
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::CustomTypeDeclaration(ct) => {
            assert_eq!(v(&ct.name), "Msg");
            assert_eq!(ct.constructors.len(), 2);
            assert_eq!(v(&ct.constructors[0].value.name), "Increment");
            assert_eq!(v(&ct.constructors[1].value.name), "Decrement");
        }
        _ => panic!("expected CustomTypeDeclaration"),
    }
}

#[test]
fn parameterized_custom_type() {
    let src = "\
module Main exposing (..)

type Maybe a = Just a | Nothing
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::CustomTypeDeclaration(ct) => {
            assert_eq!(v(&ct.name), "Maybe");
            assert_eq!(ct.generics.len(), 1);
            assert_eq!(ct.constructors.len(), 2);
            assert_eq!(ct.constructors[0].value.args.len(), 1);
            assert_eq!(ct.constructors[1].value.args.len(), 0);
        }
        _ => panic!("expected CustomTypeDeclaration"),
    }
}

#[test]
fn custom_type_with_multiple_args() {
    let src = "\
module Main exposing (..)

type Result error value = Ok value | Err error
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::CustomTypeDeclaration(ct) => {
            assert_eq!(ct.generics.len(), 2);
            assert_eq!(ct.constructors[0].value.args.len(), 1);
            assert_eq!(ct.constructors[1].value.args.len(), 1);
        }
        _ => panic!("expected CustomTypeDeclaration"),
    }
}

// ── Expressions ──────────────────────────────────────────────────────

#[test]
fn literal_expression() {
    let src = "\
module Main exposing (..)

x = 42
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::Literal(Literal::Int(42))));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn string_literal_expression() {
    let src = "\
module Main exposing (..)

x = \"hello\"
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::Literal(Literal::String(s)) if s == "hello"));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn binary_operator_precedence() {
    // 1 + 2 * 3  should parse as  1 + (2 * 3)
    let src = "\
module Main exposing (..)

x = 1 + 2 * 3
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::OperatorApplication {
                    operator,
                    left,
                    right,
                    ..
                } => {
                    assert_eq!(operator, "+");
                    assert!(matches!(&left.value, Expr::Literal(Literal::Int(1))));
                    assert!(matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "*"));
                }
                other => panic!("expected OperatorApplication, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn pipeline_operators() {
    let src = "\
module Main exposing (..)

x = foo |> bar |> baz
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            // |> is left-associative, so: (foo |> bar) |> baz
            match &func.declaration.value.body.value {
                Expr::OperatorApplication { operator, .. } => {
                    assert_eq!(operator, "|>");
                }
                other => panic!("expected OperatorApplication, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn function_application() {
    let src = "\
module Main exposing (..)

x = add 1 2
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::Application(args) => {
                    assert_eq!(args.len(), 3);
                }
                other => panic!("expected Application, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn if_then_else() {
    let src = "\
module Main exposing (..)

x = if True then 1 else 0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::IfElse { branches, else_branch } => {
                    assert_eq!(branches.len(), 1);
                }
                other => panic!("expected IfElse, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn lambda_expression() {
    let src = "\
module Main exposing (..)

x = \\a b -> a
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::Lambda { args, .. } => {
                    assert_eq!(args.len(), 2);
                }
                other => panic!("expected Lambda, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn list_expression() {
    let src = "\
module Main exposing (..)

x = [ 1, 2, 3 ]
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::List(elements) => {
                    assert_eq!(elements.len(), 3);
                }
                other => panic!("expected List, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_expression() {
    let src = r#"
module Main exposing (..)

x = { name = "Alice", age = 30 }
"#;
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::Record(fields) => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(v(&fields[0].value.field), "name");
                    assert_eq!(v(&fields[1].value.field), "age");
                }
                other => panic!("expected Record, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_update_expression() {
    let src = "\
module Main exposing (..)

x = { model | count = 0 }
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::RecordUpdate { base, updates } => {
                    assert_eq!(v(base), "model");
                    assert_eq!(updates.len(), 1);
                }
                other => panic!("expected RecordUpdate, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn tuple_expression() {
    let src = "\
module Main exposing (..)

x = ( 1, 2 )
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::Tuple(elems) if elems.len() == 2));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn unit_expression() {
    let src = "\
module Main exposing (..)

x = ()
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::Unit));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn prefix_operator() {
    let src = "\
module Main exposing (..)

x = (+)
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::PrefixOperator(op) if op == "+"));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn negation() {
    let src = "\
module Main exposing (..)

x = -1
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.body.value, Expr::Negation(_)));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn let_in_expression() {
    let src = "\
module Main exposing (..)

x =
    let
        y = 1
    in
    y
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::LetIn { declarations, body } => {
                    assert_eq!(declarations.len(), 1);
                }
                other => panic!("expected LetIn, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn case_of_expression() {
    let src = "\
module Main exposing (..)

x =
    case msg of
        Increment ->
            1
        Decrement ->
            0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::CaseOf { branches, .. } => {
                    assert_eq!(branches.len(), 2);
                }
                other => panic!("expected CaseOf, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Patterns ─────────────────────────────────────────────────────────

#[test]
fn wildcard_pattern() {
    let src = "\
module Main exposing (..)

f _ = 1
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert_eq!(func.declaration.value.args.len(), 1);
            assert!(matches!(&func.declaration.value.args[0].value, Pattern::Anything));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn constructor_pattern_in_case() {
    let src = "\
module Main exposing (..)

x =
    case m of
        Just val ->
            val
        Nothing ->
            0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.body.value {
                Expr::CaseOf { branches, .. } => {
                    assert!(matches!(&branches[0].pattern.value, Pattern::Constructor { name, args, .. } if name == "Just" && args.len() == 1));
                    assert!(matches!(&branches[1].pattern.value, Pattern::Constructor { name, args, .. } if name == "Nothing" && args.is_empty()));
                }
                _ => panic!("expected CaseOf"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_destructuring_pattern() {
    let src = "\
module Main exposing (..)

f { name, age } = name
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            match &func.declaration.value.args[0].value {
                Pattern::Record(fields) => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(v(&fields[0]), "name");
                    assert_eq!(v(&fields[1]), "age");
                }
                other => panic!("expected Record pattern, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn tuple_pattern() {
    let src = "\
module Main exposing (..)

f ( a, b ) = a
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(&func.declaration.value.args[0].value, Pattern::Tuple(elems) if elems.len() == 2));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Full programs ────────────────────────────────────────────────────

#[test]
fn minimal_program() {
    let src = "\
module Main exposing (main)

import Html

main = Html.text \"Hello\"
";
    let m = parse_ok(src);
    assert_eq!(m.imports.len(), 1);
    assert_eq!(m.declarations.len(), 1);
}

#[test]
fn counter_program() {
    let src = r#"
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
"#;
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 3);

    // type Msg
    assert!(matches!(v(&m.declarations[0]), Declaration::CustomTypeDeclaration(_)));
    // type alias Model
    assert!(matches!(v(&m.declarations[1]), Declaration::AliasDeclaration(_)));
    // update function
    assert!(matches!(v(&m.declarations[2]), Declaration::FunctionDeclaration(_)));
}

#[test]
fn multiple_functions() {
    let src = "\
module Main exposing (..)

add x y = x + y

multiply x y = x * y

identity x = x
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 3);
}

// ── Error cases ──────────────────────────────────────────────────────

#[test]
fn missing_module_header() {
    let result = parse("import Html");
    assert!(result.is_err());
}

#[test]
fn missing_exposing() {
    let result = parse("module Main");
    assert!(result.is_err());
}
