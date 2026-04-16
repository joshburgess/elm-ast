# Architecture

This document describes the internal design of `elm-ast`. Read this if you want to understand how the library works, contribute changes, or build tools on top of it.

## Source layout

```
src/
  lib.rs                   Re-exports, feature gates
  span.rs                  Position (line/col/offset), Span (half-open interval)
  node.rs                  Spanned<T>: wraps every AST node with span + comments
  token.rs                 Token enum (lexer output)
  literal.rs               Literal enum (Int, Float, String, Char)
  ident.rs                 Ident, ModuleName aliases; QualifiedName struct
  comment.rs               Comment enum (Line, Block, Doc)
  operator.rs              InfixDirection, InfixDeclaration
  import.rs                Import struct
  exposing.rs              Exposing, ExposedItem enums
  module_header.rs         ModuleHeader (Normal, Port, Effect)
  expr.rs                  Expr enum: 20+ expression variants
  pattern.rs               Pattern enum: destructuring forms
  type_annotation.rs       TypeAnnotation enum: type syntax
  declaration.rs           Declaration enum: top-level items
  file.rs                  ElmModule: root AST node for a .elm file
  lexer.rs                 Lexer struct, tokenization
  parse/
    mod.rs                 Parser struct, ParseError, public parse()/parse_recovering()
    expr.rs                Expression parser (CPS/trampoline + Pratt)
    pattern.rs             Pattern parser
    type_annotation.rs     Type annotation parser
    declaration.rs         Declaration parser
    module.rs              Module header, imports, full file parsing
  print.rs                 Pretty-printer (elm-format-inspired)
  display.rs               Display impls via the printer
  visit.rs                 Visit trait: immutable traversal
  visit_mut.rs             VisitMut trait: in-place mutation
  fold.rs                  Fold trait: owned transformation
  builder.rs               Helpers for programmatic AST construction
  wasm.rs                  WASM bindings via wasm-bindgen
tests/
  parser_tests.rs          Parser unit tests
  printer_tests.rs         Printer unit tests
  edge_case_tests.rs       Edge cases, serde, builders, comments
  property_tests.rs        Property-based tests (proptest)
  integration_tests.rs     291 real-world files from 50 packages
```

## Feature gates

Core AST types (`Expr`, `Pattern`, `Declaration`, etc.) are always available. Everything else is behind feature flags:

| Feature | Modules | Purpose |
|---|---|---|
| `parsing` | `parse/*`, `lexer` | `parse()`, `parse_recovering()` |
| `printing` | `print`, `display` | `print()`, `Display` impls |
| `visit` | `visit` | Immutable AST traversal |
| `visit-mut` | `visit_mut` | In-place AST mutation |
| `fold` | `fold` | Owned AST transformation |
| `serde` | (derives on all types) | `Serialize`/`Deserialize` |
| `wasm` | `wasm` | Browser bindings |

The `full` feature (default) enables `parsing`, `printing`, `visit`, `visit-mut`, and `fold`.

## Core abstraction: `Spanned<T>`

Every AST node is wrapped in `Spanned<T>`:

```rust
pub struct Spanned<T> {
    // source location (start..end)
    pub span: Span,  
    // the actual AST node
    pub value: T,       
    // leading comments
    pub comments: Vec<Spanned<Comment>>,  
}
```

This means source locations and comments are carried everywhere without polluting the node types themselves. `Hash` and `PartialEq` on `Spanned<T>` delegate to the inner value only, ignoring span and comments, so two structurally identical ASTs compare equal regardless of where they appeared in source.

## Lexer

The lexer (`lexer.rs`) produces a flat `Vec<Spanned<Token>>` with no INDENT/DEDENT tokens. This follows the elm/compiler approach: indentation is determined by the parser from token column positions, not from synthetic layout tokens.

Key design choices:
- **Newlines are tokens.** The parser uses `Token::Newline` to track line boundaries for indentation decisions.
- **Comments are tokens.** `LineComment`, `BlockComment`, and `DocComment` are preserved in the token stream. The parser collects them during `skip_whitespace()` and attaches them to AST nodes.
- **Recoverable.** The lexer collects errors and continues scanning rather than aborting on the first problem.
- Supports nestable `{- -}` block comments, multi-line `"""` strings, `[glsl| ... |]` blocks, unicode escapes, hex literals, and custom operators.

## Parser

### Structure

The parser (`parse/mod.rs`) is a cursor over the token stream with lookahead. It tracks:
- `pos`: current position in the token array
- `paren_depth`: nesting depth of `()`/`[]`/`{}`; when > 0, indentation rules are suspended
- `app_context_col`: optional column override for application argument collection inside list/record brackets
- `collected_comments`: comments accumulated during whitespace skipping

