use crate::exposing::{ExposedItem, Exposing};
use crate::file::ElmModule;
use crate::import::Import;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::span::Span;
use crate::token::Token;

use super::declaration::parse_declaration;
use super::{ParseError, ParseResult, Parser};

/// Parse a complete Elm module (file).
pub fn parse_module(p: &mut Parser) -> ParseResult<ElmModule> {
    // Drain any comments collected during initial whitespace skipping.
    p.drain_comments();

    // Parse module header.
    let header = parse_module_header(p)?;

    // Parse imports.
    // Use skip_whitespace_before_doc so we don't accidentally consume
    // doc comments that belong to the first declaration. If we encounter
    // a DocComment, check whether an Import follows — if so, skip past
    // the doc comment (it's a module-level doc) and continue the loop.
    let mut imports = Vec::new();
    loop {
        p.skip_whitespace_before_doc();
        if matches!(p.peek(), Token::DocComment(_)) {
            if matches!(p.peek_past_whitespace(), Token::Import) {
                p.skip_whitespace(); // consume the doc comment
            // Fall through to parse the import below.
            } else {
                break; // Doc comment is for the first declaration.
            }
        }
        if !matches!(p.peek(), Token::Import) {
            break;
        }
        imports.push(parse_import(p)?);
    }

    // Parse declarations.
    // Use skip_whitespace_before_doc so doc comments stay in the stream
    // for parse_declaration's try_doc_comment to pick up.
    let mut declarations = Vec::new();
    loop {
        p.skip_whitespace_before_doc();
        if p.is_eof() {
            break;
        }
        declarations.push(parse_declaration(p)?);
    }

    // All comments encountered during parsing were saved by skip_whitespace.
    let comments = p.drain_comments();

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
        Token::LowerName(ref s) if s == "effect" => {
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
                        ));
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
    // Track the end of the import's meaningful content explicitly. We can't
    // use `spanned_from` here because it derives `end` from the last consumed
    // token, and `skip_whitespace` below consumes trailing `Newline` tokens
    // while looking for optional `as`/`exposing` — that would leak the
    // import's span past its trailing whitespace and make line-based fixes
    // (like removing unused imports) eat blank lines after the import.
    let mut end = module_name.span.end;

    // Optional `as Alias`
    p.skip_whitespace();
    let alias = if matches!(p.peek(), Token::As) {
        p.advance();
        let name = parse_module_name(p)?;
        end = name.span.end;
        Some(name)
    } else {
        None
    };

    // Optional `exposing (...)`
    p.skip_whitespace();
    let exposing = if matches!(p.peek(), Token::Exposing) {
        p.advance();
        let exp = parse_exposing(p)?;
        end = exp.span.end;
        Some(exp)
    } else {
        None
    };

    Ok(Spanned::new(
        Span::new(start, end),
        Import {
            module_name,
            alias,
            exposing,
        },
    ))
}

/// Parse a module with error recovery.
///
/// If the module header or imports fail, returns `(None, errors)`.
/// If declarations fail, skips to the next declaration and continues,
/// returning the partial AST with all collected errors.
pub fn parse_module_recovering(p: &mut Parser) -> (Option<ElmModule>, Vec<ParseError>) {
    let mut errors = Vec::new();

    p.drain_comments();

    let header = match parse_module_header(p) {
        Ok(h) => h,
        Err(e) => return (None, vec![e]),
    };

    let mut imports = Vec::new();
    loop {
        p.skip_whitespace_before_doc();
        if matches!(p.peek(), Token::DocComment(_)) {
            if matches!(p.peek_past_whitespace(), Token::Import) {
                p.skip_whitespace();
            } else {
                break;
            }
        }
        if !matches!(p.peek(), Token::Import) {
            break;
        }
        match parse_import(p) {
            Ok(imp) => imports.push(imp),
            Err(e) => {
                errors.push(e);
                p.skip_to_next_declaration();
            }
        }
    }

    let mut declarations = Vec::new();
    loop {
        p.skip_whitespace_before_doc();
        if p.is_eof() {
            break;
        }
        match parse_declaration(p) {
            Ok(decl) => declarations.push(decl),
            Err(e) => {
                errors.push(e);
                p.skip_to_next_declaration();
            }
        }
    }

    let comments = p.drain_comments();

    (
        Some(ElmModule {
            header,
            imports,
            declarations,
            comments,
        }),
        errors,
    )
}
