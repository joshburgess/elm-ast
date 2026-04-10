# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-10

Initial release.

### Added

- Complete strongly-typed AST for all Elm 0.19.1 syntax constructs
- `Spanned<T>` wrapper carrying source location (`Span`) on every node
- Lexer with support for nestable block comments, multi-line strings, GLSL blocks, unicode escapes, hex literals, and custom operators
- Fully iterative expression parser with zero stack recursion: iterative Pratt parsing for operators, CPS continuations for compound expressions, and a trampoline loop — O(1) call-stack depth regardless of nesting
- Indentation-aware layout and error recovery (`parse_recovering()`)
- Pretty-printer producing idempotent, elm-format-style output with comment round-tripping (top-level, `let`/`in`, `case`/`of`)
- `Visit` trait for immutable AST traversal
- `VisitMut` trait for in-place AST mutation
- `Fold` trait for owned AST transformation
- Builder API for programmatic AST construction
- `Display` impls for all AST types
- Feature gates: `parsing`, `printing`, `visit`, `visit-mut`, `fold`, `serde`, `wasm`
- `serde` support (`Serialize`/`Deserialize`) for all 28 AST types
- WASM bindings via `wasm-bindgen`
- 373 tests across the workspace, including integration tests against 149 real-world `.elm` files from 23 packages

### Tool suite

- **elm-unused** -- project-wide dead code detection
- **elm-lint** -- 14 built-in lint rules
- **elm-deps** -- module dependency graphs, cycle detection, coupling metrics
- **elm-refactor** -- cross-file rename, sort/qualify imports
- **elm-search** -- semantic AST-aware code search with 10 query types

[0.1.0]: https://github.com/joshburgess/elm-ast/releases/tag/v0.1.0
