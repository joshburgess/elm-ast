use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, LetDeclaration, RecordSetter, Signature,
};
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::token::Token;

use super::pattern::parse_pattern;
use super::type_annotation::parse_type;
use super::{ParseResult, Parser};

/// Parse an expression.
///
/// This is the top-level expression parser. It handles binary operators
/// with Pratt parsing.
pub fn parse_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    parse_binary_expr(p, 0)
}

/// Pratt parser for binary operators.
///
/// Elm's operators have precedences 0–9. We use the standard Pratt technique
/// where `min_bp` is the minimum binding power (precedence) for this call.
fn parse_binary_expr(p: &mut Parser, min_bp: u8) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    let mut left = parse_unary_expr(p)?;

    loop {
        p.skip_whitespace();

        let (op, prec, assoc) = match extract_operator(p) {
            Some(info) => info,
            None => break,
        };

        let (left_bp, right_bp) = binding_power(prec, &assoc);
        if left_bp < min_bp {
            break;
        }

        p.advance(); // consume the operator token

        let right = parse_binary_expr(p, right_bp)?;

        let expr = Expr::OperatorApplication {
            operator: op,
            direction: assoc,
            left: Box::new(left),
            right: Box::new(right),
        };
        left = p.spanned_from(start, expr);
    }

    Ok(left)
}

/// Extract operator info from current token if it's a binary operator.
/// Returns (operator_name, precedence, associativity).
fn extract_operator(p: &Parser) -> Option<(String, u8, InfixDirection)> {
    match p.peek() {
        Token::Operator(op) => {
            let (prec, assoc) = operator_info(op);
            Some((op.clone(), prec, assoc))
        }
        Token::Minus => Some(("-".into(), 6, InfixDirection::Left)),
        // `|>` and `<|` are stored as Operator, so they're covered above.
        _ => None,
    }
}

/// Get the precedence and associativity of a known Elm operator.
///
/// Based on the Elm 0.19 default operator table:
/// ```text
/// infixr 0 (<|)
/// infixl 0 (|>)
/// infixr 2 (||)
/// infixr 3 (&&)
/// infix  4 (==) (/=) (<) (>) (<=) (>=)
/// infixr 5 (::) (++)
/// infixl 6 (+) (-)
/// infixl 7 (*) (/) (//)
/// infixr 8 (^)
/// infixl 9 (<<)
/// infixr 9 (>>)
/// ```
fn operator_info(op: &str) -> (u8, InfixDirection) {
    match op {
        "<|" => (0, InfixDirection::Right),
        "|>" => (0, InfixDirection::Left),
        "||" => (2, InfixDirection::Right),
        "&&" => (3, InfixDirection::Right),
        "==" | "/=" | "<" | ">" | "<=" | ">=" => (4, InfixDirection::Non),
        "::" | "++" => (5, InfixDirection::Right),
        "+" | "-" => (6, InfixDirection::Left),
        "*" | "/" | "//" => (7, InfixDirection::Left),
        "^" => (8, InfixDirection::Right),
        "<<" => (9, InfixDirection::Left),
        ">>" => (9, InfixDirection::Right),
        // Unknown operators default to precedence 9, left-associative.
        _ => (9, InfixDirection::Left),
    }
}

/// Convert (precedence, associativity) to (left_bp, right_bp) for Pratt parsing.
fn binding_power(prec: u8, assoc: &InfixDirection) -> (u8, u8) {
    // Pratt binding power: multiply by 2 to leave room for left/right distinction.
    let base = prec * 2;
    match assoc {
        InfixDirection::Left => (base, base + 1),
        InfixDirection::Right => (base, base),
        InfixDirection::Non => (base, base + 1), // treat as left for parsing, reject double use later
    }
}

/// Parse a unary expression (prefix negation or function application).
fn parse_unary_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    p.skip_whitespace();
    let start = p.current_pos();

    // Prefix negation: `-expr`
    if matches!(p.peek(), Token::Minus) {
        // Check if this is prefix negation (no space before the operand, or at start).
        // For now, treat `-` followed by an expression as negation in prefix position.
        let minus_span = p.peek_span();
        p.advance();

        // If the next token is immediately adjacent (no whitespace gap), it's negation.
        // Otherwise, it's the binary minus operator handled in parse_binary_expr.
        // Since we already consumed it here in unary position, parse it as negation.
        let operand = parse_application(p)?;
        let expr = Expr::Negation(Box::new(operand));
        return Ok(Spanned::new(minus_span.merge(p.span_from(start)), expr));
    }

    parse_application(p)
}

