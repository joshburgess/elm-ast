# elm-ast

A `syn`-quality Rust library for parsing and constructing Elm 0.19.1 ASTs, plus a suite of developer tools built on top.

## Overview

`elm-ast` provides a complete, strongly-typed representation of Elm source code as a Rust AST, along with a parser, printer, and visitor/fold traits for traversal and transformation. It is modeled after Rust's [`syn`](https://github.com/dtolnay/syn) crate, with a formatting approach inspired by [`elm-format`](https://github.com/avh4/elm-format).

**Tested against 149 real-world `.elm` files from 23 packages** (including `elm/core`, `elm/browser`, `rtfeldman/elm-css`, `mdgriffith/elm-ui`, `elm-explorations/test`) with 100% parse, round-trip, and printer idempotency rates.

## Tool suite

Built on `elm-ast`, five standalone CLI tools for Elm development:

| Tool | Description | Speed |
|---|---|---|
| [**`elm-unused`**](tools/elm-unused/) | Project-wide dead code detection | 10ms / 26 files |
| [**`elm-lint`**](tools/elm-lint/) | 14 built-in lint rules | 7ms / 26 files |
| [**`elm-deps`**](tools/elm-deps/) | Dependency graphs, cycle detection, coupling metrics | 18ms / 13 files |
| [**`elm-refactor`**](tools/elm-refactor/) | Cross-file rename, sort/qualify imports | 7ms / 18 files |
| [**`elm-search`**](tools/elm-search/) | Semantic AST-aware code search (10 query types) | 3ms / 18 files |

### Quick examples

```bash
# Find dead code
cargo run -p elm-unused -- src

# Lint with all 14 rules
cargo run -p elm-lint -- src

# Visualize module dependencies
cargo run -p elm-deps -- --mermaid src

# Rename a function across all files
cargo run -p elm-refactor -- rename Main.oldName oldName newName src

# Find all functions returning Maybe
cargo run -p elm-search -- --dir src returns Maybe

# Find unused function arguments
cargo run -p elm-search -- --dir src unused-args
```

## Library quick start

```rust
use elm_ast::{parse, print};

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
elm-ast = "0.1"
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
| `wasm` | WASM bindings via `wasm-bindgen` |

### Minimal dependency (AST types only)

```toml
[dependencies]
elm-ast = { version = "0.1", default-features = false }
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
use elm_ast::parse;

// Strict: fails on first error
let module = parse(source)?;

// Recovering: returns partial AST + all errors
let (maybe_module, errors) = elm_ast::parse_recovering(source);
```

## Printing

```rust
use elm_ast::print;

let output = print(&module);

// Or use Display
println!("{module}");
```

The printer produces idempotent output: `print(parse(print(parse(src)))) == print(parse(src))`.

### Comment preservation

Top-level comments (line comments `--` and block comments `{- -}` between declarations) are captured during parsing and round-tripped through the printer. Comments are placed immediately before the declaration they precede.

**Limitations:** Comments inside expressions (e.g., within a `let` block or on a specific line of a case branch) are not yet preserved. Doc comments (`{-| -}`) are attached to their declarations and always round-trip correctly.

## Visitors

```rust
use elm_ast::visit::{Visit, walk_expr};
use elm_ast::expr::Expr;
use elm_ast::node::Spanned;

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

Three traversal traits:
- **`Visit`** -- immutable traversal (`&` references)
- **`VisitMut`** -- in-place mutation (`&mut` references)
- **`Fold`** -- owned transformation (takes ownership, returns new tree)

## Builder API

Construct AST nodes programmatically (with dummy spans):

```rust
use elm_ast::builder::*;

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

With the `serde` feature, all AST types support JSON serialization:

```rust
let module = elm_ast::parse(source).unwrap();
let json = serde_json::to_string_pretty(&module).unwrap();
let module2: elm_ast::ElmModule = serde_json::from_str(&json).unwrap();
```

## Architecture

The design follows `syn`'s proven patterns:
- **Enum-of-structs AST** -- each variant wraps a dedicated struct with named fields
- **`Spanned<T>`** -- every node carries a `Span` (byte offset + line/column)
- **`Box<T>`** for recursive sub-expressions
- **Feature-gated modules** for compile-time control

The printer uses an approach inspired by [`elm-format`](https://github.com/avh4/elm-format): eagerly detect whether sub-expressions are multi-line, then switch containers to vertical layout when any child is multi-line.

## Test coverage

350 tests across the workspace:

| Suite | Tests |
|---|---|
| Lexer | 59 |
| Parser | 71 |
| Printer | 42 |
| Visitors | 29 |
| Edge cases + serde + builders + comments | 78 |
| Integration (149 real files, 23 packages) | 3 |
| elm-unused | 5 |
| elm-lint | 25 |
| elm-deps | 8 |
| elm-refactor | 10 |
| elm-search | 21 |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