### Indentation

Elm's layout rules are column-based. The parser checks whether a token's column is indented past some reference column to decide whether it continues the current construct or starts a new one. Inside parentheses, brackets, and braces, indentation rules are suspended entirely (any column is valid).

### Expression parser: CPS + Pratt + trampoline

This is the most architecturally significant part of the codebase. Traditional recursive-descent expression parsers overflow the call stack on deeply nested input. `elm-ast` eliminates this entirely:

**1. Iterative Pratt parsing** for binary operators. An explicit `Vec<PendingOp>` acts as a heap-allocated operator stack. The `pratt_loop` function iteratively collects operators and operands, comparing binding powers to decide when to reduce, without any recursive calls through precedence levels.

**2. CPS (continuation-passing style)** for compound expressions. Every construct that would normally call `parse_expr` recursively (if/case/let/lambda/paren/tuple/list/record) instead returns `Step::NeedExpr(continuation)`, where the continuation is a closure capturing the partial parse state. For example, when parsing `if cond then ...`, after seeing `if`, the parser returns a continuation that says "I need the condition expression; when you give it to me, I'll continue parsing the `then` branch."

**3. Trampoline loop** drives execution. The top-level `parse_expr` function runs a loop: when a compound form needs a sub-expression, its continuation is pushed onto a `Vec<Cont>` and the loop restarts. When a sub-expression completes, the continuation is popped and invoked.

```rust
type Cont = Box<dyn FnOnce(&mut Parser, Spanned<Expr>) -> ParseResult<Step>>;

enum Step {
    Done(Spanned<Expr>),
    NeedExpr(Cont),
}
```

This guarantees O(1) call-stack depth regardless of expression nesting. The continuation stack is bounded by `MAX_EXPR_DEPTH` (256) as a resource guard.

### Application column checking

Function application arguments are collected by `application_loop`, which checks that each argument's column is indented past the function's column (on subsequent lines). Inside list and record literals, this check is relaxed: the `app_context_col` field lets the list/record parser set the opening bracket's column as the reference point instead. This is consumed (via `.take()`) at the entry to `parse_application_cps` and threaded through CPS closures to prevent leaking into nested expressions. Case and let parsers clear it before parsing branch/declaration bodies.

### Error recovery

`parse_recovering()` returns a partial AST alongside errors. When a declaration fails to parse, the parser skips tokens until it finds the start of the next top-level declaration (column 1, matching a declaration-starting token) and continues.

## Printer

The printer (`print.rs`) produces elm-format-style output. The core decision function is `is_multiline(expr)`:

- **Block expressions** (case, if, let, lambda) are always multiline.
- **Containers** (list, tuple, record, application, operator chains) are multiline if any child is multiline.
- **Atoms** (literals, identifiers, unit) are never multiline.

When a container is multiline, it switches to vertical layout (one element per line, indented). When it's single-line, elements are space/comma-separated on one line.

Key formatting rules:
- **Vertical application layout**: when any non-function argument is multiline, each argument goes on its own indented line.
- **Multiline record setters**: when a record field value is multiline, the value goes on a new indented line after `=`.
- **Block expressions in atomic position**: case/if/let as function arguments get parenthesized with the closing `)` on its own line.

The printer is idempotent: `print(parse(print(parse(src)))) == print(parse(src))`. This is verified across all 291 test fixture files.

## Traversal traits

Three traits provide different traversal strategies:

| Trait | Borrows | Use case |
|---|---|---|
| `Visit` | `&Spanned<T>` | Read-only analysis (lint rules, search, dependency collection) |
| `VisitMut` | `&mut Spanned<T>` | In-place modification (rename, qualify imports) |
| `Fold` | `Spanned<T>` (owned) | Produce a new AST by consuming the old one |

Each trait has one method per AST node type (e.g., `visit_expr`, `visit_pattern`) with a default implementation that calls the corresponding `walk_*` function for recursive descent. Override specific methods to intercept nodes of interest.

## Test organization

Tests are layered from unit to integration:

1. **Parser/printer unit tests**: isolated snippets testing specific syntax constructs
2. **Edge case tests**: serde round-trips, builder API, comment handling
3. **Property tests**: proptest-generated random ASTs verify parse/print invariants
4. **Integration tests**: 291 real `.elm` files from 50 packages verify parse, round-trip (parse -> print -> parse produces structurally equal AST), and printer idempotency (print -> parse -> print produces identical text)

### Regression watchlist

Two files were historically the hardest to handle and are documented in `tests/integration_tests.rs`:

- **typed-svg/GradientsPatterns.elm**: required `app_context_col` to relax column checks inside brackets
- **elm-animator/Animator.elm**: required vertical application layout and multiline record setter formatting in the printer
