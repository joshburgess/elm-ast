use crate::comment::Comment;
use crate::declaration::{CustomType, Declaration, InfixDef, TypeAlias, ValueConstructor};
use crate::expr::{Function, FunctionImplementation, Signature};
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::token::Token;

use super::expr::parse_expr;
use super::pattern::parse_pattern;
use super::type_annotation::parse_type;
use super::{ParseResult, Parser};

/// Parse a top-level declaration.
pub fn parse_declaration(p: &mut Parser) -> ParseResult<Spanned<Declaration>> {
    let start = p.current_pos();

    // Collect an optional doc comment.
    let doc = p.try_doc_comment();

    p.skip_whitespace();

    match p.peek().clone() {
        // `type` — could be `type alias ...` or `type Foo = ...`
        Token::Type => {
            p.advance();
            p.skip_whitespace();

            if matches!(p.peek(), Token::Alias) {
                p.advance();
                let alias = parse_type_alias(p, doc)?;
                Ok(p.spanned_from(start, Declaration::AliasDeclaration(alias)))
            } else {
                let custom = parse_custom_type(p, doc)?;
                Ok(p.spanned_from(start, Declaration::CustomTypeDeclaration(custom)))
            }
        }

        // `port` — port declaration
        Token::Port => {
            p.advance();
            p.skip_whitespace();

            // `port module` is handled at module level, not here.
            // This is `port name : Type`
            let sig = parse_signature(p)?;
            Ok(p.spanned_from(start, Declaration::PortDeclaration(sig.value)))
        }

        // `infix` — infix declaration
        Token::Infix => {
            p.advance();
            let infix = parse_infix_declaration(p)?;
            Ok(p.spanned_from(start, Declaration::InfixDeclaration(infix)))
        }

        // Lowercase name — function definition or type signature + definition
        Token::LowerName(_) => {
            // Check if next token is `:` (type signature).
            let next = p.peek_nth_past_whitespace(1);
            if matches!(next, Token::Colon) {
                let func = parse_function_with_signature(p, doc)?;
                Ok(p.spanned_from(start, Declaration::FunctionDeclaration(Box::new(func))))
            } else {
                let func = parse_function_no_signature(p, doc)?;
                Ok(p.spanned_from(start, Declaration::FunctionDeclaration(Box::new(func))))
            }
        }

        // Pattern destructuring at the top level (rare)
        _ if can_start_pattern(p.peek()) => {
            let pattern = parse_pattern(p)?;
            p.expect(&Token::Equals)?;
            let body = parse_expr(p)?;
            Ok(p.spanned_from(start, Declaration::Destructuring { pattern, body }))
        }

        _ => Err(p.error(format!(
            "expected declaration, found {}",
            super::describe(p.peek())
        ))),
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

fn parse_function_with_signature(
    p: &mut Parser,
    doc: Option<Spanned<String>>,
) -> ParseResult<Function> {
    let sig = parse_signature(p)?;

    // Now parse the function implementation on the next line.
    p.skip_whitespace();

    let impl_start = p.current_pos();
    let name = p.expect_lower_name()?;

    let mut args = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Equals) {
            break;
        }
        if !can_start_pattern(p.peek()) {
            break;
        }
        args.push(parse_pattern(p)?);
    }

    p.expect(&Token::Equals)?;
    let body_snapshot = p.pending_comments_snapshot();
    let mut body = parse_expr(p)?;
    super::expr::attach_pre_body_comments(p, &mut body, body_snapshot);

    let implementation = FunctionImplementation { name, args, body };

    Ok(Function {
        documentation: doc,
        signature: Some(sig),
        declaration: p.spanned_from(impl_start, implementation),
    })
}

fn parse_function_no_signature(
    p: &mut Parser,
    doc: Option<Spanned<String>>,
) -> ParseResult<Function> {
    let start = p.current_pos();
    let name = p.expect_lower_name()?;

    let mut args = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Equals) {
            break;
        }
        if !can_start_pattern(p.peek()) {
            break;
        }
        args.push(parse_pattern(p)?);
    }

    p.expect(&Token::Equals)?;
    let body_snapshot = p.pending_comments_snapshot();
    let mut body = parse_expr(p)?;
    super::expr::attach_pre_body_comments(p, &mut body, body_snapshot);

    let implementation = FunctionImplementation { name, args, body };

    Ok(Function {
        documentation: doc,
        signature: None,
        declaration: p.spanned_from(start, implementation),
    })
}

