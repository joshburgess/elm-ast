#![cfg(not(target_arch = "wasm32"))]

use elm_ast::declaration::Declaration;
use elm_ast::exposing::{ExposedItem, Exposing};
use elm_ast::expr::Expr;
use elm_ast::file::ElmModule;
use elm_ast::literal::Literal;
use elm_ast::module_header::ModuleHeader;
use elm_ast::parse;
use elm_ast::pattern::Pattern;
use elm_ast::type_annotation::TypeAnnotation;

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
fn v<T>(spanned: &elm_ast::Spanned<T>) -> &T {
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
            Exposing::Explicit { items, .. } => {
                assert_eq!(items.len(), 3);
                assert!(matches!(v(&items[0]), ExposedItem::Function(n) if n == "main"));
                assert!(matches!(v(&items[1]), ExposedItem::Function(n) if n == "view"));
                assert!(
                    matches!(v(&items[2]), ExposedItem::TypeExpose { name, open } if name == "Msg" && open.is_some())
                );
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
    let m = parse_ok("effect module Task where { command = MyCmd } exposing (Task, perform)");
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
    assert_eq!(
        imp.module_name.value,
        vec!["Json".to_string(), "Decode".to_string()]
    );
    assert_eq!(
        imp.alias.as_ref().unwrap().value,
        vec!["Decode".to_string()]
    );
}

#[test]
fn import_with_exposing() {
    let m = parse_ok("module Main exposing (..)\n\nimport Html exposing (div, text)");
    let imp = v(&m.imports[0]);
    match v(imp.exposing.as_ref().unwrap()) {
        Exposing::Explicit { items, .. } => {
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

#[test]
fn import_span_does_not_leak_trailing_whitespace() {
    // Regression: `parse_import` used to let `skip_whitespace` (called while
    // probing for optional `as`/`exposing`) extend the import's span past
    // trailing newlines, causing line-based refactors (e.g. removing an
    // unused import) to eat blank lines that followed the import.
    let src = "module Main exposing (..)\n\nimport Dict\n\n\nx = 1\n";
    let m = parse_ok(src);
    assert_eq!(m.imports.len(), 1);
    let imp = &m.imports[0];
    // The import's end offset should land right after `Dict`, not past the
    // trailing newlines.
    let end_offset = imp.span.end.offset;
    assert_eq!(
        &src[..end_offset],
        "module Main exposing (..)\n\nimport Dict",
        "import span leaked past end of `Dict` into trailing whitespace",
    );
}

#[test]
fn binary_expr_span_does_not_leak_trailing_whitespace() {
    // Regression: the Pratt loop in `binary_loop` calls `skip_whitespace` at
    // the top of every iteration before checking for the next operator. When
    // it breaks (no more operator), the parser's position has advanced past
    // trailing newlines, so `spanned_from` — which derives end from
    // `tokens[pos-1].span.end` — would extend the binary expression's span
    // all the way to the start of the following declaration. Downstream
    // refactors (e.g. NoEmptyListConcat's span-based replacement) then chewed
    // up the blank lines and glued the next declaration onto the replacement.
    let src = "\
module Main exposing (..)

extras : List Int
extras =
    [] ++ [ 1, 2, 3 ]


update : Int -> Int
update n = n
";
    let m = parse_ok(src);
    // Find `extras` function body expression.
    let extras_body = m
        .declarations
        .iter()
        .find_map(|d| match &d.value {
            elm_ast::declaration::Declaration::FunctionDeclaration(f)
                if f.declaration.value.name.value == "extras" =>
            {
                Some(&f.declaration.value.body)
            }
            _ => None,
        })
        .expect("extras declaration");
    let end_offset = extras_body.span.end.offset;
    let slice = &src[..end_offset];
    assert!(
        slice.ends_with("[] ++ [ 1, 2, 3 ]"),
        "binary expr span leaked past right operand; slice ended with: {:?}",
        &slice[slice.len().saturating_sub(30)..]
    );
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
                    assert!(
                        matches!(&from.value, TypeAnnotation::Typed { name, .. } if name.value == "Int")
                    );
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
            assert!(
                matches!(&sig.value.type_annotation.value, TypeAnnotation::Tupled(elems) if elems.len() == 2)
            );
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
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::Literal(Literal::Int(42))
            ));
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
            assert!(
                matches!(&func.declaration.value.body.value, Expr::Literal(Literal::String(s)) if s == "hello")
            );
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator,
                left,
                right,
                ..
            } => {
                assert_eq!(operator, "+");
                assert!(matches!(&left.value, Expr::Literal(Literal::Int(1))));
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "*")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Application(args) => {
                assert_eq!(args.len(), 3);
            }
            other => panic!("expected Application, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::IfElse {
                branches,
                else_branch: _,
            } => {
                assert_eq!(branches.len(), 1);
            }
            other => panic!("expected IfElse, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Lambda { args, .. } => {
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::List { elements, .. } => {
                assert_eq!(elements.len(), 3);
            }
            other => panic!("expected List, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Record(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(v(&fields[0].value.field), "name");
                assert_eq!(v(&fields[1].value.field), "age");
            }
            other => panic!("expected Record, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::RecordUpdate { base, updates } => {
                assert_eq!(v(base), "model");
                assert_eq!(updates.len(), 1);
            }
            other => panic!("expected RecordUpdate, got {other:?}"),
        },
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
            assert!(
                matches!(&func.declaration.value.body.value, Expr::Tuple(elems) if elems.len() == 2)
            );
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
            assert!(
                matches!(&func.declaration.value.body.value, Expr::PrefixOperator(op) if op == "+")
            );
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
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::Negation(_)
            ));
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::LetIn { declarations, .. } => {
                assert_eq!(declarations.len(), 1);
            }
            other => panic!("expected LetIn, got {other:?}"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert_eq!(branches.len(), 2);
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
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
            assert!(matches!(
                &func.declaration.value.args[0].value,
                Pattern::Anything
            ));
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(
                    matches!(&branches[0].pattern.value, Pattern::Constructor { name, args, .. } if name == "Just" && args.len() == 1)
                );
                assert!(
                    matches!(&branches[1].pattern.value, Pattern::Constructor { name, args, .. } if name == "Nothing" && args.is_empty())
                );
            }
            _ => panic!("expected CaseOf"),
        },
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
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.args[0].value {
            Pattern::Record(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(v(&fields[0]), "name");
                assert_eq!(v(&fields[1]), "age");
            }
            other => panic!("expected Record pattern, got {other:?}"),
        },
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
            assert!(
                matches!(&func.declaration.value.args[0].value, Pattern::Tuple(elems) if elems.len() == 2)
            );
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
    assert!(matches!(
        v(&m.declarations[0]),
        Declaration::CustomTypeDeclaration(_)
    ));
    // type alias Model
    assert!(matches!(
        v(&m.declarations[1]),
        Declaration::AliasDeclaration(_)
    ));
    // update function
    assert!(matches!(
        v(&m.declarations[2]),
        Declaration::FunctionDeclaration(_)
    ));
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

