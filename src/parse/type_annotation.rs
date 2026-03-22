use crate::node::Spanned;
use crate::token::Token;
use crate::type_annotation::{RecordField, TypeAnnotation};

use super::{ParseResult, Parser};

/// Parse a type annotation.
///
/// This handles function arrows at the top level:
///   `Int -> String -> Bool`
///
/// Function arrows are right-associative:
///   `a -> b -> c` = `a -> (b -> c)`
pub fn parse_type(p: &mut Parser) -> ParseResult<Spanned<TypeAnnotation>> {
    let start = p.current_pos();
    let left = parse_type_app(p)?;

    p.skip_whitespace();
    if matches!(p.peek(), Token::Arrow) {
        p.advance(); // consume `->`
        let right = parse_type(p)?; // right-recursive for right-associativity
        let ty = TypeAnnotation::FunctionType {
            from: Box::new(left),
            to: Box::new(right),
        };
        Ok(p.spanned_from(start, ty))
    } else {
        Ok(left)
    }
}

/// Parse a type application: `Maybe Int`, `Dict String Int`, or an atomic type.
fn parse_type_app(p: &mut Parser) -> ParseResult<Spanned<TypeAnnotation>> {
    let start = p.current_pos();
    p.skip_whitespace();

    // Check if this starts with an upper name (potential type application).
    match p.peek().clone() {
        Token::UpperName(_) => {
            let (module_name, name) = parse_qualified_upper(p)?;
            let name_span = name.span;

            // Collect type arguments: these are atomic types that follow.
            let mut args = Vec::new();
            loop {
                p.skip_whitespace();
                // Type args must be atomic (no unparenthesized arrows or applications).
                // Also stop at tokens that can't start a type.
                if !can_start_atomic_type(p.peek()) {
                    break;
                }
                // For indentation: args should be on the same line or indented past the type name.
                if p.current_column() <= name_span.start.column
                    && p.current_pos().line != name_span.start.line
                {
                    break;
                }
                args.push(parse_type_atomic(p)?);
            }

            let ty = TypeAnnotation::Typed {
                module_name,
                name,
                args,
            };
            Ok(p.spanned_from(start, ty))
        }
        _ => parse_type_atomic(p),
    }
}

/// Public entry point to parse an atomic type (used by constructor arg parsing).
pub fn parse_type_atomic_public(p: &mut Parser) -> ParseResult<Spanned<TypeAnnotation>> {
    parse_type_atomic(p)
}

/// Parse an atomic (non-application, non-arrow) type.
fn parse_type_atomic(p: &mut Parser) -> ParseResult<Spanned<TypeAnnotation>> {
    p.skip_whitespace();
    let start = p.current_pos();

    match p.peek().clone() {
        // Type variable: `a`, `msg`, `comparable`
        Token::LowerName(name) => {
            p.advance();
            Ok(p.spanned_from(start, TypeAnnotation::GenericType(name)))
        }

        // Uppercase name: could be a simple type or qualified type (without args here).
        Token::UpperName(_) => {
            let (module_name, name) = parse_qualified_upper(p)?;
            let ty = TypeAnnotation::Typed {
                module_name,
                name,
                args: Vec::new(),
            };
            Ok(p.spanned_from(start, ty))
        }

        // Parenthesized: could be `()`, `(a, b)`, `(a, b, c)`, or `(Type)`
        Token::LeftParen => {
            p.advance(); // consume `(`
            p.skip_whitespace();

            // Unit: `()`
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(p.spanned_from(start, TypeAnnotation::Unit));
            }

            // Parse the first type inside parens.
            let first = parse_type(p)?;
            p.skip_whitespace();

            match p.peek() {
                // Tuple: `(a, b)` or `(a, b, c)`
                Token::Comma => {
                    let mut elements = vec![first];
                    while p.eat(&Token::Comma) {
                        elements.push(parse_type(p)?);
                    }
                    p.expect(&Token::RightParen)?;
                    Ok(p.spanned_from(start, TypeAnnotation::Tupled(elements)))
                }
                // Just parenthesized: `(Type)`
                Token::RightParen => {
                    p.advance();
                    // Unwrap the parentheses — they don't have semantic meaning in types.
                    Ok(first)
                }
                _ => Err(p.error("expected `,` or `)` in type")),
            }
        }

        // Record type: `{ name : String }` or `{ a | name : String }`
        Token::LeftBrace => parse_record_type(p),

        _ => Err(p.error(format!("expected type, found {}", super::describe(p.peek())))),
    }
}