/// Parse function application: `f x y z`
///
/// In Elm, juxtaposition is function application. `f a b` = `((f a) b)`.
/// The function and all arguments must be atomic expressions.
fn parse_application(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    let first = parse_atomic_expr(p)?;
    let first_line = first.span.start.line;

    let mut args = vec![first];

    loop {
        p.skip_whitespace();
        if !can_start_atomic_expr(p.peek()) {
            break;
        }
        // Application arguments should be on the same line or indented past the function.
        let arg_col = p.current_column();
        let arg_line = p.current_pos().line;
        if arg_line != first_line && arg_col <= start.column {
            break;
        }
        args.push(parse_atomic_expr(p)?);
    }

    if args.len() == 1 {
        Ok(args.into_iter().next().unwrap())
    } else {
        let expr = Expr::Application(args);
        Ok(p.spanned_from(start, expr))
    }
}

/// Parse an atomic expression (the highest-precedence, non-recursive forms).
fn parse_atomic_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    p.skip_whitespace();
    let start = p.current_pos();

    match p.peek().clone() {
        // ── Literals ─────────────────────────────────────────────
        Token::Literal(lit) => {
            p.advance();
            Ok(p.spanned_from(start, Expr::Literal(lit)))
        }

        // ── Lowercase name (value reference) ─────────────────────
        Token::LowerName(name) => {
            p.advance();
            Ok(p.spanned_from(
                start,
                Expr::FunctionOrValue {
                    module_name: Vec::new(),
                    name,
                },
            ))
        }

        // ── Uppercase name (constructor or qualified ref) ────────
        Token::UpperName(_) => {
            let (module_name, name) = parse_qualified_value(p)?;
            Ok(p.spanned_from(start, Expr::FunctionOrValue { module_name, name }))
        }

        // ── Prefix operator: `(+)`, `(::)` ──────────────────────
        // ── Unit: `()` ───────────────────────────────────────────
        // ── Tuple: `(a, b)` ──────────────────────────────────────
        // ── Parenthesized: `(expr)` ──────────────────────────────
        Token::LeftParen => {
            p.advance(); // consume `(`
            p.skip_whitespace();

            // Unit: `()`
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(p.spanned_from(start, Expr::Unit));
            }

            // Prefix operator: `(+)`, `(::)`, `(-)`
            match p.peek().clone() {
                Token::Operator(op) => {
                    p.advance();
                    p.skip_whitespace();
                    if matches!(p.peek(), Token::RightParen) {
                        p.advance();
                        return Ok(p.spanned_from(start, Expr::PrefixOperator(op)));
                    }
                    // Not a prefix op, backtrack isn't possible easily.
                    // This shouldn't happen in valid Elm.
                    return Err(p.error("expected `)` after operator in prefix expression"));
                }
                Token::Minus => {
                    p.advance();
                    p.skip_whitespace();
                    if matches!(p.peek(), Token::RightParen) {
                        p.advance();
                        return Ok(p.spanned_from(start, Expr::PrefixOperator("-".into())));
                    }
                    // It was negation inside parens: `(-expr)`
                    // We need to parse the rest as a negated expression.
                    let operand = parse_expr(p)?;
                    let neg = Expr::Negation(Box::new(operand));
                    let neg_spanned = p.spanned_from(start, neg);
                    p.skip_whitespace();

                    return match p.peek() {
                        Token::RightParen => {
                            p.advance();
                            Ok(p.spanned_from(start, Expr::Parenthesized(Box::new(neg_spanned))))
                        }
                        Token::Comma => {
                            let mut elements = vec![neg_spanned];
                            while p.eat(&Token::Comma) {
                                elements.push(parse_expr(p)?);
                            }
                            p.expect(&Token::RightParen)?;
                            Ok(p.spanned_from(start, Expr::Tuple(elements)))
                        }
                        _ => Err(p.error("expected `)` or `,`")),
                    };
                }
                _ => {}
            }

            // Regular parenthesized expression or tuple.
            let first = parse_expr(p)?;
            p.skip_whitespace();

            match p.peek() {
                Token::Comma => {
                    let mut elements = vec![first];
                    while p.eat(&Token::Comma) {
                        elements.push(parse_expr(p)?);
                    }
                    p.expect(&Token::RightParen)?;
                    Ok(p.spanned_from(start, Expr::Tuple(elements)))
                }
                Token::RightParen => {
                    p.advance();
                    Ok(p.spanned_from(start, Expr::Parenthesized(Box::new(first))))
                }
                _ => Err(p.error("expected `,` or `)` in expression")),
            }
        }

        // ── Record or record update ──────────────────────────────
        Token::LeftBrace => parse_record_expr(p),

        // ── List ─────────────────────────────────────────────────
        Token::LeftBracket => {
            p.advance(); // consume `[`
            p.skip_whitespace();

            // Check for GLSL in case lexer didn't catch it.
            if matches!(p.peek(), Token::RightBracket) {
                p.advance();
                return Ok(p.spanned_from(start, Expr::List(Vec::new())));
            }

            let mut elements = Vec::new();
            elements.push(parse_expr(p)?);
            while p.eat(&Token::Comma) {
                elements.push(parse_expr(p)?);
            }
            p.expect(&Token::RightBracket)?;
            Ok(p.spanned_from(start, Expr::List(elements)))
        }

        // ── GLSL block ──────────────────────────────────────────
        Token::Glsl(src) => {
            p.advance();
            Ok(p.spanned_from(start, Expr::GLSLExpression(src)))
        }

        // ── If-then-else ─────────────────────────────────────────
        Token::If => parse_if_expr(p),

        // ── Case-of ──────────────────────────────────────────────
        Token::Case => parse_case_expr(p),

        // ── Let-in ───────────────────────────────────────────────
        Token::Let => parse_let_expr(p),

        // ── Lambda ───────────────────────────────────────────────
        Token::Backslash => parse_lambda_expr(p),

        // ── Record access function: `.name` ──────────────────────
        Token::Dot => {
            p.advance(); // consume `.`
            match p.peek().clone() {
                Token::LowerName(name) => {
                    p.advance();
                    Ok(p.spanned_from(start, Expr::RecordAccessFunction(name)))
                }
                _ => Err(p.error("expected field name after `.`")),
            }
        }

        _ => Err(p.error(format!(
            "expected expression, found {}",
            super::describe(p.peek())
        ))),
    }
    .and_then(|expr| parse_record_access_chain(p, expr))
}