fn parse_type_alias(p: &mut Parser, doc: Option<Spanned<String>>) -> ParseResult<TypeAlias> {
    let name = p.expect_upper_name()?;

    // Parse generic type parameters.
    let mut generics = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Equals) {
            break;
        }
        match p.peek().clone() {
            Token::LowerName(var) => {
                let tok = p.advance();
                generics.push(Spanned::new(tok.span, var));
            }
            _ => break,
        }
    }

    p.expect(&Token::Equals)?;
    let type_annotation = parse_type(p)?;

    Ok(TypeAlias {
        documentation: doc,
        name,
        generics,
        type_annotation,
    })
}

fn parse_custom_type(p: &mut Parser, doc: Option<Spanned<String>>) -> ParseResult<CustomType> {
    let name = p.expect_upper_name()?;

    // Parse generic type parameters.
    let mut generics = Vec::new();
    loop {
        p.skip_whitespace();
        if matches!(p.peek(), Token::Equals) {
            break;
        }
        match p.peek().clone() {
            Token::LowerName(var) => {
                let tok = p.advance();
                generics.push(Spanned::new(tok.span, var));
            }
            _ => break,
        }
    }

    p.expect(&Token::Equals)?;

    // Parse constructors separated by `|`. Capture comments between
    // constructors and split them into "pre-pipe" (appearing BEFORE the
    // `|` separator, printed as trailing on previous constructor) vs
    // "post-pipe" (appearing AFTER `|`, printed inline with the pipe).
    // elm-format distinguishes these by byte offset relative to the `|`.
    let snapshot_outer = p.pending_comments_snapshot();
    let mut constructors = Vec::new();
    let mut first_ctor = parse_value_constructor(p)?;
    // Comments between `=` and the first ctor name are leading on first ctor.
    let first_name_start = first_ctor.value.name.span.start.offset;
    let before_first = p.take_pending_comments_since(snapshot_outer);
    let (leading_first, other_first): (Vec<_>, Vec<_>) = before_first
        .into_iter()
        .partition(|c| c.span.end.offset <= first_name_start);
    p.restore_pending_comments(other_first);
    if !leading_first.is_empty() {
        let mut all_leading = leading_first;
        all_leading.extend(std::mem::take(&mut first_ctor.comments));
        first_ctor.comments = all_leading;
    }
    constructors.push(first_ctor);

    loop {
        // Find the next `|` and capture its offset. skip_whitespace pushes
        // comments BEFORE `|` onto pending so they're available to split.
        p.skip_whitespace();
        if !matches!(p.peek(), Token::Pipe) {
            break;
        }
        let pipe_offset = p.peek_span().start.offset;
        p.advance(); // consume `|`

        let mut ctor = parse_value_constructor(p)?;

        let prev_end = constructors.last().unwrap().span.end.offset;
        let ctor_name_start = ctor.value.name.span.start.offset;

        let all = p.take_pending_comments_since(snapshot_outer);

        let mut pre_pipe: Vec<Spanned<crate::comment::Comment>> = Vec::new();
        let mut post_pipe: Vec<Spanned<crate::comment::Comment>> = Vec::new();
        let mut other: Vec<Spanned<crate::comment::Comment>> = Vec::new();
        for c in all {
            let in_gap =
                c.span.start.offset > prev_end && c.span.end.offset <= ctor_name_start;
            if !in_gap {
                other.push(c);
            } else if c.span.start.offset < pipe_offset {
                pre_pipe.push(c);
            } else {
                post_pipe.push(c);
            }
        }
        p.restore_pending_comments(other);

        // pre_pipe: comments BEFORE `|` in source. Attach to CURRENT ctor's
        //   `comments` so the printer emits them detached above `|`.
        // post_pipe: comments AFTER `|` in source. Store separately so the
        //   printer emits them inline with `|` (via pre_pipe_comments field,
        //   which despite its name stores the inline-with-pipe comments).
        if !pre_pipe.is_empty() {
            let mut all_leading = pre_pipe;
            all_leading.extend(std::mem::take(&mut ctor.comments));
            ctor.comments = all_leading;
        }
        if !post_pipe.is_empty() {
            ctor.value.pre_pipe_comments = post_pipe;
        }
        constructors.push(ctor);
    }

    Ok(CustomType {
        documentation: doc,
        name,
        generics,
        constructors,
    })
}

