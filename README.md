# elm-ast-rs

A `syn`-quality Rust library for parsing and constructing Elm 0.19.1 ASTs.

## Overview

`elm-ast-rs` provides a complete, strongly-typed representation of Elm source code as a Rust AST, along with a parser, printer, and visitor/fold traits for traversal and transformation. It is modeled after Rust's [`syn`](https://github.com/dtolnay/syn) crate, with a formatting approach inspired by [`elm-format`](https://github.com/avh4/elm-format).

**Tested against 93 real-world `.elm` files from 15 packages** (including `elm/core`, `elm/browser`, `rtfeldman/elm-css`, `mdgriffith/elm-ui`) with 100% parse, round-trip, and printer idempotency rates.

## Quick start

```rust
use elm_ast_rs::{parse, print};

let source = r#"
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y
"#;

// Parse
let module = parse(source).unwrap();

// Inspect
println!("{} declarations", module.declarations.len());

// Print back to valid Elm
let output = print(&module);
```

## Features

All features are enabled by default via `full`. Disable `default-features` and pick what you need to reduce compile times.

```toml
[dependencies]
elm-ast-rs = "0.1"
```

| Feature | Description |
|---------|-------------|
| `full` | Enables all features below (default) |
| `parsing` | `parse()` and `parse_recovering()` functions |
| `printing` | `print()`, `Display` impls, `Printer` struct |
| `visit` | `Visit` trait for immutable AST traversal |
| `visit-mut` | `VisitMut` trait for in-place AST mutation |
| `fold` | `Fold` trait for owned AST transformation |
| `serde` | `Serialize`/`Deserialize` on all AST types |

### Minimal dependency (AST types only)

```toml
[dependencies]
elm-ast-rs = { version = "0.1", default-features = false }
```

### With serde

```toml
[dependencies]
elm-ast-rs = { version = "0.1", features = ["serde"] }
```

## AST types

Every Elm 0.19.1 syntax construct has a corresponding Rust type:

| Elm construct | Rust type |
|---|---|
| Module header | `ModuleHeader` (Normal, Port, Effect) |
| Imports | `Import` |
| Exposing lists | `Exposing`, `ExposedItem` |
| Type annotations | `TypeAnnotation` (GenericType, Typed, Unit, Tupled, Record, GenericRecord, FunctionType) |
| Patterns | `Pattern` (Anything, Var, Literal, Tuple, Constructor, Record, Cons, List, As, ...) |
| Expressions | `Expr` (22 variants: literals, application, operators, if/case/let, lambda, records, lists, ...) |
| Declarations | `Declaration` (FunctionDeclaration, AliasDeclaration, CustomTypeDeclaration, PortDeclaration, InfixDeclaration) |
| Complete file | `ElmModule` |

All nodes carry source location information via `Spanned<T>`.

## Parsing

```rust
use elm_ast_rs::parse;

// Strict: fails on first error
let module = parse(source)?;

// Recovering: returns partial AST + all errors
let (maybe_module, errors) = elm_ast_rs::parse_recovering(source);
```

## Printing

```rust
use elm_ast_rs::print;

let output = print(&module);

// Or use Display
println!("{module}");
```

The printer produces idempotent output: `print(parse(print(parse(src)))) == print(parse(src))`.

## Visitors

```rust
use elm_ast_rs::visit::{Visit, walk_expr};
use elm_ast_rs::expr::Expr;
use elm_ast_rs::node::Spanned;

struct FunctionCallCounter(usize);

impl Visit for FunctionCallCounter {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if matches!(&expr.value, Expr::Application(_)) {
            self.0 += 1;
        }
        walk_expr(self, expr);
    }
}

let mut counter = FunctionCallCounter(0);
counter.visit_module(&module);
println!("{} function calls", counter.0);
```

Three traversal traits are available:
- **`Visit`** -- immutable traversal (`&` references)
- **`VisitMut`** -- in-place mutation (`&mut` references)
- **`Fold`** -- owned transformation (takes ownership, returns new tree)

## Builder API

Construct AST nodes programmatically (with dummy spans):

```rust
use elm_ast_rs::builder::*;

let m = module(
    vec!["Main"],
    vec![
        func("add", vec![pvar("x"), pvar("y")],
            binop("+", var("x"), var("y"))),
    ],
);

println!("{m}"); // prints valid Elm
```

## Serde

With the `serde` feature enabled, all AST types support JSON serialization:

```rust
let module = elm_ast_rs::parse(source).unwrap();
let json = serde_json::to_string_pretty(&module).unwrap();
let module2: elm_ast_rs::ElmModule = serde_json::from_str(&json).unwrap();
```

## Comment handling

Comments are extracted from the token stream and can be associated with declarations:

```rust
use elm_ast_rs::{Lexer, parse};
use elm_ast_rs::file::{extract_comments, associate_comments};

let module = parse(source).unwrap();
let (tokens, _) = Lexer::new(source).tokenize();
let all_comments = extract_comments(&tokens);
let per_decl = associate_comments(&module, &all_comments);

for (i, comments) in per_decl.iter().enumerate() {
    for c in comments {
        println!("decl {i}: {c}");
    }
}
```

## Architecture

The design follows `syn`'s proven patterns:
- **Enum-of-structs AST** -- each variant wraps a dedicated struct with named fields
- **`Spanned<T>`** -- every node carries a `Span` (byte offset + line/column)
- **`Box<T>`** for recursive sub-expressions
- **Feature-gated modules** for compile-time control

The printer uses an approach inspired by `elm-format`: eagerly detect whether sub-expressions are multi-line, then switch containers (lists, tuples, applications) to vertical layout when any child is multi-line.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