// ── Port declarations ────────────────────────────────────────────────

#[test]
fn port_declaration() {
    let src = "\
port module Ports exposing (..)

port sendMessage : String -> Cmd msg
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 1);
    match v(&m.declarations[0]) {
        Declaration::PortDeclaration(sig) => {
            assert_eq!(v(&sig.name), "sendMessage");
            match &sig.type_annotation.value {
                TypeAnnotation::FunctionType { from, to } => {
                    assert!(matches!(
                        &from.value,
                        TypeAnnotation::Typed { name, .. } if name.value == "String"
                    ));
                    assert!(matches!(
                        &to.value,
                        TypeAnnotation::Typed { name, .. } if name.value == "Cmd"
                    ));
                }
                _ => panic!("expected FunctionType"),
            }
        }
        _ => panic!("expected PortDeclaration"),
    }
}

#[test]
fn port_declaration_round_trip() {
    let src = "\
port module Ports exposing (..)

port sendMessage : String -> Cmd msg

port onMessage : (String -> msg) -> Sub msg
";
    let ast1 = parse(src).unwrap();
    let printed = elm_ast::print::print(&ast1);
    let ast2 = parse(&printed).unwrap();
    assert_eq!(ast2.declarations.len(), 2);
    assert!(matches!(
        v(&ast2.declarations[0]),
        Declaration::PortDeclaration(_)
    ));
    assert!(matches!(
        v(&ast2.declarations[1]),
        Declaration::PortDeclaration(_)
    ));
}