fn parse_value_constructor(p: &mut Parser) -> ParseResult<Spanned<ValueConstructor>> {
    let start = p.current_pos();
    let name = p.expect_upper_name()?;

    // Parse constructor argument types (atomic types only).
    let mut args = Vec::new();
    loop {
        p.skip_whitespace();
        if !can_start_atomic_type(p.peek()) {
            break;
        }
        // Stop at `|` (next constructor) or tokens that end the type def.
        if matches!(p.peek(), Token::Pipe) {
            break;
        }
        // Arguments must be on the same line or indented past the constructor name.
        if !p.in_paren_context()
            && p.current_column() <= name.span.start.column
            && p.current_pos().line != name.span.start.line
        {
            break;
        }
        args.push(super::type_annotation::parse_type_atomic_public(p)?);
    }

    // Claim a same-line trailing line comment: `| Ctor args -- text`.
    // `skip_whitespace` inside the loop above has already pushed it into
    // `collected_comments`.
    let last_line = args
        .last()
        .map(|a| a.span.end.line)
        .unwrap_or(name.span.end.line);
    let trailing_comment = match p.collected_comments.last() {
        Some(c) if c.span.start.line == last_line && matches!(c.value, Comment::Line(_)) => {
            p.collected_comments.pop()
        }
        _ => None,
    };

    Ok(p.spanned_from(
        start,
        ValueConstructor {
            name,
            args,
            pre_pipe_comments: Vec::new(),
            trailing_comment,
        },
    ))
}

fn parse_infix_declaration(p: &mut Parser) -> ParseResult<InfixDef> {
    p.skip_whitespace();
    let dir_start = p.current_pos();
    let direction = match p.peek() {
        Token::LowerName(name) if name == "left" => {
            p.advance();
            Spanned::new(p.span_from(dir_start), InfixDirection::Left)
        }
        Token::LowerName(name) if name == "right" => {
            p.advance();
            Spanned::new(p.span_from(dir_start), InfixDirection::Right)
        }
        Token::LowerName(name) if name == "non" => {
            p.advance();
            Spanned::new(p.span_from(dir_start), InfixDirection::Non)
        }
        _ => return Err(p.error("expected `left`, `right`, or `non` in infix declaration")),
    };

    p.skip_whitespace();
    let prec_start = p.current_pos();
    let precedence = match p.peek().clone() {
        Token::Literal(crate::literal::Literal::Int(n)) => {
            p.advance();
            Spanned::new(p.span_from(prec_start), n as u8)
        }
        _ => return Err(p.error("expected precedence number in infix declaration")),
    };

    p.expect(&Token::LeftParen)?;
    p.skip_whitespace();
    let op_start = p.current_pos();
    let operator = match p.peek().clone() {
        Token::Operator(op) => {
            p.advance();
            Spanned::new(p.span_from(op_start), op)
        }
        Token::Minus => {
            p.advance();
            Spanned::new(p.span_from(op_start), "-".into())
        }
        _ => return Err(p.error("expected operator in infix declaration")),
    };
    p.expect(&Token::RightParen)?;

    p.expect(&Token::Equals)?;
    let function = p.expect_lower_name()?;

    Ok(InfixDef {
        direction,
        precedence,
        operator,
        function,
    })
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

fn can_start_atomic_type(tok: &Token) -> bool {
    matches!(
        tok,
        Token::LowerName(_) | Token::UpperName(_) | Token::LeftParen | Token::LeftBrace
    )
}
