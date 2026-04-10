# Pre-release TODO

Test coverage gaps and improvements to address before publishing to crates.io.

## 1. Parse error tests

Only 2 error cases are tested (`missing_module_header`, `missing_exposing`), plus 4 error recovery tests. There are ~14 distinct error messages in the parser with no coverage. Users hitting a malformed `.elm` file are relying on untested paths.

Specific untested error messages:
- `expected 'left', 'right', or 'non' in infix declaration`
- `expected operator in infix declaration`
- `expected precedence number in infix declaration`
- `expected ',' or ')' in expression`
- `expected ')' after operator in prefix expression`
- `expected ')' or ','`
- `expected at least one argument in lambda`
- `expected at least one case branch`
- `expected field name after '.'`
- `expected operator in exposing list`
- `expected ',' or ')' in pattern`
- `expected number after '-' in pattern`
- `expected ',' or ')' in type`

Each of these should have a test that feeds the parser a malformed input and asserts it returns an error (not a panic) with a useful message.

## 2. Port and infix declaration parser tests

`port module` header parsing is tested, but `port name : Type` declarations have zero parser unit tests. Infix declarations (`infix left 5 (|=) = keeper`) also have zero unit tests. They work on real files (integration tests pass), but there is no targeted verification of the parsed AST structure.

Need:
- Parse a port declaration, assert it produces `Declaration::PortDeclaration` with correct name and type annotation
- Parse an infix declaration, assert it produces `Declaration::InfixDeclaration` with correct operator, precedence, direction, and function name
- Round-trip tests for both

## 3. `Expr::BinOps` tests

`BinOps` is the raw unresolved operator chain variant in the AST. It exists, is handled by the printer, visitors, and fold, but has zero tests proving any of that works. Need a test that constructs or parses a `BinOps` node and verifies it prints and round-trips correctly.

## 4. GLSL expression tests

GLSL expressions (`[glsl| ... |]`) are only checked in the integration test's equality comparison. No parse test, no print test, no round-trip test. Need:
- Lexer test for GLSL block delimiters
- Parser test that parses a GLSL expression and asserts `Expr::GLSLExpression`
- Printer round-trip test

## 5. Top-level destructuring tests

`Declaration::Destructuring` (e.g., `(a, b) = someTuple`) is only handled in the integration test's equality comparison. Need:
- Parser test that parses a top-level destructuring and asserts the pattern and body
- Printer round-trip test

## 6. Integration test comment verification

`assert_module_eq` in the integration tests checks import and declaration counts but not comment counts. We don't know if any of the 149 real-world files lose comments on round-trip. Add a comment count comparison to `assert_module_eq` or add a dedicated integration test that verifies comment preservation across all fixture files.

## 7. Visitor/Fold trait coverage

19 visitor tests exist, but many trait methods have no override test:
- `visit_comment` / `fold_comment`
- `visit_infix_def` / `fold_infix_def`
- `visit_exposed_item` / `fold_exposed_item`
- `visit_ident` / `fold_ident`
- `fold_literal` for Char, Hex, Float variants (only String is tested)

Each of these should have a test that overrides the method, runs it on appropriate input, and verifies the override was called or the transformation applied.