// ── Infix declarations ──────────────────────────────────────────────

#[test]
fn infix_declaration_left() {
    let src = "\
module Main exposing (..)

infix left 6 (+) = add
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 1);
    match v(&m.declarations[0]) {
        Declaration::InfixDeclaration(infix) => {
            assert_eq!(infix.operator.value, "+");
            assert_eq!(infix.function.value, "add");
            assert_eq!(infix.precedence.value, 6);
            assert_eq!(
                infix.direction.value,
                elm_ast::operator::InfixDirection::Left
            );
        }
        _ => panic!("expected InfixDeclaration"),
    }
}

#[test]
fn infix_declaration_right() {
    let src = "\
module Main exposing (..)

infix right 5 (|>) = apR
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::InfixDeclaration(infix) => {
            assert_eq!(infix.operator.value, "|>");
            assert_eq!(infix.function.value, "apR");
            assert_eq!(infix.precedence.value, 5);
            assert_eq!(
                infix.direction.value,
                elm_ast::operator::InfixDirection::Right
            );
        }
        _ => panic!("expected InfixDeclaration"),
    }
}

#[test]
fn infix_declaration_non() {
    let src = "\
module Main exposing (..)

infix non 4 (==) = eq
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::InfixDeclaration(infix) => {
            assert_eq!(infix.operator.value, "==");
            assert_eq!(infix.function.value, "eq");
            assert_eq!(infix.precedence.value, 4);
            assert_eq!(
                infix.direction.value,
                elm_ast::operator::InfixDirection::Non
            );
        }
        _ => panic!("expected InfixDeclaration"),
    }
}

#[test]
fn infix_declaration_round_trip() {
    let src = "\
module Main exposing (..)

infix left 6 (+) = add

infix right 0 (|>) = apR
";
    let ast1 = parse(src).unwrap();
    let printed = elm_ast::print::print(&ast1);
    let ast2 = parse(&printed).unwrap();
    assert_eq!(ast2.declarations.len(), 2);
    assert!(matches!(
        v(&ast2.declarations[0]),
        Declaration::InfixDeclaration(_)
    ));
    assert!(matches!(
        v(&ast2.declarations[1]),
        Declaration::InfixDeclaration(_)
    ));
}

// ── Top-level destructuring ─────────────────────────────────────────

#[test]
fn top_level_destructuring_tuple() {
    let src = "\
module Main exposing (..)

( a, b ) = someTuple
";
    let m = parse_ok(src);
    assert_eq!(m.declarations.len(), 1);
    match v(&m.declarations[0]) {
        Declaration::Destructuring { pattern, body } => {
            assert!(matches!(&pattern.value, Pattern::Tuple(elems) if elems.len() == 2));
            assert!(matches!(
                &body.value,
                Expr::FunctionOrValue { name, .. } if name == "someTuple"
            ));
        }
        _ => panic!("expected Destructuring"),
    }
}