/// After parsing an atomic expression, check for `.field` access chains.
fn parse_record_access_chain(
    p: &mut Parser,
    mut expr: Spanned<Expr>,
) -> ParseResult<Spanned<Expr>> {
    // Don't skip whitespace — `.field` must be immediately adjacent.
    while matches!(p.peek(), Token::Dot) {
        // Check that the dot is immediately adjacent (same line, no gap).
        let dot_end = p.peek_span().start;
        let expr_end = expr.span.end;
        if dot_end.offset != expr_end.offset {
            break;
        }

        p.advance(); // consume `.`

        match p.peek().clone() {
            Token::LowerName(name) => {
                let start = expr.span.start;
                let name_tok = p.advance();
                let field = Spanned::new(name_tok.span, name);
                expr = p.spanned_from(
                    start,
                    Expr::RecordAccess {
                        record: Box::new(expr),
                        field,
                    },
                );
            }
            _ => {
                return Err(p.error("expected field name after `.`"));
            }
        }
    }
    Ok(expr)
}

// ── Compound expressions ─────────────────────────────────────────────

fn parse_if_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    p.expect(&Token::If)?;

    let mut branches = Vec::new();
    let condition = parse_expr(p)?;
    p.expect(&Token::Then)?;
    let then_branch = parse_expr(p)?;
    branches.push((condition, then_branch));

    p.expect(&Token::Else)?;

    // Chained if-else:  `else if ... then ... else ...`
    p.skip_whitespace();
    let else_branch = if matches!(p.peek(), Token::If) {
        // Recurse: the `else if` becomes a nested if-else expression.
        // But we flatten into branches for our AST.
        parse_if_expr(p)?
    } else {
        parse_expr(p)?
    };

    Ok(p.spanned_from(
        start,
        Expr::IfElse {
            branches,
            else_branch: Box::new(else_branch),
        },
    ))
}

