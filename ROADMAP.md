# elm-ast-rs Roadmap

A `syn`-quality Rust library for parsing and constructing Elm 0.19.1 ASTs.

## Status: Feature-complete

All planned phases are complete. The library and tool suite are production-ready.

## Completed

### Phase 1: AST Types

Complete typed AST covering all Elm 0.19.1 syntax: `Span`, `Spanned<T>`,
`Ident`, `ModuleHeader`, `Import`, `Exposing`, `Expr` (22 variants),
`Pattern`, `TypeAnnotation`, `Declaration`, `Literal`, `Comment`, `ElmModule`.

### Phase 2: Lexer

Full tokenizer with nestable block comments, multi-line strings, GLSL blocks,
unicode escapes, hex literals, custom operators (`|=`, `|.`), and newline
tracking for indentation-sensitive layout.

### Phase 3: Parser

Pratt parser for operator precedence, indentation-aware layout with paren-context
suspension, error recovery via `parse_recovering()`, exact column matching for
nested case expressions inside parens.

### Phase 4: Printer

elm-format-inspired multiline detection: `is_multiline()` eagerly checks if
sub-expressions would be multi-line, containers switch to vertical layout when
any child is multi-line. 100% idempotent on 149 real-world files.

### Phase 5: Visitors

`Visit` (immutable), `VisitMut` (in-place mutation), `Fold` (owned transformation)
traits with one method per AST node type and public `walk_*` default descent functions.

### Phase 6: Testing and Hardening

149 source files from 23 packages: 100% parse, 100% round-trip with deep AST
equality, 100% printer idempotency. Criterion benchmarks for lex/parse/print.

### Phase 7: Ecosystem

- Feature gates: `parsing`, `printing`, `visit`, `visit-mut`, `fold`, `serde`, `wasm`
- serde: Serialize/Deserialize on all 28 AST types
- WASM: wasm-bindgen bindings, builds for wasm32-unknown-unknown
- Builder API for programmatic AST construction
- Display impls for all AST types
- Comment extraction and declaration association
- Error recovery with `parse_recovering()`
- GitHub Actions CI
- MIT + Apache-2.0 licenses

### Phase 8: Tool Suite

Five standalone CLI tools built on elm-ast-rs:

- **elm-unused** — project-wide dead code detection (unused imports, functions,
  exports, constructors, types)
- **elm-lint** — 14 built-in lint rules (NoDebug, NoUnusedImports,
  NoMissingTypeAnnotation, NoBooleanCase, etc.)
- **elm-deps** — module dependency graphs (DOT/Mermaid output), circular
  dependency detection, coupling metrics
- **elm-refactor** — cross-file rename, sort-imports, qualify-imports
- **elm-search** — semantic AST-aware code search with 10 query types
  (returns, case-on, calls, unused-args, lambda arity, etc.)

## Test coverage

263 tests across the workspace. 149 real-world Elm files from 23 packages.

## Future possibilities

These are not planned but would be natural extensions:

- **Fuzzing** — property-based testing with `cargo fuzz`
- **elm-api-diff** — compare package API surfaces between versions
- **elm-stats** — project metrics dashboard (LOC, complexity, module sizes)
- **Auto-fix** — lint rules that can automatically fix their findings
- **LSP integration** — language server protocol features using elm-ast-rs
- **Incremental parsing** — reparse only changed regions for editor integration