/// Parse a record type or generic record type.
fn parse_record_type(p: &mut Parser) -> ParseResult<Spanned<TypeAnnotation>> {
    let start = p.current_pos();
    p.expect(&Token::LeftBrace)?;
    p.skip_whitespace();

    // Empty record: `{}`
    if matches!(p.peek(), Token::RightBrace) {
        p.advance();
        return Ok(p.spanned_from(start, TypeAnnotation::Record(Vec::new())));
    }

    // Check for generic record: `{ a | ... }`
    // We need to look ahead: if it's `lowerName |`, it's a generic record.
    if matches!(p.peek(), Token::LowerName(_)) {
        let save_pos = p.pos;
        let maybe_base = p.expect_lower_name();
        if let Ok(base) = maybe_base {
            p.skip_whitespace();
            if matches!(p.peek(), Token::Pipe) {
                p.advance(); // consume `|`
                let fields = parse_record_fields(p)?;
                p.expect(&Token::RightBrace)?;
                return Ok(p.spanned_from(start, TypeAnnotation::GenericRecord { base, fields }));
            }
            // Not a generic record — backtrack.
            p.pos = save_pos;
        } else {
            p.pos = save_pos;
        }
    }

    // Regular record type.
    let fields = parse_record_fields(p)?;
    p.expect(&Token::RightBrace)?;
    Ok(p.spanned_from(start, TypeAnnotation::Record(fields)))
}

/// Parse comma-separated record fields: `name : Type, age : Type`
fn parse_record_fields(p: &mut Parser) -> ParseResult<Vec<Spanned<RecordField>>> {
    let mut fields = Vec::new();
    fields.push(parse_record_field(p)?);

    while p.eat(&Token::Comma) {
        fields.push(parse_record_field(p)?);
    }

    Ok(fields)
}

/// Parse a single record field: `name : Type`
fn parse_record_field(p: &mut Parser) -> ParseResult<Spanned<RecordField>> {
    let start = p.current_pos();
    let name = p.expect_lower_name()?;
    p.expect(&Token::Colon)?;
    let type_annotation = parse_type(p)?;
    Ok(p.spanned_from(
        start,
        RecordField {
            name,
            type_annotation,
        },
    ))
}

/// Parse a possibly-qualified uppercase name: `Maybe`, `Dict.Dict`
///
/// Returns `(module_path, final_name)`.
fn parse_qualified_upper(p: &mut Parser) -> ParseResult<(Vec<String>, Spanned<String>)> {
    let mut parts = Vec::new();
    let first = p.expect_upper_name()?;
    parts.push(first);

    // Consume `.UpperName` segments.
    while matches!(p.peek(), Token::Dot) {
        // Peek ahead: is the token after `.` an upper name?
        if !matches!(p.peek_nth_past_whitespace(1), Token::UpperName(_)) {
            // It might be `.lowerName` (record access) — don't consume the dot.
            // But in type context, `.` followed by non-upper is the end.
            break;
        }
        // Check that the dot is immediately adjacent (no whitespace).
        let dot_pos = p.pos;
        p.advance(); // consume `.`
        // If there's whitespace between dot and next name, backtrack.
        if matches!(p.peek(), Token::Newline) {
            p.pos = dot_pos;
            break;
        }
        match p.peek() {
            Token::UpperName(_) => {
                let name = p.expect_upper_name()?;
                parts.push(name);
            }
            _ => {
                p.pos = dot_pos;
                break;
            }
        }
    }

    let name = parts.pop().unwrap();
    let module_name = parts.into_iter().map(|s| s.value).collect();
    Ok((module_name, name))
}

/// Can this token start an atomic type?
fn can_start_atomic_type(tok: &Token) -> bool {
    matches!(
        tok,
        Token::LowerName(_)
            | Token::UpperName(_)
            | Token::LeftParen
            | Token::LeftBrace
    )
}
