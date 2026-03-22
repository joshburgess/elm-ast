use crate::comment::Comment;
use crate::exposing::{ExposedItem, Exposing};
use crate::file::ElmModule;
use crate::import::Import;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::token::Token;

use super::declaration::parse_declaration;
use super::{ParseResult, Parser};

/// Parse a complete Elm module (file).
pub fn parse_module(p: &mut Parser) -> ParseResult<ElmModule> {
    // Collect leading comments.
    let mut comments = Vec::new();
    collect_comments(p, &mut comments);

    // Parse module header.
    let header = parse_module_header(p)?;

    // Parse imports.
    let mut imports = Vec::new();
    loop {
        collect_comments(p, &mut comments);
        p.skip_whitespace();
        if !matches!(p.peek(), Token::Import) {
            break;
        }
        imports.push(parse_import(p)?);
    }

    // Parse declarations.
    let mut declarations = Vec::new();
    loop {
        collect_comments(p, &mut comments);
        p.skip_whitespace();
        if p.is_eof() {
            break;
        }
        declarations.push(parse_declaration(p)?);
    }

    Ok(ElmModule {
        header,
        imports,
        declarations,
        comments,
    })
}

/// Parse a module header: `module Foo exposing (..)`, `port module ...`, `effect module ...`
fn parse_module_header(p: &mut Parser) -> ParseResult<Spanned<ModuleHeader>> {
    p.skip_whitespace();
    let start = p.current_pos();

    match p.peek().clone() {
        Token::Effect => {
            p.advance(); // consume `effect`
            p.expect(&Token::Module)?;
            let name = parse_module_name(p)?;
            p.expect(&Token::Where)?;

            // Parse the `{ command = MyCmd, subscription = MySub }` block.
            p.expect(&Token::LeftBrace)?;
            let mut command = None;
            let mut subscription = None;

            loop {
                p.skip_whitespace();
                if matches!(p.peek(), Token::RightBrace) {
                    break;
                }
                let key = p.expect_lower_name()?;
                p.expect(&Token::Equals)?;
                let val = p.expect_upper_name()?;

                match key.value.as_str() {
                    "command" => command = Some(val),
                    "subscription" => subscription = Some(val),
                    _ => {
                        return Err(p.error_at(
                            key.span,
                            format!("unexpected effect module key: {}", key.value),
                        ))
                    }
                }

                // Optional comma between entries.
                p.eat(&Token::Comma);
            }
            p.expect(&Token::RightBrace)?;

            p.expect(&Token::Exposing)?;
            let exposing = parse_exposing(p)?;

            Ok(p.spanned_from(
                start,
                ModuleHeader::Effect {
                    name,
                    exposing,
                    command,
                    subscription,
                },
            ))
        }

        Token::Port => {
            p.advance(); // consume `port`
            p.expect(&Token::Module)?;
            let name = parse_module_name(p)?;
            p.expect(&Token::Exposing)?;
            let exposing = parse_exposing(p)?;
            Ok(p.spanned_from(start, ModuleHeader::Port { name, exposing }))
        }

        Token::Module => {
            p.advance(); // consume `module`
            let name = parse_module_name(p)?;
            p.expect(&Token::Exposing)?;
            let exposing = parse_exposing(p)?;
            Ok(p.spanned_from(start, ModuleHeader::Normal { name, exposing }))
        }

        _ => Err(p.error("expected `module`, `port module`, or `effect module`")),
    }
}