fn parse_case_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    p.expect(&Token::Case)?;
    let subject = parse_expr(p)?;
    p.expect(&Token::Of)?;

    // Parse branches. Each branch: `pattern -> expr`
    // Branches must be at the same indentation level, indented past `case`.
    p.skip_whitespace();
    let branch_col = p.current_column();

    let mut branches = Vec::new();
    loop {
        p.skip_whitespace();
        if p.is_eof() {
            break;
        }
        let col = p.current_column();
        if p.in_paren_context() {
            // Inside parens, branches must be at the SAME column as the first
            // branch. This allows the paren context to relax outer indentation
            // while still correctly separating nested case expressions.
            if !branches.is_empty() && col != branch_col {
                break;
            }
        } else {
            // Outside parens, branches must be at or past the first branch's column.
            if col < branch_col {
                break;
            }
            // A branch at or before the `case` keyword is a new declaration.
            if col < start.column + 1 && !branches.is_empty() {
                break;
            }
        }
        // Stop if we see something that can't start a pattern.
        if !can_start_pattern(p.peek()) {
            break;
        }

        let pat = parse_pattern(p)?;
        p.expect(&Token::Arrow)?;
        let body = parse_expr(p)?;
        branches.push(CaseBranch { pattern: pat, body });
    }

    if branches.is_empty() {
        return Err(p.error("expected at least one case branch"));
    }

    Ok(p.spanned_from(
        start,
        Expr::CaseOf {
            expr: Box::new(subject),
            branches,
        },
    ))
}

fn parse_let_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    p.expect(&Token::Let)?;

    // Parse let declarations.
    // Each declaration must be indented past the `let` keyword.
    let let_col = start.column;
    let mut declarations = Vec::new();

    p.skip_whitespace();
    let decl_col = p.current_column();

    loop {
        p.skip_whitespace();
        if p.is_eof() || matches!(p.peek(), Token::In) {
            break;
        }
        // Declarations must be at the same column as the first declaration.
        if p.current_column() < decl_col && !p.in_paren_context() {
            break;
        }
        if p.current_column() < let_col + 1 && !p.in_paren_context() {
            break;
        }

        let decl_start = p.current_pos();
        let decl = parse_let_declaration(p, decl_col)?;
        declarations.push(p.spanned_from(decl_start, decl));
    }

    p.expect(&Token::In)?;
    let body = parse_expr(p)?;

    Ok(p.spanned_from(
        start,
        Expr::LetIn {
            declarations,
            body: Box::new(body),
        },
    ))
}

/// Parse a single let declaration (function def or destructuring).
fn parse_let_declaration(p: &mut Parser, _decl_col: u32) -> ParseResult<LetDeclaration> {
    p.skip_whitespace();

    // Try to determine if this is a function definition or a destructuring.
    // A function def starts with: `name patterns... = expr` or `name : Type`
    // A destructuring starts with a pattern followed by `=`

    match p.peek().clone() {
        Token::LowerName(_) => {
            // Could be a function def: `name args... = expr`
            // Or a type signature: `name : Type`
            // Or a destructuring: `name = expr` (which is just a zero-arg function)

            // Check if next meaningful token is `:` (type signature)
            let next = p.peek_nth_past_whitespace(1);
            if matches!(next, Token::Colon) {
                // This is a type signature. Parse it, then parse the function body.
                let sig = parse_signature(p)?;
                p.skip_whitespace();
                // Now parse the actual function implementation.
                let impl_decl = parse_let_function_impl(p, Some(sig))?;
                return Ok(impl_decl);
            }

            // Otherwise it's a function implementation (possibly zero-arg).
            parse_let_function_impl(p, None)
        }
        _ => {
            // Destructuring pattern.
            let pattern = parse_pattern(p)?;
            p.expect(&Token::Equals)?;
            let body = parse_expr(p)?;
            Ok(LetDeclaration::Destructuring {
                pattern: Box::new(pattern),
                body: Box::new(body),
            })
        }
    }
}

fn parse_signature(p: &mut Parser) -> ParseResult<Spanned<Signature>> {
    let start = p.current_pos();
    let name = p.expect_lower_name()?;
    p.expect(&Token::Colon)?;
    let type_annotation = parse_type(p)?;
    Ok(p.spanned_from(
        start,
        Signature {
            name,
            type_annotation,
        },
    ))
}

fn parse_let_function_impl(
    p: &mut Parser,
    signature: Option<Spanned<Signature>>,
) -> ParseResult<LetDeclaration> {
    let start = p.current_pos();
    let name = p.expect_lower_name()?;

    // Parse argument patterns until we hit `=`.
    let mut args = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Equals) {
            break;
        }
        if !can_start_pattern(p.peek()) {
            break;
        }
        args.push(super::pattern::parse_pattern(p)?);
    }

    p.expect(&Token::Equals)?;
    let body = parse_expr(p)?;

    let implementation = FunctionImplementation { name, args, body };
    let func = Function {
        documentation: None,
        signature,
        declaration: p.spanned_from(start, implementation),
    };

    Ok(LetDeclaration::Function(Box::new(func)))
}

