# Pre-release TODO

Test coverage gaps and improvements to address before publishing to crates.io.

## 1. Parse error tests ✅

Added 13 error tests covering all distinct error messages in the parser:
- Infix direction, operator, and precedence errors
- Expression comma/rparen/bracket errors
- Prefix operator rparen error
- Lambda argument and case branch errors
- Field name after dot error
- Operator in exposing list error
- Pattern comma/rparen and negative number errors
- Type comma/rparen error
- 2 error recovery tests verifying partial AST recovery

## 2. Port and infix declaration parser tests ✅

- Port declaration: parse and verify `Declaration::PortDeclaration` with correct name and type annotation
- Port declaration round-trip with two ports
- Infix declaration: left, right, and non associativity tested, verifying operator, precedence, direction, function
- Infix declaration round-trip

## 3. `Expr::BinOps` tests ✅

- Construction and print test (verifies BinOps nodes print without stack overflow)
- Visitor traversal test (verifies Visit descends into all BinOps operands)
- Fold traversal test (verifies Fold transforms literals inside BinOps)
- Fixed infinite recursion bug in printer for BinOps nodes (added explicit BinOps arm in `write_expr_inner`)

## 4. GLSL expression tests ✅

- Lexer test: verifies `[glsl| ... |]` produces `Token::Glsl`
- Parser test: parses GLSL expression and asserts `Expr::GLSLExpression` with shader content
- Printer round-trip test: verifies `[glsl|` and `|]` survive round-trip

## 5. Top-level destructuring tests ✅

- Tuple destructuring: `( a, b ) = someTuple` → `Declaration::Destructuring` with tuple pattern
- Record destructuring: `{ name, age } = person` → `Declaration::Destructuring` with record pattern
- Round-trip test

## 6. Integration test comment verification ✅

Added comment count comparison to `assert_module_eq`. All 149 real-world files now verify that comment counts survive round-trip.

## 7. Visitor/Fold trait coverage ✅

- `visit_comment`: collects line and block comments from parsed modules
- `visit_infix_def`: collects operators from infix declarations
- `visit_exposed_item`: collects exposed items from module header and imports
- `visit_ident`: verifies port names are visited
- `fold_comment`: transforms comment text
- `fold_infix_def`: transforms infix definition function names
- `fold_literal` for Char, Float, Hex variants (in addition to existing String/Int tests)