/// Parse a dotted module name: `Html.Attributes`
fn parse_module_name(p: &mut Parser) -> ParseResult<Spanned<Vec<String>>> {
    let start = p.current_pos();
    let first = p.expect_upper_name()?;
    let mut parts = vec![first.value];

    while matches!(p.peek(), Token::Dot) {
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

    Ok(p.spanned_from(start, parts))
}

/// Parse an exposing list: `(..)` or `(foo, Bar, Baz(..))`
pub fn parse_exposing(p: &mut Parser) -> ParseResult<Spanned<Exposing>> {
    let start = p.current_pos();
    p.expect(&Token::LeftParen)?;
    p.skip_whitespace();

    // `(..)`
    if matches!(p.peek(), Token::DotDot) {
        let dot_span = p.peek_span();
        p.advance();
        p.expect(&Token::RightParen)?;
        return Ok(p.spanned_from(start, Exposing::All(dot_span)));
    }

    // Explicit list.
    let mut items = Vec::new();
    items.push(parse_exposed_item(p)?);

    while p.eat(&Token::Comma) {
        items.push(parse_exposed_item(p)?);
    }

    p.expect(&Token::RightParen)?;
    Ok(p.spanned_from(start, Exposing::Explicit(items)))
}

fn parse_exposed_item(p: &mut Parser) -> ParseResult<Spanned<ExposedItem>> {
    p.skip_whitespace();
    let start = p.current_pos();

    match p.peek().clone() {
        // Lowercase: function expose
        Token::LowerName(name) => {
            p.advance();
            Ok(p.spanned_from(start, ExposedItem::Function(name)))
        }

        // Uppercase: type expose (possibly with `(..)`)
        Token::UpperName(name) => {
            p.advance();
            p.skip_whitespace();

            // Check for `(..)`
            if matches!(p.peek(), Token::LeftParen) {
                let open_start = p.peek_span();
                p.advance();
                p.skip_whitespace();
                if matches!(p.peek(), Token::DotDot) {
                    p.advance();
                    let close = p.expect(&Token::RightParen)?;
                    let open_span = open_start.merge(close.span);
                    Ok(p.spanned_from(
                        start,
                        ExposedItem::TypeExpose {
                            name,
                            open: Some(open_span),
                        },
                    ))
                } else {
                    // `Type()` is not valid Elm, but let's be lenient.
                    p.expect(&Token::RightParen)?;
                    Ok(p.spanned_from(start, ExposedItem::TypeOrAlias(name)))
                }
            } else {
                Ok(p.spanned_from(start, ExposedItem::TypeOrAlias(name)))
            }
        }

        // Operator in parens: `(+)`
        Token::LeftParen => {
            p.advance();
            p.skip_whitespace();
            let op = match p.peek().clone() {
                Token::Operator(op) => {
                    p.advance();
                    op
                }
                Token::Minus => {
                    p.advance();
                    "-".into()
                }
                _ => return Err(p.error("expected operator in exposing list")),
            };
            p.expect(&Token::RightParen)?;
            Ok(p.spanned_from(start, ExposedItem::Infix(op)))
        }

        _ => Err(p.error(format!(
            "expected exposed item, found {}",
            super::describe(p.peek())
        ))),
    }
}

/// Parse an import declaration.
fn parse_import(p: &mut Parser) -> ParseResult<Spanned<Import>> {
    let start = p.current_pos();
    p.expect(&Token::Import)?;

    let module_name = parse_module_name(p)?;

    // Optional `as Alias`
    p.skip_whitespace();
    let alias = if matches!(p.peek(), Token::As) {
        p.advance();
        Some(parse_module_name(p)?)
    } else {
        None
    };

    // Optional `exposing (...)`
    p.skip_whitespace();
    let exposing = if matches!(p.peek(), Token::Exposing) {
        p.advance();
        Some(parse_exposing(p)?)
    } else {
        None
    };

    Ok(p.spanned_from(
        start,
        Import {
            module_name,
            alias,
            exposing,
        },
    ))
}

/// Collect any comment tokens into the comments vec.
fn collect_comments(p: &mut Parser, comments: &mut Vec<Spanned<Comment>>) {
    loop {
        match p.peek().clone() {
            Token::Newline => {
                p.advance();
            }
            Token::LineComment(text) => {
                let tok = p.advance();
                comments.push(Spanned::new(tok.span, Comment::Line(text)));
            }
            Token::BlockComment(text) => {
                let tok = p.advance();
                comments.push(Spanned::new(tok.span, Comment::Block(text)));
            }
            Token::DocComment(_text) => {
                // Don't consume doc comments here — they get attached to declarations.
                break;
            }
            _ => break,
        }
    }
}
