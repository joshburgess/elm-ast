# elm-ast

Inspired by [`syn`](https://github.com/dtolnay/syn), `elm-ast` is a Rust library for parsing and constructing Elm 0.19.1 ASTs.

## Overview

`elm-ast` provides a complete, strongly-typed representation of Elm source code as a Rust AST, along with a parser, printer, and visitor/fold traits for traversal and transformation. It is modeled after Rust's [`syn`](https://github.com/dtolnay/syn) crate, with a formatting approach inspired by [`elm-format`](https://github.com/avh4/elm-format).

**Tested against 291 real-world `.elm` files from 50 packages** (including `elm/core`, `elm/browser`, `rtfeldman/elm-css`, `mdgriffith/elm-ui`, `dillonkearns/elm-markdown`, `folkertdev/elm-flate`, `elm-explorations/test`) with 100% parse, round-trip, and printer idempotency rates.

## Quick start

```rust
use elm_ast::{parse, print};

let source = r#"
module Main exposing (..)

add : Int -> Int -> Int
add x y = x + y
"#;

// Parse
let module = parse(source)?;

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

Comments inside `let`/`in` blocks and `case`/`of` branches are attached to their respective AST nodes and round-trip correctly. Doc comments (`{-| -}`) are attached to their declarations and always round-trip correctly.

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
let module = elm_ast::parse(source)?;
let json = serde_json::to_string_pretty(&module)?;
let module2: elm_ast::ElmModule = serde_json::from_str(&json)?;
```

## Architecture

The design follows `syn`'s proven patterns:
- **Enum-of-structs AST** -- each variant wraps a dedicated struct with named fields
- **`Spanned<T>`** -- every node carries a `Span` (byte offset + line/column)
- **`Box<T>`** for recursive sub-expressions
- **Feature-gated modules** for compile-time control

The printer uses an approach inspired by [`elm-format`](https://github.com/avh4/elm-format): eagerly detect whether sub-expressions are multi-line, then switch containers to vertical layout when any child is multi-line.

### Fully iterative expression parser

The expression parser uses **zero stack recursion**. Traditional recursive-descent parsers can overflow the call stack on deeply nested input. `elm-ast` eliminates this entirely through three techniques:

1. **Iterative Pratt parsing** -- binary operators use an explicit `Vec<PendingOp>` heap-allocated operator stack instead of recursive descent through precedence levels.
2. **CPS (continuation-passing style)** -- every compound expression (if/case/let/lambda/paren/tuple/list/record) that would normally call `parse_expr` recursively instead returns a `NeedExpr(continuation)` step, where the continuation is a closure capturing the partial parse state.
3. **Trampoline loop** -- a top-level loop drives execution: when a compound form needs a sub-expression, its continuation is pushed onto a heap-allocated stack and the loop restarts. When a sub-expression completes, the continuation is popped and invoked.

This guarantees **O(1) call-stack depth** regardless of expression nesting. The continuation stack is bounded by `MAX_EXPR_DEPTH` (256) as a resource guard, not a safety requirement.

None of this was strictly necessary -- a simple depth limit would have sufficed -- but it was fun to build, and, most importantly, it is thoroughly tested and works.

## Test coverage

379 tests (366 native + 13 WASM):

| Suite | Tests |
|---|---|
| Lexer | 59 |
| Parser | 109 |
| Printer | 55 |
| Visitors | 29 |
| Edge cases + serde + builders + comments | 104 |
| Property-based (proptest) | 5 |
| Integration (291 real files, 50 packages) | 3 |
| WASM bindings (wasm-pack) | 13 |

## License

Dual licensed under [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT).
