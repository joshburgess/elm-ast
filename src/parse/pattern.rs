use crate::literal::Literal;
use crate::node::Spanned;
use crate::pattern::Pattern;
use crate::token::Token;

use super::{ParseResult, Parser};

/// Parse a pattern (including cons `::` and `as`).
///
/// Precedence (lowest to highest):
///   1. `as` — `pat as name`
///   2. `::` — `x :: xs`
///   3. Constructor application — `Just x`
///   4. Atomic patterns
pub fn parse_pattern(p: &mut Parser) -> ParseResult<Spanned<Pattern>> {
    let start = p.current_pos();
    let pat = parse_cons_pattern(p)?;

    // Check for `as` binding.
    p.skip_whitespace();
    if matches!(p.peek(), Token::As) {
        p.advance(); // consume `as`
        let name = p.expect_lower_name()?;
        let result = Pattern::As {
            pattern: Box::new(pat),
            name,
        };
        Ok(p.spanned_from(start, result))
    } else {
        Ok(pat)
    }
}

/// Parse a cons pattern: `x :: xs` or `x :: y :: rest`
/// Cons is right-associative.
fn parse_cons_pattern(p: &mut Parser) -> ParseResult<Spanned<Pattern>> {
    let start = p.current_pos();
    let left = parse_app_pattern(p)?;

    p.skip_whitespace();
    if matches!(p.peek(), Token::Operator(op) if op == "::") {
        p.advance(); // consume `::`
        let right = parse_cons_pattern(p)?; // right-recursive
        let result = Pattern::Cons {
            head: Box::new(left),
            tail: Box::new(right),
        };
        Ok(p.spanned_from(start, result))
    } else {
        Ok(left)
    }
}

/// Parse a constructor application pattern: `Just x`, `Err msg`
fn parse_app_pattern(p: &mut Parser) -> ParseResult<Spanned<Pattern>> {
    let start = p.current_pos();
    p.skip_whitespace();

    // Check for constructor pattern (uppercase name possibly qualified).
    if matches!(p.peek(), Token::UpperName(_)) {
        let ctor_start = p.current_pos();
        let (module_name, ctor_name) = parse_qualified_ctor(p)?;

        // Collect arguments (atomic patterns).
        let mut args = Vec::new();
        loop {
            p.skip_whitespace();
            if !can_start_atomic_pattern(p.peek()) {
                break;
            }
            // Args must be indented past the constructor or on the same line.
            if p.current_column() <= ctor_start.column
                && p.current_pos().line != ctor_start.line
            {
                break;
            }
            args.push(parse_atomic_pattern(p)?);
        }

        let result = Pattern::Constructor {
            module_name,
            name: ctor_name,
            args,
        };
        Ok(p.spanned_from(start, result))
    } else {
        parse_atomic_pattern(p)
    }
}

/// Parse an atomic (non-application) pattern.
fn parse_atomic_pattern(p: &mut Parser) -> ParseResult<Spanned<Pattern>> {
    p.skip_whitespace();
    let start = p.current_pos();

    match p.peek().clone() {
        // Wildcard: `_`
        Token::Underscore => {
            p.advance();
            Ok(p.spanned_from(start, Pattern::Anything))
        }

        // Variable: `x`, `name`
        Token::LowerName(name) => {
            p.advance();
            Ok(p.spanned_from(start, Pattern::Var(name)))
        }

        // Constructor without args (atomic): `Nothing`, `True`
        Token::UpperName(_) => {
            let (module_name, name) = parse_qualified_ctor(p)?;
            Ok(p.spanned_from(
                start,
                Pattern::Constructor {
                    module_name,
                    name,
                    args: Vec::new(),
                },
            ))
        }

        // Literal patterns
        Token::Literal(lit) => {
            p.advance();
            match lit {
                Literal::Hex(n) => Ok(p.spanned_from(start, Pattern::Hex(n))),
                _ => Ok(p.spanned_from(start, Pattern::Literal(lit))),
            }
        }

        // Negated number pattern: `-42`
        Token::Minus => {
            p.advance();
            p.skip_whitespace();
            match p.peek().clone() {
                Token::Literal(Literal::Int(n)) => {
                    p.advance();
                    Ok(p.spanned_from(start, Pattern::Literal(Literal::Int(-n))))
                }
                Token::Literal(Literal::Float(n)) => {
                    p.advance();
                    Ok(p.spanned_from(start, Pattern::Literal(Literal::Float(-n))))
                }
                _ => Err(p.error("expected number after `-` in pattern")),
            }
        }

        // Parenthesized, tuple, or unit: `()`, `(a, b)`, `(pat)`
        Token::LeftParen => {
            p.advance(); // consume `(`
            p.skip_whitespace();

            // Unit: `()`
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(p.spanned_from(start, Pattern::Unit));
            }

            let first = parse_pattern(p)?;
            p.skip_whitespace();

            match p.peek() {
                // Tuple: `(a, b)` or `(a, b, c)`
                Token::Comma => {
                    let mut elements = vec![first];
                    while p.eat(&Token::Comma) {
                        elements.push(parse_pattern(p)?);
                    }
                    p.expect(&Token::RightParen)?;
                    Ok(p.spanned_from(start, Pattern::Tuple(elements)))
                }
                // Parenthesized pattern
                Token::RightParen => {
                    p.advance();
                    Ok(p.spanned_from(start, Pattern::Parenthesized(Box::new(first))))
                }
                _ => Err(p.error("expected `,` or `)` in pattern")),
            }
        }

        // Record destructuring: `{ name, age }`
        Token::LeftBrace => {
            p.advance(); // consume `{`
            let mut fields = Vec::new();
            if !matches!(p.peek_past_whitespace(), Token::RightBrace) {
                fields.push(p.expect_lower_name()?);
                while p.eat(&Token::Comma) {
                    fields.push(p.expect_lower_name()?);
                }
            }
            p.expect(&Token::RightBrace)?;
            Ok(p.spanned_from(start, Pattern::Record(fields)))
        }

        // List pattern: `[ x, y, z ]`
        Token::LeftBracket => {
            p.advance(); // consume `[`
            p.skip_whitespace();
            let mut elements = Vec::new();
            if !matches!(p.peek(), Token::RightBracket) {
                elements.push(parse_pattern(p)?);
                while p.eat(&Token::Comma) {
                    elements.push(parse_pattern(p)?);
                }
            }
            p.expect(&Token::RightBracket)?;
            Ok(p.spanned_from(start, Pattern::List(elements)))
        }

        _ => Err(p.error(format!(
            "expected pattern, found {}",
            super::describe(p.peek())
        ))),
    }
}

/// Parse a possibly-qualified constructor name: `Just`, `Maybe.Nothing`
fn parse_qualified_ctor(p: &mut Parser) -> ParseResult<(Vec<String>, String)> {
    let first = p.expect_upper_name()?;
    let mut parts = vec![first.value];

    // Consume `.UpperName` segments for qualification.
    while matches!(p.peek(), Token::Dot) {
        if !matches!(p.peek_nth_past_whitespace(1), Token::UpperName(_)) {
            break;
        }
        let dot_pos = p.pos;
        p.advance(); // consume `.`
        match p.peek() {
            Token::UpperName(_) => {
                let name = p.expect_upper_name()?;
                parts.push(name.value);
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

/// Can this token start an atomic pattern?
fn can_start_atomic_pattern(tok: &Token) -> bool {
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