#[test]
fn top_level_destructuring_record() {
    let src = "\
module Main exposing (..)

{ name, age } = person
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::Destructuring { pattern, .. } => match &pattern.value {
            Pattern::Record(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(v(&fields[0]), "name");
                assert_eq!(v(&fields[1]), "age");
            }
            _ => panic!("expected Record pattern"),
        },
        _ => panic!("expected Destructuring"),
    }
}

#[test]
fn top_level_destructuring_round_trip() {
    let src = "\
module Main exposing (..)

( a, b ) = someTuple
";
    let ast1 = parse(src).unwrap();
    let printed = elm_ast::print::print(&ast1);
    let ast2 = parse(&printed).unwrap();
    assert_eq!(ast2.declarations.len(), 1);
    assert!(matches!(
        v(&ast2.declarations[0]),
        Declaration::Destructuring { .. }
    ));
}

// ── GLSL expressions ────────────────────────────────────────────────

#[test]
fn glsl_expression_parse() {
    let src = "\
module Main exposing (..)

shader = [glsl|
    precision mediump float;
    void main() {
        gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
    }
|]
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::GLSLExpression(src) => {
                assert!(src.contains("precision mediump float"));
                assert!(src.contains("gl_FragColor"));
            }
            other => panic!("expected GLSLExpression, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn glsl_expression_round_trip() {
    let src = "\
module Main exposing (..)

shader = [glsl|
    void main() {}
|]
";
    let ast1 = parse(src).unwrap();
    let printed = elm_ast::print::print(&ast1);
    assert!(printed.contains("[glsl|"));
    assert!(printed.contains("|]"));
    let ast2 = parse(&printed).unwrap();
    assert_eq!(ast2.declarations.len(), 1);
    match v(&ast2.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::GLSLExpression(_)
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn glsl_lexer_token() {
    let lexer = elm_ast::Lexer::new("[glsl| some shader code |]");
    let (tokens, errors) = lexer.tokenize();
    assert!(errors.is_empty(), "unexpected lex errors: {errors:?}");
    assert!(
        tokens
            .iter()
            .any(|t| matches!(&t.value, elm_ast::Token::Glsl(_)))
    );
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

// ── Parse error tests ───────────────────────────────────────────────
// Each test verifies that the parser returns an error (not a panic)
// for a specific malformed input.

#[test]
fn error_expected_direction_in_infix() {
    let result = parse("module Main exposing (..)\n\ninfix bogus 6 (+) = add");
    assert!(result.is_err());
    let msg = result.unwrap_err()[0].message.clone();
    assert!(
        msg.contains("left") || msg.contains("right") || msg.contains("non"),
        "error should mention valid directions, got: {msg}"
    );
}

#[test]
fn error_expected_operator_in_infix() {
    let result = parse("module Main exposing (..)\n\ninfix left 6 (notAnOp) = add");
    // "notAnOp" is a LowerName, not an Operator, so this should error
    // when expecting ')' or produce an error about expected operator
    assert!(result.is_err());
}

#[test]
fn error_expected_precedence_in_infix() {
    let result = parse("module Main exposing (..)\n\ninfix left foo (+) = add");
    assert!(result.is_err());
    let msg = result.unwrap_err()[0].message.clone();
    assert!(
        msg.contains("precedence"),
        "error should mention precedence, got: {msg}"
    );
}

#[test]
fn error_expected_comma_or_rparen_in_expression() {
    // `( 1 ]` — after the expression `1` we expect `,` or `)`, but got `]`
    let result = parse("module Main exposing (..)\n\nx = ( 1 ]");
    assert!(result.is_err());
}

#[test]
fn error_expected_rparen_after_prefix_operator() {
    let result = parse("module Main exposing (..)\n\nx = (+ 1)");
    // `(+ 1)` is not a valid prefix operator expression — `(+)` is
    assert!(result.is_err());
}

#[test]
fn error_expected_rparen_or_comma() {
    let result = parse("module Main exposing (..)\n\nx = ( 1, 2 ]");
    assert!(result.is_err());
}

#[test]
fn error_expected_at_least_one_lambda_arg() {
    let result = parse("module Main exposing (..)\n\nx = \\ -> 1");
    assert!(result.is_err());
    let msg = result.unwrap_err()[0].message.clone();
    assert!(
        msg.contains("argument") || msg.contains("lambda"),
        "error should mention lambda arguments, got: {msg}"
    );
}

#[test]
fn error_expected_at_least_one_case_branch() {
    let result = parse(
        "\
module Main exposing (..)

x =
    case y of
",
    );
    assert!(result.is_err());
    let msg = result.unwrap_err()[0].message.clone();
    assert!(
        msg.contains("case branch"),
        "error should mention case branch, got: {msg}"
    );
}

#[test]
fn error_expected_field_name_after_dot() {
    let result = parse("module Main exposing (..)\n\nx = foo.123");
    assert!(result.is_err());
}

#[test]
fn error_expected_operator_in_exposing_list() {
    let result = parse("module Main exposing ((123))");
    assert!(result.is_err());
    let msg = result.unwrap_err()[0].message.clone();
    assert!(
        msg.contains("operator") || msg.contains("exposing"),
        "error should mention operator in exposing, got: {msg}"
    );
}

#[test]
fn error_expected_comma_or_rparen_in_pattern() {
    let result = parse("module Main exposing (..)\n\nf ( a b ) = a");
    assert!(result.is_err());
}

#[test]
fn error_expected_number_after_minus_in_pattern() {
    let result = parse("module Main exposing (..)\n\nf (-foo) = 1");
    assert!(result.is_err());
}

#[test]
fn error_expected_comma_or_rparen_in_type() {
    // `( Int ]` — in a type tuple, after `Int` we expect `,` or `)`, but got `]`
    let result = parse("module Main exposing (..)\n\nf : ( Int ]\nf x = 1");
    assert!(result.is_err());
}

// ── Error recovery tests ────────────────────────────────────────────

#[test]
fn error_recovery_skips_bad_declaration() {
    let src = "\
module Main exposing (..)

good1 = 1

bad declaration here !!!

good2 = 2
";
    let (maybe_module, errors) = elm_ast::parse_recovering(src);
    assert!(!errors.is_empty(), "should have parse errors");
    let module = maybe_module.expect("should have partial module");
    // At least one good declaration should have been recovered
    assert!(
        !module.declarations.is_empty(),
        "should recover at least one declaration"
    );
}

#[test]
fn error_recovery_returns_partial_ast() {
    let src = "\
module Main exposing (..)

x = 1

y = if then

z = 3
";
    let (maybe_module, errors) = elm_ast::parse_recovering(src);
    assert!(!errors.is_empty());
    let module = maybe_module.expect("should have partial module");
    // Should recover x and z even though y fails
    assert!(
        module.declarations.len() >= 2,
        "should recover at least 2 declarations, got {}",
        module.declarations.len()
    );
}

// ── Parenthesized expressions ────────────────────────────────────────

#[test]
fn parenthesized_expression() {
    let src = "\
module Main exposing (..)

x = (42)
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::Parenthesized { expr: inner, .. } if matches!(&inner.value, Expr::Literal(Literal::Int(42)))
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn nested_parenthesized_expression() {
    let src = "\
module Main exposing (..)

x = ((1 + 2))
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::Parenthesized { .. }
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn parens_override_precedence() {
    // (1 + 2) * 3  should parse as  (1 + 2) * 3, not 1 + (2 * 3)
    let src = "\
module Main exposing (..)

x = (1 + 2) * 3
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication { operator, left, .. } => {
                assert_eq!(operator, "*");
                assert!(matches!(&left.value, Expr::Parenthesized { .. }));
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Application edge cases ───────────────────────────────────────────

#[test]
fn single_arg_application() {
    let src = "\
module Main exposing (..)

x = negate 5
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Application(args) => assert_eq!(args.len(), 2),
            other => panic!("expected Application, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn many_arg_application() {
    let src = "\
module Main exposing (..)

x = f a b c d e
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Application(args) => assert_eq!(args.len(), 6),
            other => panic!("expected Application, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn application_with_complex_args() {
    let src = "\
module Main exposing (..)

x = f (a + b) [1, 2] { name = \"hi\" }
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Application(args) => {
                assert_eq!(args.len(), 4);
                assert!(matches!(&args[1].value, Expr::Parenthesized { .. }));
                assert!(matches!(&args[2].value, Expr::List { .. }));
                assert!(matches!(&args[3].value, Expr::Record(_)));
            }
            other => panic!("expected Application, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Tuple edge cases ─────────────────────────────────────────────────

#[test]
fn three_element_tuple() {
    let src = "\
module Main exposing (..)

x = ( 1, \"hello\", True )
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Tuple(elems) => assert_eq!(elems.len(), 3),
            other => panic!("expected Tuple, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn nested_tuples() {
    let src = "\
module Main exposing (..)

x = ( (1, 2), (3, 4) )
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Tuple(elems) => {
                assert_eq!(elems.len(), 2);
                assert!(matches!(&elems[0].value, Expr::Tuple(_)));
                assert!(matches!(&elems[1].value, Expr::Tuple(_)));
            }
            other => panic!("expected Tuple, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Prefix operators ─────────────────────────────────────────────────

#[test]
fn prefix_operators_all() {
    for op in &[
        "+", "-", "*", "/", "//", "++", "::", "&&", "||", "==", "/=", "<", ">", "<=", ">=", "^",
        "|>", "<|", ">>", "<<",
    ] {
        let src = format!("module Main exposing (..)\n\nx = ({op})");
        let m = parse_ok(&src);
        match v(&m.declarations[0]) {
            Declaration::FunctionDeclaration(func) => {
                assert!(
                    matches!(&func.declaration.value.body.value, Expr::PrefixOperator(o) if o == op),
                    "failed for operator {op}"
                );
            }
            _ => panic!("expected FunctionDeclaration for {op}"),
        }
    }
}

// ── Record access ────────────────────────────────────────────────────

#[test]
fn record_access_function() {
    let src = "\
module Main exposing (..)

x = .name
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::RecordAccessFunction(n) if n == "name"
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_access_chain() {
    let src = "\
module Main exposing (..)

x = model.user.name
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::RecordAccess { record, field } => {
                assert_eq!(&field.value, "name");
                assert!(
                    matches!(&record.value, Expr::RecordAccess { field, .. } if field.value == "user")
                );
            }
            other => panic!("expected RecordAccess, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn record_access_on_function_result() {
    let src = "\
module Main exposing (..)

x = (getModel ()).name
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            assert!(matches!(
                &func.declaration.value.body.value,
                Expr::RecordAccess { .. }
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Operator precedence and associativity ────────────────────────────

#[test]
fn boolean_operator_precedence() {
    // a || b && c  should parse as  a || (b && c) because && binds tighter
    let src = "\
module Main exposing (..)

x = a || b && c
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator, right, ..
            } => {
                assert_eq!(operator, "||");
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "&&")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn right_associative_cons() {
    // 1 :: 2 :: []  should parse as  1 :: (2 :: [])
    let src = "\
module Main exposing (..)

x = 1 :: 2 :: []
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator,
                left,
                right,
                ..
            } => {
                assert_eq!(operator, "::");
                assert!(matches!(&left.value, Expr::Literal(Literal::Int(1))));
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "::")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn right_associative_pipe_left() {
    // f <| g <| x  should parse as  f <| (g <| x)
    let src = "\
module Main exposing (..)

x = f <| g <| h
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator, right, ..
            } => {
                assert_eq!(operator, "<|");
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "<|")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn append_with_arithmetic() {
    // a ++ b ++ c  is right-assoc: a ++ (b ++ c)
    let src = "\
module Main exposing (..)

x = a ++ b ++ c
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator, right, ..
            } => {
                assert_eq!(operator, "++");
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "++")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn mixed_arithmetic_and_comparison() {
    // a + b == c * d  should be  (a + b) == (c * d)
    let src = "\
module Main exposing (..)

x = a + b == c * d
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::OperatorApplication {
                operator,
                left,
                right,
                ..
            } => {
                assert_eq!(operator, "==");
                assert!(
                    matches!(&left.value, Expr::OperatorApplication { operator, .. } if operator == "+")
                );
                assert!(
                    matches!(&right.value, Expr::OperatorApplication { operator, .. } if operator == "*")
                );
            }
            other => panic!("expected OperatorApplication, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Qualified values ─────────────────────────────────────────────────

#[test]
fn deeply_qualified_value() {
    let src = "\
module Main exposing (..)

x = A.B.C.func
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::FunctionOrValue { module_name, name } => {
                assert_eq!(module_name, &vec!["A", "B", "C"]);
                assert_eq!(name, "func");
            }
            other => panic!("expected FunctionOrValue, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Pattern edge cases ───────────────────────────────────────────────

#[test]
fn string_literal_pattern() {
    let src = "\
module Main exposing (..)

x s =
    case s of
        \"hello\" -> 1
        _ -> 0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(
                    &branches[0].pattern.value,
                    Pattern::Literal(Literal::String(s)) if s == "hello"
                ));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn char_literal_pattern() {
    let src = "\
module Main exposing (..)

x c =
    case c of
        'a' -> 1
        _ -> 0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(
                    &branches[0].pattern.value,
                    Pattern::Literal(Literal::Char('a'))
                ));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn list_pattern() {
    let src = "\
module Main exposing (..)

x xs =
    case xs of
        [ a, b, c ] -> a
        _ -> 0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(
                    matches!(&branches[0].pattern.value, Pattern::List(elems) if elems.len() == 3)
                );
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn as_pattern() {
    let src = "\
module Main exposing (..)

x y =
    case y of
        (Just v) as original -> original
        _ -> Nothing
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(&branches[0].pattern.value, Pattern::As { .. }));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn hex_pattern() {
    let src = "\
module Main exposing (..)

x n =
    case n of
        0xFF -> True
        _ -> False
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(&branches[0].pattern.value, Pattern::Hex(255)));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn cons_pattern() {
    let src = "\
module Main exposing (..)

x xs =
    case xs of
        head :: tail -> head
        [] -> 0
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(&branches[0].pattern.value, Pattern::Cons { .. }));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn unit_pattern() {
    let src = "\
module Main exposing (..)

x y =
    case y of
        () -> True
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => {
                assert!(matches!(&branches[0].pattern.value, Pattern::Unit));
            }
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Type annotation edge cases ───────────────────────────────────────

#[test]
fn generic_type_variable() {
    let src = "\
module Main exposing (..)

identity : a -> a
identity x = x
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            match &sig.value.type_annotation.value {
                TypeAnnotation::FunctionType { from, to } => {
                    assert!(matches!(&from.value, TypeAnnotation::GenericType(n) if n == "a"));
                    assert!(matches!(&to.value, TypeAnnotation::GenericType(n) if n == "a"));
                }
                other => panic!("expected FunctionType, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn unit_type() {
    let src = "\
module Main exposing (..)

x : ()
x = ()
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            assert!(matches!(
                &sig.value.type_annotation.value,
                TypeAnnotation::Unit
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn three_element_tuple_type() {
    let src = "\
module Main exposing (..)

x : ( Int, String, Bool )
x = ( 1, \"hi\", True )
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            assert!(matches!(
                &sig.value.type_annotation.value,
                TypeAnnotation::Tupled(elems) if elems.len() == 3
            ));
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn generic_record_type_in_function_signature() {
    let src = "\
module Main exposing (..)

getName : { a | name : String } -> String
getName r = r.name
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => {
            let sig = func.signature.as_ref().unwrap();
            match &sig.value.type_annotation.value {
                TypeAnnotation::FunctionType { from, .. } => {
                    assert!(matches!(
                        &from.value,
                        TypeAnnotation::GenericRecord { base, fields }
                        if base.value == "a" && fields.len() == 1
                    ));
                }
                other => panic!("expected FunctionType, got {other:?}"),
            }
        }
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Case edge cases ──────────────────────────────────────────────────

#[test]
fn single_branch_case() {
    let src = "\
module Main exposing (..)

x y =
    case y of
        _ -> 42
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => assert_eq!(branches.len(), 1),
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn many_branch_case() {
    let mut src = String::from("module Main exposing (..)\n\nx n =\n    case n of\n");
    for i in 0..15 {
        src.push_str(&format!("        {} -> {}\n", i, i * 10));
    }
    src.push_str("        _ -> -1\n");
    let m = parse_ok(&src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::CaseOf { branches, .. } => assert_eq!(branches.len(), 16),
            other => panic!("expected CaseOf, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Let expression edge cases ────────────────────────────────────────

#[test]
fn let_with_type_signature() {
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
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::LetIn { declarations, .. } => {
                assert_eq!(declarations.len(), 1);
            }
            other => panic!("expected LetIn, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn let_destructuring() {
    let src = "\
module Main exposing (..)

x =
    let
        ( a, b ) = (1, 2)
    in
    a + b
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::LetIn { declarations, .. } => {
                assert!(matches!(
                    &declarations[0].value,
                    elm_ast::expr::LetDeclaration::Destructuring { .. }
                ));
            }
            other => panic!("expected LetIn, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn let_multiple_declarations() {
    let src = "\
module Main exposing (..)

x =
    let
        a = 1
        b = 2
        c = 3
        d = 4
    in
    a + b + c + d
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::LetIn { declarations, .. } => assert_eq!(declarations.len(), 4),
            other => panic!("expected LetIn, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Lambda edge cases ────────────────────────────────────────────────

#[test]
fn single_arg_lambda() {
    let src = "\
module Main exposing (..)

x = \\a -> a
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Lambda { args, .. } => assert_eq!(args.len(), 1),
            other => panic!("expected Lambda, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

#[test]
fn multi_arg_lambda() {
    let src = "\
module Main exposing (..)

x = \\a b c d -> a + b + c + d
";
    let m = parse_ok(src);
    match v(&m.declarations[0]) {
        Declaration::FunctionDeclaration(func) => match &func.declaration.value.body.value {
            Expr::Lambda { args, .. } => assert_eq!(args.len(), 4),
            other => panic!("expected Lambda, got {other:?}"),
        },
        _ => panic!("expected FunctionDeclaration"),
    }
}

// ── Error recovery edge cases ────────────────────────────────────────

#[test]
fn error_recovery_multiple_bad_declarations() {
    let src = "\
module Main exposing (..)

a = 1

b = if then

c = case of

d = 4
";
    let (maybe_module, errors) = elm_ast::parse_recovering(src);
    assert!(errors.len() >= 2);
    let module = maybe_module.expect("should have partial module");
    assert!(
        module.declarations.len() >= 2,
        "should recover at least a and d, got {}",
        module.declarations.len()
    );
}

#[test]
fn error_recovery_bad_first_declaration() {
    let src = "\
module Main exposing (..)

a = if then

b = 2

c = 3
";
    let (maybe_module, errors) = elm_ast::parse_recovering(src);
    assert!(!errors.is_empty());
    let module = maybe_module.expect("should have partial module");
    assert!(
        module.declarations.len() >= 2,
        "should recover b and c, got {}",
        module.declarations.len()
    );
}