fn parse_lambda_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    p.expect(&Token::Backslash)?;

    // Parse argument patterns until `->`.
    let mut args = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Arrow) {
            break;
        }
        args.push(parse_pattern(p)?);
    }
    if args.is_empty() {
        return Err(p.error("expected at least one argument in lambda"));
    }

    p.expect(&Token::Arrow)?;
    let body = parse_expr(p)?;

    Ok(p.spanned_from(
        start,
        Expr::Lambda {
            args,
            body: Box::new(body),
        },
    ))
}

fn parse_record_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let start = p.current_pos();
    p.expect(&Token::LeftBrace)?;
    p.skip_whitespace();

    // Empty record: `{}`
    if matches!(p.peek(), Token::RightBrace) {
        p.advance();
        return Ok(p.spanned_from(start, Expr::Record(Vec::new())));
    }

    // Check for record update: `{ name | ... }`
    // We need to look ahead: `lowerName |`
    if matches!(p.peek(), Token::LowerName(_)) {
        let save_pos = p.pos;
        if let Ok(base_name) = p.expect_lower_name() {
            p.skip_whitespace();
            if matches!(p.peek(), Token::Pipe) {
                p.advance(); // consume `|`
                let updates = parse_record_setters(p)?;
                p.expect(&Token::RightBrace)?;
                return Ok(p.spanned_from(
                    start,
                    Expr::RecordUpdate {
                        base: base_name,
                        updates,
                    },
                ));
            }
            // Not a record update — backtrack.
            p.pos = save_pos;
        } else {
            p.pos = save_pos;
        }
    }

    // Regular record expression.
    let fields = parse_record_setters(p)?;
    p.expect(&Token::RightBrace)?;
    Ok(p.spanned_from(start, Expr::Record(fields)))
}

fn parse_record_setters(p: &mut Parser) -> ParseResult<Vec<Spanned<RecordSetter>>> {
    let mut setters = Vec::new();
    setters.push(parse_record_setter(p)?);

    while p.eat(&Token::Comma) {
        setters.push(parse_record_setter(p)?);
    }

    Ok(setters)
}

fn parse_record_setter(p: &mut Parser) -> ParseResult<Spanned<RecordSetter>> {
    let start = p.current_pos();
    let field = p.expect_lower_name()?;
    p.expect(&Token::Equals)?;
    let value = parse_expr(p)?;
    Ok(p.spanned_from(start, RecordSetter { field, value }))
}

/// Parse a possibly-qualified value reference: `foo`, `List.map`, `Maybe.Just`
fn parse_qualified_value(p: &mut Parser) -> ParseResult<(Vec<String>, String)> {
    let first = p.expect_upper_name()?;
    let mut parts: Vec<String> = vec![first.value];

    // Consume `.Name` segments.
    while matches!(p.peek(), Token::Dot) {
        // Peek at what follows the dot.
        let next = p.peek_nth_past_whitespace(1);
        match next {
            Token::UpperName(_) | Token::LowerName(_) => {}
            _ => break,
        }

        let dot_pos = p.pos;
        p.advance(); // consume `.`

        match p.peek().clone() {
            Token::UpperName(name) => {
                p.advance();
                parts.push(name);
            }
            Token::LowerName(name) => {
                p.advance();
                parts.push(name);
                break; // lowercase name is always the final segment
            }
            _ => {
                p.pos = dot_pos;
                break;
            }
        }
    }

    let name = parts.pop().unwrap();
    Ok((parts, name))
}

fn can_start_pattern(tok: &Token) -> bool {
    matches!(
        tok,
        Token::Underscore
            | Token::LowerName(_)
            | Token::UpperName(_)
            | Token::Literal(_)
            | Token::Minus
            | Token::LeftParen
            | Token::LeftBrace
            | Token::LeftBracket
    )
}

fn can_start_atomic_expr(tok: &Token) -> bool {
    matches!(
        tok,
        Token::Literal(_)
            | Token::LowerName(_)
            | Token::UpperName(_)
            | Token::LeftParen
            | Token::LeftBrace
            | Token::LeftBracket
            | Token::Dot
            | Token::Glsl(_)
            | Token::If
            | Token::Case
            | Token::Let
            | Token::Backslash
    )
}
