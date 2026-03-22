# elm-ast-rs Roadmap

A `syn`-quality Rust library for parsing and constructing Elm 0.19.1 ASTs.

## Vision

Fill the gap: there is no current, complete, idiomatic Rust crate that parses
Elm 0.19.1 into a strongly-typed AST. `elm-ast-rs` aims to be the definitive
Rust library for working with Elm source code — parsing, analyzing,
transforming, and emitting it.

## Architecture

Following `syn`'s proven patterns:

- **Enum-of-structs AST** — each enum variant holds a dedicated struct with
  named fields
- **`Span` on every node** — source locations for precise error reporting
- **`Parse` trait** — uniform `fn parse(input: &mut Parser) -> Result<Self>`
  so every node is composable and independently parseable
- **`Print` trait** — round-trip capability (parse → transform → emit valid Elm)
- **`Visit` / `VisitMut` / `Fold` traits** — one method per AST node for
  traversal and transformation
- **Feature flags** — gate `full`, `parsing`, `printing`, `visit`, `fold` to
  control compile times
- **`Box<T>`** for recursive sub-expressions to keep enum sizes manageable

## Key References

| Source | Purpose |
|---|---|
| `elm/compiler` `AST/Source.hs` | Definitive AST type catalog |
| `elm/compiler` `Parse/` | Indentation-sensitive parsing reference |
| `stil4m/elm-syntax` | Well-documented Elm-in-Elm AST cross-reference |
| `tree-sitter-elm` | Formal grammar for precedence/associativity |
| Rust `syn` crate | Architectural template |

## Phases

### Phase 1: AST Types (current)

Define the complete typed AST covering all Elm 0.19.1 syntax:

- [x] `Span`, `Position` — source location types
- [x] `Spanned<T>` — the `Node` wrapper carrying span + value
- [x] `Ident` — identifiers (lower, upper, operator, qualified)
- [x] `Module` — module header (normal, port, effect)
- [x] `Import` — import declarations
- [x] `Exposing` — exposing lists and exposed items
- [x] `Expression` — all expression forms
- [x] `Pattern` — all pattern forms
- [x] `TypeAnnotation` — all type annotation forms
- [x] `Declaration` — top-level declarations (functions, types, aliases, ports, infix)
- [x] `Literal` — char, string, int, float literals
- [x] `Operator` — operator metadata, precedence, associativity
- [x] `Comment` — single-line, multi-line, doc comments
- [x] `ElmModule` (file) — the root node tying everything together

### Phase 2: Lexer / Tokenizer

- [ ] Token types for all Elm lexemes
- [ ] Indentation tracking (emit virtual INDENT/DEDENT/NEWLINE tokens)
- [ ] Nestable multi-line comment handling (`{- {- -} -}`)
- [ ] String literal handling (single-line `"..."`, multi-line `"""..."""`)
- [ ] GLSL block handling (`[glsl| ... |]`)
- [ ] Span attachment to every token

### Phase 3: Parser

- [ ] `Parse` trait definition
- [ ] `ParseStream` / cursor abstraction
- [ ] Module header parsing
- [ ] Import parsing
- [ ] Declaration parsing
- [ ] Expression parsing with Pratt parser for operator precedence
- [ ] Pattern parsing
- [ ] Type annotation parsing
- [ ] Indentation-sensitive block parsing (let/in, case/of)
- [ ] Error recovery and reporting with spans

### Phase 4: Printer / Code Generation

- [ ] `Print` trait definition
- [ ] Pretty-printer with configurable formatting
- [ ] Round-trip fidelity tests (parse → print → parse = same AST)
- [ ] `quote!`-style macro for constructing Elm AST from quasi-quoted Elm code

### Phase 5: Visitors and Transformations

- [ ] `Visit` trait — immutable traversal
- [ ] `VisitMut` trait — mutable in-place traversal
- [ ] `Fold` trait — owned transformation
- [ ] Code-generate all visitor methods from AST schema
- [ ] Builder / convenience constructors for AST nodes

### Phase 6: Testing and Hardening

- [ ] Parse every `.elm` file in `elm/core`
- [ ] Parse every `.elm` file in `elm/browser`, `elm/html`, `elm/json`, `elm/http`
- [ ] Parse popular community packages
- [ ] Round-trip tests on all of the above
- [ ] Fuzzing
- [ ] Benchmarks

### Phase 7: Ecosystem

- [ ] `elm-ast-rs-codegen` — machine-readable AST schema (like `syn-codegen`)
- [ ] serde support for AST serialization/deserialization
- [ ] WASM target for browser-based Elm tooling
- [ ] Example tools: formatter, linter, dead code detector

## Engineering Challenges

1. **Indentation-sensitive parsing** — Elm uses significant whitespace. The
   canonical parser handles this with indentation context tracking. We need
   either indentation-aware combinators or a two-pass lexer approach.

2. **Operator precedence** — Elm operators have user-definable precedence and
   associativity via `infix` declarations. The parser must handle this
   dynamically (Pratt parsing).

3. **Round-trip fidelity** — Preserving comments, whitespace, and formatting
   through parse → transform → print cycles requires careful span and
   trivia tracking.

4. **GLSL blocks** — `[glsl| ... |]` embeds raw shader code. Must be lexed
   as opaque content.

5. **Effect modules** — `effect module` syntax is internal to elm/core but
   must be parseable for completeness.
