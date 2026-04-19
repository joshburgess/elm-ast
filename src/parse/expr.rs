use crate::comment::Comment;
use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, IfBranch, LetDeclaration, RecordSetter,
    Signature,
};
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::span::{Position, Span};
use crate::token::Token;

use super::pattern::parse_pattern;
use super::type_annotation::parse_type;
use super::{ParseResult, Parser};

// ── CPS types ────────────────────────────────────────────────────────

/// A boxed continuation: given the parser and a completed sub-expression,
/// produce the next step.
type Cont = Box<dyn FnOnce(&mut Parser, Spanned<Expr>) -> ParseResult<Step>>;

/// A step in the CPS trampoline. Either we have a finished expression,
/// or we need a sub-expression and have a continuation to resume with.
enum Step {
    Done(Spanned<Expr>),
    NeedExpr(Cont),
}

// ── Helper structs ───────────────────────────────────────────────────

struct PendingOp {
    left: Spanned<Expr>,
    op: String,
    assoc: InfixDirection,
    right_bp: u8,
}

struct IfChainEntry {
    start: Position,
    condition: Spanned<Expr>,
    then_branch: Spanned<Expr>,
    trailing_comments: Vec<Spanned<Comment>>,
}

enum RecordContext {
    Plain,
    Update(Spanned<String>),
}

// ── Trampoline ───────────────────────────────────────────────────────

/// Parse an expression.
///
/// Uses a CPS (continuation-passing style) trampoline to eliminate all
/// recursion. Every compound expression (if, case, let, lambda, paren,
/// list, record) that would normally call `parse_expr` recursively
/// instead returns a `Step::NeedExpr(continuation)`. The trampoline
/// pushes the continuation onto a heap-allocated stack and loops back
/// to parse the sub-expression from scratch. When a sub-expression
/// completes, the trampoline pops and invokes the continuation.
///
/// This guarantees O(1) call-stack depth regardless of expression
/// nesting. The continuation stack size is bounded by `MAX_EXPR_DEPTH`.
pub fn parse_expr(p: &mut Parser) -> ParseResult<Spanned<Expr>> {
    let mut stack: Vec<Cont> = Vec::new();

    'outer: loop {
        let step = parse_binary_expr_cps(p)?;
        let mut current = step;

        loop {
            match current {
                Step::NeedExpr(cont) => {
                    if stack.len() >= super::MAX_EXPR_DEPTH {
                        return Err(p.error(format!(
                            "expression nesting too deep (limit: {})",
                            super::MAX_EXPR_DEPTH
                        )));
                    }
                    stack.push(cont);
                    continue 'outer;
                }
                Step::Done(expr) => match stack.pop() {
                    None => return Ok(expr),
                    Some(cont) => {
                        current = cont(p, expr)?;
                    }
                },
            }
        }
    }
}

// ── Binary operator chain (iterative Pratt parser) ───────────────────

/// Start parsing a binary expression. Returns `Step::Done` if the full
/// expression was parsed without encountering a compound sub-expression,
/// or `Step::NeedExpr` if a compound form needs a sub-expression first.
fn parse_binary_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let step = parse_unary_expr_cps(p)?;
    match step {
        Step::Done(left) => binary_loop(p, Vec::new(), left),
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            binary_after_operand(p, step, Vec::new())
        }))),
    }
}

/// Continue the Pratt loop with a completed operand.
fn binary_loop(
    p: &mut Parser,
    mut pending: Vec<PendingOp>,
    mut left: Spanned<Expr>,
) -> ParseResult<Step> {
    loop {
        let left_end_line = left.span.end.line;
        let left_end_offset = left.span.end.offset;
        p.skip_whitespace();

        let (op, prec, assoc) = match extract_operator(p) {
            Some(info) => info,
            None => break,
        };

        // Claim any LINE comments positioned between `left` and the
        // operator whose start line is strictly after `left`'s end line.
        // For pipes these are the "step-preceding" comments elm-format
        // shows on their own line above the `|>`. The same pattern applies
        // to other binary operators when a line comment sits on its own
        // line between the operand and the next operator. Attach them as
        // leading comments on the right operand so they live with the
        // operator step through fold/unfold. Comments may have been
        // collected by an earlier `skip_whitespace` (e.g. the trailing one
        // in `application_loop`) before we reached here, so we scan by
        // source offset instead of a snapshot index.
        let mut step_leading: Vec<Spanned<Comment>> = Vec::new();
        {
            let mut i = 0;
            while i < p.collected_comments.len() {
                let c = &p.collected_comments[i];
                let is_line = matches!(c.value, Comment::Line(_));
                let after_left = c.span.start.offset >= left_end_offset;
                let before_op = c.span.end.offset <= p.current().span.start.offset;
                let different_line = c.span.start.line > left_end_line;
                if is_line && after_left && before_op && different_line {
                    step_leading.push(p.collected_comments.remove(i));
                } else {
                    i += 1;
                }
            }
        }

        let (left_bp, right_bp) = binding_power(prec, &assoc);

        // Fold pending operators whose right operand is now complete.
        while let Some(top) = pending.last() {
            if top.right_bp > left_bp {
                let top = pending.pop().unwrap();
                let start = top.left.span.start;
                // Compute end from the right operand itself, NOT from the
                // parser's current position — `skip_whitespace` at the top of
                // the loop may have advanced past trailing newlines, which
                // would make `spanned_from` leak the span into whitespace.
                let end = left.span.end;
                let expr = Expr::OperatorApplication {
                    operator: top.op,
                    direction: top.assoc,
                    left: Box::new(top.left),
                    right: Box::new(left),
                };
                left = Spanned::new(Span::new(start, end), expr);
            } else {
                break;
            }
        }

        p.advance(); // consume the operator token

        pending.push(PendingOp {
            left,
            op,
            assoc,
            right_bp,
        });

        let step = parse_unary_expr_cps(p)?;
        match step {
            Step::Done(mut expr) => {
                if !step_leading.is_empty() {
                    step_leading.extend(expr.comments);
                    expr.comments = step_leading;
                }
                left = expr;
            }
            Step::NeedExpr(cont) => {
                // Thread the claimed pipe-leading comments through the
                // continuation. Restoring to the pending buffer would fail:
                // when binary_loop resumes with the completed operand, the
                // comment's line is no longer strictly after `left`'s end,
                // so the reclaim check rejects it and the comment drops.
                let leading = step_leading;
                return Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
                    let step = cont(p, sub_expr)?;
                    binary_after_operand_with_leading(p, step, pending, leading)
                })));
            }
        }
    }

    // Fold all remaining pending operators. Same concern as the inner fold:
    // use the right operand's own span.end, not the parser position, because
    // the loop exited after `skip_whitespace` consumed trailing newlines.
    while let Some(top) = pending.pop() {
        let start = top.left.span.start;
        let end = left.span.end;
        let expr = Expr::OperatorApplication {
            operator: top.op,
            direction: top.assoc,
            left: Box::new(top.left),
            right: Box::new(left),
        };
        left = Spanned::new(Span::new(start, end), expr);
    }

    Ok(Step::Done(left))
}

/// Wrapper: when a compound form produces its operand, resume the binary loop.
fn binary_after_operand(p: &mut Parser, step: Step, pending: Vec<PendingOp>) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            binary_after_operand(p, step, pending)
        }))),
        Step::Done(left) => binary_loop(p, pending, left),
    }
}

/// Like `binary_after_operand`, but also prepends a set of leading comments
/// to the completed right operand before resuming the loop. Used when a pipe
/// operator claimed step-preceding line comments but the operand went through
/// CPS (so the Done-path attachment in `binary_loop` was skipped).
fn binary_after_operand_with_leading(
    p: &mut Parser,
    step: Step,
    pending: Vec<PendingOp>,
    leading: Vec<Spanned<Comment>>,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            binary_after_operand_with_leading(p, step, pending, leading)
        }))),
        Step::Done(mut left) => {
            if !leading.is_empty() {
                let mut combined = leading;
                combined.extend(std::mem::take(&mut left.comments));
                left.comments = combined;
            }
            binary_loop(p, pending, left)
        }
    }
}

// ── Operator helpers (unchanged) ─────────────────────────────────────

/// Extract operator info from current token if it's a binary operator.
/// Returns (operator_name, precedence, associativity).
fn extract_operator(p: &Parser) -> Option<(String, u8, InfixDirection)> {
    match p.peek() {
        Token::Operator(op) => {
            let (prec, assoc) = operator_info(op);
            Some((op.clone(), prec, assoc))
        }
        Token::Minus => Some(("-".into(), 6, InfixDirection::Left)),
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
    let base = prec * 2;
    match assoc {
        InfixDirection::Left => (base, base + 1),
        InfixDirection::Right => (base, base),
        InfixDirection::Non => (base, base + 1),
    }
}

// ── Unary expression ─────────────────────────────────────────────────

/// Parse a unary expression (prefix negation or function application).
fn parse_unary_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    p.skip_whitespace();
    let start = p.current_pos();

    if matches!(p.peek(), Token::Minus) {
        let minus_span = p.peek_span();
        p.advance();

        let step = parse_application_cps(p)?;
        match step {
            Step::Done(operand) => {
                let expr = Expr::Negation(Box::new(operand));
                Ok(Step::Done(Spanned::new(
                    minus_span.merge(p.span_from(start)),
                    expr,
                )))
            }
            Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
                let step = cont(p, sub_expr)?;
                unary_wrap_negation(p, step, start, minus_span)
            }))),
        }
    } else {
        parse_application_cps(p)
    }
}

/// Wrapper: when a compound form inside negation produces its operand, apply negation.
fn unary_wrap_negation(
    p: &mut Parser,
    step: Step,
    start: Position,
    minus_span: Span,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            unary_wrap_negation(p, step, start, minus_span)
        }))),
        Step::Done(operand) => {
            let expr = Expr::Negation(Box::new(operand));
            Ok(Step::Done(Spanned::new(
                minus_span.merge(p.span_from(start)),
                expr,
            )))
        }
    }
}

// ── Application ──────────────────────────────────────────────────────

/// Parse function application: `f x y z`
fn parse_application_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    // Consume app_context_col (set by list/record parsers) so that nested
    // expression parsing doesn't inherit it. The value is threaded through
    // the CPS continuations for this application only.
    let ctx_col = p.app_context_col.take();
    let first_step = parse_atomic_expr_cps(p)?;

    match first_step {
        Step::Done(first) => {
            let first_line = first.span.start.line;
            application_loop(p, start, first_line, vec![first], ctx_col)
        }
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            app_after_first_atom(p, step, start, ctx_col)
        }))),
    }
}

/// Wrapper: when the first atom of an application comes from a compound form.
fn app_after_first_atom(
    p: &mut Parser,
    step: Step,
    start: Position,
    ctx_col: Option<u32>,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            app_after_first_atom(p, step, start, ctx_col)
        }))),
        Step::Done(first) => {
            let first_line = first.span.start.line;
            application_loop(p, start, first_line, vec![first], ctx_col)
        }
    }
}

/// Iterative application loop: collect arguments.
fn application_loop(
    p: &mut Parser,
    start: Position,
    first_line: u32,
    mut args: Vec<Spanned<Expr>>,
    ctx_col: Option<u32>,
) -> ParseResult<Step> {
    loop {
        let pre_ws_snapshot = p.pending_comments_snapshot();
        p.skip_whitespace();
        // Whitespace must precede `-` for it to be treated as unary negation
        // consumed as an arg (`f -x`). Otherwise `n-1` is binary subtraction.
        let prev_end = args.last().map(|a| a.span.end.offset).unwrap_or(0);
        let has_ws_before_minus = p.current().span.start.offset > prev_end;
        let is_unary_neg_arg = has_ws_before_minus && is_unary_minus_arg(p);
        if !can_start_atomic_expr(p.peek()) && !is_unary_neg_arg {
            break;
        }
        let arg_col = p.current_column();
        let arg_line = p.current_pos().line;
        // When ctx_col is set (inside list/record), use the bracket's
        // column as the reference — arguments past the bracket are valid.
        // Otherwise, require args to be strictly more indented than the
        // function to avoid consuming sibling declarations or case branches.
        let ref_col = ctx_col.unwrap_or(start.column);
        if arg_line != first_line && arg_col <= ref_col {
            break;
        }
        // Claim inter-arg comments collected between the previous arg and
        // this one. Two cases:
        //  1. INLINE BLOCK comments on the same line as prev's end
        //     (e.g. `f 0x30 {- 0 -} x`).
        //  2. LINE or BLOCK comments on dedicated lines between prev and
        //     the current arg — these are leading comments on the new arg.
        // Comments on the same line as prev's end that aren't inline block
        // comments (i.e., line comments trailing prev) remain in pending.
        let mut inter_arg_comments: Vec<Spanned<Comment>> = Vec::new();
        if let Some(prev) = args.last() {
            let prev_end_line = prev.span.end.line;
            let mut i = pre_ws_snapshot;
            while i < p.collected_comments.len() {
                let c = &p.collected_comments[i];
                let is_inline_block = matches!(c.value, Comment::Block(_))
                    && c.span.start.line == prev_end_line
                    && c.span.end.line == prev_end_line;
                let is_on_own_line = c.span.start.line > prev_end_line;
                if is_inline_block || is_on_own_line {
                    inter_arg_comments.push(p.collected_comments.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        let step = if is_unary_neg_arg {
            // Consume `-`, then parse atomic arg, wrap in Negation.
            let minus_start = p.current_pos();
            let minus_span = p.peek_span();
            p.advance();
            let inner_step = parse_atomic_expr_cps(p)?;
            match inner_step {
                Step::Done(operand) => {
                    let expr = Expr::Negation(Box::new(operand));
                    Step::Done(Spanned::new(
                        minus_span.merge(p.span_from(minus_start)),
                        expr,
                    ))
                }
                Step::NeedExpr(cont) => Step::NeedExpr(Box::new(move |p, sub_expr| {
                    let inner = cont(p, sub_expr)?;
                    unary_wrap_negation(p, inner, minus_start, minus_span)
                })),
            }
        } else {
            parse_atomic_expr_cps(p)?
        };
        match step {
            Step::Done(mut arg) => {
                if !inter_arg_comments.is_empty() {
                    inter_arg_comments.extend(arg.comments);
                    arg.comments = inter_arg_comments;
                }
                args.push(arg);
            }
            Step::NeedExpr(cont) => {
                // Thread the claimed inter-arg comments through the
                // continuation. Restoring to the pending buffer fails the
                // round-trip: when application_loop resumes with the Done
                // arg, the next iteration's prev_end_line is the new arg's
                // end (not the original prev arg's), so the reclaim check
                // rejects the comment and it drops.
                let leading = inter_arg_comments;
                return Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
                    let step = cont(p, sub_expr)?;
                    app_after_arg_with_leading(p, step, start, first_line, args, ctx_col, leading)
                })));
            }
        }
    }

    if args.len() == 1 {
        Ok(Step::Done(args.into_iter().next().unwrap()))
    } else {
        Ok(Step::Done(p.spanned_from(start, Expr::Application(args))))
    }
}

/// Wrapper: when an application argument comes from a compound form.
/// Prepends any claimed inter-arg leading comments to the completed argument
/// before it joins the args list. The `leading` vector is empty in the common
/// case where no inter-arg comments were claimed.
fn app_after_arg_with_leading(
    p: &mut Parser,
    step: Step,
    start: Position,
    first_line: u32,
    args: Vec<Spanned<Expr>>,
    ctx_col: Option<u32>,
    leading: Vec<Spanned<Comment>>,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            app_after_arg_with_leading(p, step, start, first_line, args, ctx_col, leading)
        }))),
        Step::Done(mut arg) => {
            if !leading.is_empty() {
                let mut combined = leading;
                combined.extend(std::mem::take(&mut arg.comments));
                arg.comments = combined;
            }
            let mut args = args;
            args.push(arg);
            application_loop(p, start, first_line, args, ctx_col)
        }
    }
}

// ── Atomic expressions ───────────────────────────────────────────────

/// Parse an atomic expression. Returns `Step::Done` for simple forms,
/// `Step::NeedExpr` for compound forms that need sub-expressions.
fn parse_atomic_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    p.skip_whitespace();
    let start = p.current_pos();

    let step = match p.peek().clone() {
        // ── Literals ─────────────────────────────────────────────
        Token::Literal(lit) => {
            p.advance();
            Step::Done(p.spanned_from(start, Expr::Literal(lit)))
        }

        // ── Lowercase name (value reference) ─────────────────────
        Token::LowerName(name) => {
            p.advance();
            Step::Done(p.spanned_from(
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
            Step::Done(p.spanned_from(start, Expr::FunctionOrValue { module_name, name }))
        }

        // ── Paren / tuple / prefix op / unit ─────────────────────
        Token::LeftParen => parse_paren_cps(p, start)?,

        // ── Record or record update ──────────────────────────────
        Token::LeftBrace => parse_record_expr_cps(p)?,

        // ── List ─────────────────────────────────────────────────
        Token::LeftBracket => parse_list_cps(p, start)?,

        // ── GLSL block ──────────────────────────────────────────
        Token::Glsl(src) => {
            p.advance();
            Step::Done(p.spanned_from(start, Expr::GLSLExpression(src)))
        }

        // ── If-then-else ─────────────────────────────────────────
        Token::If => parse_if_expr_cps(p)?,

        // ── Case-of ──────────────────────────────────────────────
        Token::Case => parse_case_expr_cps(p)?,

        // ── Let-in ───────────────────────────────────────────────
        Token::Let => parse_let_expr_cps(p)?,

        // ── Lambda ───────────────────────────────────────────────
        Token::Backslash => parse_lambda_expr_cps(p)?,

        // ── Record access function: `.name` ──────────────────────
        Token::Dot => {
            p.advance();
            match p.peek().clone() {
                Token::LowerName(name) => {
                    p.advance();
                    Step::Done(p.spanned_from(start, Expr::RecordAccessFunction(name)))
                }
                _ => return Err(p.error("expected field name after `.`")),
            }
        }

        _ => {
            return Err(p.error(format!(
                "expected expression, found {}",
                super::describe(p.peek())
            )));
        }
    };

    // Apply record access chain (.field) to the result.
    Ok(match step {
        Step::Done(expr) => Step::Done(parse_record_access_chain(p, expr)?),
        Step::NeedExpr(cont) => Step::NeedExpr(Box::new(move |p, sub_expr| {
            let inner = cont(p, sub_expr)?;
            apply_record_access(p, inner)
        })),
    })
}

/// Wrapper: apply record access chain when a compound form eventually produces Done.
fn apply_record_access(p: &mut Parser, step: Step) -> ParseResult<Step> {
    match step {
        Step::Done(expr) => Ok(Step::Done(parse_record_access_chain(p, expr)?)),
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let inner = cont(p, sub_expr)?;
            apply_record_access(p, inner)
        }))),
    }
}

/// After parsing an atomic expression, check for `.field` access chains.
fn parse_record_access_chain(
    p: &mut Parser,
    mut expr: Spanned<Expr>,
) -> ParseResult<Spanned<Expr>> {
    // Don't skip whitespace — `.field` must be immediately adjacent.
    while matches!(p.peek(), Token::Dot) {
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

// ── If-then-else (CPS) ──────────────────────────────────────────────

fn parse_if_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    p.expect(&Token::If)?;
    // Snapshot before parsing the condition so that any comments appearing
    // between `if` and the condition expression get attached as leading
    // comments on the condition. elm-format places them on their own line
    // above the condition inside the `if ... then` block.
    let cond_snapshot = p.pending_comments_snapshot();
    Ok(Step::NeedExpr(Box::new(move |p, mut condition| {
        attach_pre_body_comments(p, &mut condition, cond_snapshot);
        if_after_condition(p, start, Vec::new(), condition)
    })))
}

fn if_after_condition(
    p: &mut Parser,
    start: Position,
    chain: Vec<IfChainEntry>,
    condition: Spanned<Expr>,
) -> ParseResult<Step> {
    p.expect(&Token::Then)?;
    // Snapshot before skipping whitespace so any comments consumed between
    // `then` and the branch body ride along as leading comments on the
    // then-branch expression.
    let then_snapshot = p.pending_comments_snapshot();
    p.skip_whitespace();
    Ok(Step::NeedExpr(Box::new(move |p, then_branch| {
        if_after_then(p, start, chain, condition, then_branch, then_snapshot)
    })))
}

fn if_after_then(
    p: &mut Parser,
    start: Position,
    mut chain: Vec<IfChainEntry>,
    condition: Spanned<Expr>,
    mut then_branch: Spanned<Expr>,
    then_snapshot: usize,
) -> ParseResult<Step> {
    attach_pre_body_comments(p, &mut then_branch, then_snapshot);
    // Pending comments left by attach_pre_body_comments are post-body (i.e.
    // between `then_branch` end and the upcoming `else` keyword). Calling
    // `expect(Else)` below also skips whitespace which may collect a few
    // more. Snapshot now captures the post-body set as trailing_comments.
    p.expect(&Token::Else)?;
    let trailing_comments = p.take_pending_comments_since(then_snapshot);
    chain.push(IfChainEntry {
        start,
        condition,
        then_branch,
        trailing_comments,
    });
    let else_snapshot = p.pending_comments_snapshot();
    p.skip_whitespace();

    if matches!(p.peek(), Token::If) {
        // Chained if-else: parse next condition
        let next_start = p.current_pos();
        p.expect(&Token::If)?;
        Ok(Step::NeedExpr(Box::new(move |p, condition| {
            if_after_condition(p, next_start, chain, condition)
        })))
    } else {
        // Final else branch
        Ok(Step::NeedExpr(Box::new(move |p, else_branch| {
            let mut result = else_branch;
            attach_pre_body_comments(p, &mut result, else_snapshot);
            // Fold from right to left to build nested IfElse structure.
            for entry in chain.into_iter().rev() {
                result = p.spanned_from(
                    entry.start,
                    Expr::IfElse {
                        branches: vec![IfBranch {
                            condition: entry.condition,
                            then_branch: entry.then_branch,
                            trailing_comments: entry.trailing_comments,
                        }],
                        else_branch: Box::new(result),
                    },
                );
            }
            Ok(Step::Done(result))
        })))
    }
}

/// Take comments collected since `snapshot` that appear before `body.span.start`
/// and prepend them as leading comments on `body`. Comments after the body
/// stay in the pending buffer for the next enclosing context to claim.
pub(super) fn attach_pre_body_comments(p: &mut Parser, body: &mut Spanned<Expr>, snapshot: usize) {
    let body_start = body.span.start.offset;
    let pending = p.take_pending_comments_since(snapshot);
    if pending.is_empty() {
        return;
    }
    let mut pre: Vec<Spanned<Comment>> = Vec::new();
    let mut post: Vec<Spanned<Comment>> = Vec::new();
    for c in pending {
        if c.span.end.offset <= body_start {
            pre.push(c);
        } else {
            post.push(c);
        }
    }
    if !post.is_empty() {
        p.restore_pending_comments(post);
    }
    if !pre.is_empty() {
        let mut combined = pre;
        combined.extend(std::mem::take(&mut body.comments));
        body.comments = combined;
    }
}

// ── Case-of (CPS) ───────────────────────────────────────────────────

fn parse_case_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    // Snapshot the pending-comments buffer so branch-comment capture
    // only picks up comments that appear inside this case expression,
    // not module-level or earlier comments already pending.
    let comments_snapshot = p.pending_comments_snapshot();
    p.expect(&Token::Case)?;
    // Need subject
    Ok(Step::NeedExpr(Box::new(move |p, subject| {
        case_after_subject(p, start, subject, comments_snapshot)
    })))
}

fn case_after_subject(
    p: &mut Parser,
    start: Position,
    subject: Spanned<Expr>,
    comments_snapshot: usize,
) -> ParseResult<Step> {
    p.expect(&Token::Of)?;
    p.skip_whitespace();
    let branch_col = p.current_column();

    case_next_branch(p, start, subject, Vec::new(), branch_col, comments_snapshot)
}

fn case_next_branch(
    p: &mut Parser,
    start: Position,
    subject: Spanned<Expr>,
    branches: Vec<CaseBranch>,
    branch_col: u32,
    comments_snapshot: usize,
) -> ParseResult<Step> {
    p.skip_whitespace();

    // Check if we should parse another branch.
    if !case_should_continue(p, &branches, branch_col, start) {
        if branches.is_empty() {
            return Err(p.error("expected at least one case branch"));
        }
        return Ok(Step::Done(p.spanned_from(
            start,
            Expr::CaseOf {
                expr: Box::new(subject),
                branches,
            },
        )));
    }

    // Attach comments to the branch pattern. Only take comments that were
    // collected after the case expression started — leave outer comments
    // in place for the enclosing module/declaration to pick up.
    let branch_comments = p.take_pending_comments_since(comments_snapshot);
    let mut pat = parse_pattern(p)?;
    if !branch_comments.is_empty() {
        pat.comments = branch_comments;
    }
    p.expect(&Token::Arrow)?;

    // Clear the application context column so that the branch body uses
    // normal column checking (prevents over-consuming the next branch pattern).
    p.app_context_col = None;

    // Snapshot so comments consumed between `->` and the branch body ride
    // along as leading comments on the branch body expression rather than
    // leaking to the next branch's pattern.
    let body_snapshot = p.pending_comments_snapshot();

    // Need branch body
    Ok(Step::NeedExpr(Box::new(move |p, mut body| {
        attach_pre_body_comments(p, &mut body, body_snapshot);
        let mut branches = branches;
        branches.push(CaseBranch { pattern: pat, body });
        case_next_branch(p, start, subject, branches, branch_col, comments_snapshot)
    })))
}

fn case_should_continue(
    p: &mut Parser,
    branches: &[CaseBranch],
    branch_col: u32,
    start: Position,
) -> bool {
    if p.is_eof() {
        return false;
    }
    let col = p.current_column();
    if p.in_paren_context() {
        if !branches.is_empty() && col != branch_col {
            return false;
        }
    } else {
        if col < branch_col {
            return false;
        }
        if col < start.column + 1 && !branches.is_empty() {
            return false;
        }
    }
    can_start_pattern(p.peek())
}

// ── Let-in (CPS) ────────────────────────────────────────────────────

fn parse_let_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    // Snapshot pending comments so that per-let-decl comment capture
    // only takes comments collected inside this let block.
    let comments_snapshot = p.pending_comments_snapshot();
    p.expect(&Token::Let)?;

    let let_col = start.column;
    p.skip_whitespace();
    let decl_col = p.current_column();

    let_next_decl(p, start, let_col, decl_col, Vec::new(), comments_snapshot)
}

fn let_next_decl(
    p: &mut Parser,
    start: Position,
    let_col: u32,
    decl_col: u32,
    declarations: Vec<Spanned<LetDeclaration>>,
    comments_snapshot: usize,
) -> ParseResult<Step> {
    p.skip_whitespace();

    // Check if we're done with declarations.
    if p.is_eof()
        || matches!(p.peek(), Token::In)
        || (!p.in_paren_context() && p.current_column() < decl_col)
        || (!p.in_paren_context() && p.current_column() < let_col + 1)
    {
        // Any comments captured between the last declaration and the `in`
        // keyword stick to the let block as trailing comments.
        let trailing_comments = p.take_pending_comments_since(comments_snapshot);
        p.expect(&Token::In)?;
        // Clear app_context_col for the `in` body.
        p.app_context_col = None;
        let in_body_snapshot = p.pending_comments_snapshot();
        // Need body
        return Ok(Step::NeedExpr(Box::new(move |p, mut body| {
            attach_pre_body_comments(p, &mut body, in_body_snapshot);
            Ok(Step::Done(p.spanned_from(
                start,
                Expr::LetIn {
                    declarations,
                    body: Box::new(body),
                    trailing_comments,
                },
            )))
        })));
    }

    // Attach comments before this let declaration. Only take comments that
    // were collected after the let expression started — outer comments
    // remain available for the enclosing context.
    let let_decl_comments = p.take_pending_comments_since(comments_snapshot);
    let decl_start = p.current_pos();

    match p.peek().clone() {
        Token::LowerName(_) => {
            let next = p.peek_nth_past_whitespace(1);
            if matches!(next, Token::Colon) {
                // Type signature followed by function implementation.
                let sig = parse_signature(p)?;
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

                // Clear app_context_col for the function body.
                p.app_context_col = None;

                let body_snapshot = p.pending_comments_snapshot();
                // Need function body
                Ok(Step::NeedExpr(Box::new(move |p, mut body| {
                    attach_pre_body_comments(p, &mut body, body_snapshot);
                    let implementation = FunctionImplementation { name, args, body };
                    let func = Function {
                        documentation: None,
                        signature: Some(sig),
                        declaration: p.spanned_from(impl_start, implementation),
                    };
                    let decl = LetDeclaration::Function(Box::new(func));
                    let mut spanned_decl = p.spanned_from(decl_start, decl);
                    if !let_decl_comments.is_empty() {
                        spanned_decl.comments = let_decl_comments;
                    }
                    let mut declarations = declarations;
                    declarations.push(spanned_decl);
                    let_next_decl(p, start, let_col, decl_col, declarations, comments_snapshot)
                })))
            } else {
                // Function implementation (no signature).
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

                // Clear app_context_col for the function body.
                p.app_context_col = None;

                let body_snapshot = p.pending_comments_snapshot();
                // Need function body
                Ok(Step::NeedExpr(Box::new(move |p, mut body| {
                    attach_pre_body_comments(p, &mut body, body_snapshot);
                    let implementation = FunctionImplementation { name, args, body };
                    let func = Function {
                        documentation: None,
                        signature: None,
                        declaration: p.spanned_from(impl_start, implementation),
                    };
                    let decl = LetDeclaration::Function(Box::new(func));
                    let mut spanned_decl = p.spanned_from(decl_start, decl);
                    if !let_decl_comments.is_empty() {
                        spanned_decl.comments = let_decl_comments;
                    }
                    let mut declarations = declarations;
                    declarations.push(spanned_decl);
                    let_next_decl(p, start, let_col, decl_col, declarations, comments_snapshot)
                })))
            }
        }
        _ => {
            // Destructuring pattern.
            let pattern = parse_pattern(p)?;
            p.expect(&Token::Equals)?;

            // Clear app_context_col for the destructuring body.
            p.app_context_col = None;

            let body_snapshot = p.pending_comments_snapshot();
            // Need destructuring body
            Ok(Step::NeedExpr(Box::new(move |p, mut body| {
                attach_pre_body_comments(p, &mut body, body_snapshot);
                let decl = LetDeclaration::Destructuring {
                    pattern: Box::new(pattern),
                    body: Box::new(body),
                };
                let mut spanned_decl = p.spanned_from(decl_start, decl);
                if !let_decl_comments.is_empty() {
                    spanned_decl.comments = let_decl_comments;
                }
                let mut declarations = declarations;
                declarations.push(spanned_decl);
                let_next_decl(p, start, let_col, decl_col, declarations, comments_snapshot)
            })))
        }
    }
}

fn parse_signature(p: &mut Parser) -> ParseResult<Spanned<Signature>> {
    let start = p.current_pos();
    let name = p.expect_lower_name()?;
    p.expect(&Token::Colon)?;
    let type_annotation = parse_type(p)?;
    let trailing_comment = {
        let end_line = type_annotation.span.end.line;
        let end_offset = type_annotation.span.end.offset;
        if let Some(last) = p.collected_comments.last()
            && last.span.start.line == end_line
            && last.span.start.offset >= end_offset
        {
            p.collected_comments.pop()
        } else {
            None
        }
    };
    Ok(p.spanned_from(
        start,
        Signature {
            name,
            type_annotation,
            trailing_comment,
        },
    ))
}

// ── Lambda (CPS) ────────────────────────────────────────────────────

fn parse_lambda_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    p.expect(&Token::Backslash)?;

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

    // Snapshot BEFORE parsing the body so a line comment sitting between the
    // arrow and the body (e.g. a note above a `let`, `if`, or `case`) can be
    // attached to the body as a leading comment.
    let body_snapshot = p.pending_comments_snapshot();

    // Need body
    Ok(Step::NeedExpr(Box::new(move |p, mut body| {
        attach_pre_body_comments(p, &mut body, body_snapshot);
        Ok(Step::Done(p.spanned_from(
            start,
            Expr::Lambda {
                args,
                body: Box::new(body),
            },
        )))
    })))
}

// ── Paren / Tuple / Prefix operator / Unit (CPS) ────────────────────

fn parse_paren_cps(p: &mut Parser, start: Position) -> ParseResult<Step> {
    p.advance(); // consume `(`
    // Snapshot BEFORE skip_whitespace so any leading line comments between
    // `(` and the first element can be attached to the first element.
    let first_snapshot = p.pending_comments_snapshot();
    p.skip_whitespace();

    // Unit: `()`
    if matches!(p.peek(), Token::RightParen) {
        p.advance();
        return Ok(Step::Done(p.spanned_from(start, Expr::Unit)));
    }

    // Prefix operator: `(+)`, `(::)`, `(-)`
    match p.peek().clone() {
        Token::Operator(op) => {
            p.advance();
            p.skip_whitespace();
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(Step::Done(p.spanned_from(start, Expr::PrefixOperator(op))));
            }
            return Err(p.error("expected `)` after operator in prefix expression"));
        }
        Token::Minus => {
            let minus_start = p.current_pos();
            p.advance();
            p.skip_whitespace();
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(Step::Done(
                    p.spanned_from(start, Expr::PrefixOperator("-".into())),
                ));
            }
            // Negation inside parens binds to the application level only,
            // so `(-2 * x)` parses as `BinOp(Negation(2), *, x)`, not
            // `Negation(BinOp(2, *, x))`.
            let app_step = parse_application_cps(p)?;
            return paren_neg_after_app(p, start, minus_start, app_step);
        }
        _ => {}
    }

    // Regular parenthesized expression or tuple.
    Ok(Step::NeedExpr(Box::new(move |p, first| {
        paren_after_first(p, start, first, first_snapshot)
    })))
}

fn paren_neg_after_app(
    p: &mut Parser,
    paren_start: Position,
    minus_start: Position,
    step: Step,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            paren_neg_after_app(p, paren_start, minus_start, step)
        }))),
        Step::Done(operand) => {
            let neg_span = Span::new(minus_start, operand.span.end);
            let neg_spanned = Spanned::new(neg_span, Expr::Negation(Box::new(operand)));
            let bin_step = binary_loop(p, Vec::new(), neg_spanned)?;
            // `( -expr ... )` doesn't have a leading comment between `(`
            // and the first expr slot, so there's nothing to attach — use
            // the current snapshot as a no-op anchor.
            let snap = p.pending_comments_snapshot();
            paren_after_binary(p, paren_start, bin_step, snap)
        }
    }
}

fn paren_after_binary(
    p: &mut Parser,
    paren_start: Position,
    step: Step,
    first_snapshot: usize,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            paren_after_binary(p, paren_start, step, first_snapshot)
        }))),
        Step::Done(first) => paren_after_first(p, paren_start, first, first_snapshot),
    }
}

fn paren_after_first(
    p: &mut Parser,
    start: Position,
    mut first: Spanned<Expr>,
    first_snapshot: usize,
) -> ParseResult<Step> {
    attach_pre_body_comments(p, &mut first, first_snapshot);
    p.skip_whitespace();
    match p.peek() {
        Token::Comma => {
            p.advance(); // consume comma
            let elements = vec![first];
            // Snapshot BEFORE skip_whitespace / next-element parsing so
            // comments between `,` and the next element are captured.
            let next_snapshot = p.pending_comments_snapshot();
            Ok(Step::NeedExpr(Box::new(move |p, next| {
                tuple_after_element(p, start, elements, next, next_snapshot)
            })))
        }
        Token::RightParen => {
            // Capture comments between the inner expression's end and the
            // closing `)` as trailing comments on the Parenthesized wrapper.
            // elm-format keeps a dangling `-- comment` before `)` visible:
            //     (   inner
            //      -- trailing
            //     )
            let first_end = first.span.end.offset;
            let rparen_start = p.peek_span().start.offset;
            let all = p.take_pending_comments_since(0);
            let (trailing, keep): (Vec<_>, Vec<_>) = all.into_iter().partition(|c| {
                c.span.start.offset >= first_end && c.span.end.offset <= rparen_start
            });
            p.restore_pending_comments(keep);
            p.advance();
            Ok(Step::Done(p.spanned_from(
                start,
                Expr::Parenthesized {
                    expr: Box::new(first),
                    trailing_comments: trailing,
                },
            )))
        }
        _ => Err(p.error("expected `,` or `)` in expression")),
    }
}

fn tuple_after_element(
    p: &mut Parser,
    start: Position,
    mut elements: Vec<Spanned<Expr>>,
    mut elem: Spanned<Expr>,
    elem_snapshot: usize,
) -> ParseResult<Step> {
    // Snapshot BEFORE attach_pre_body_comments so any post-body comments
    // restored via restore_pending_comments remain visible to the next
    // element's snapshot (see list_after_element for details).
    let next_snapshot = elem_snapshot;
    attach_pre_body_comments(p, &mut elem, elem_snapshot);
    elements.push(elem);
    p.skip_whitespace();
    if p.eat(&Token::Comma) {
        // More elements
        Ok(Step::NeedExpr(Box::new(move |p, next| {
            tuple_after_element(p, start, elements, next, next_snapshot)
        })))
    } else {
        p.expect(&Token::RightParen)?;
        Ok(Step::Done(p.spanned_from(start, Expr::Tuple(elements))))
    }
}

// ── List (CPS) ───────────────────────────────────────────────────────

fn parse_list_cps(p: &mut Parser, start: Position) -> ParseResult<Step> {
    let bracket_col = start.column;
    p.advance(); // consume `[`
    // Snapshot BEFORE skip_whitespace so any leading line comments between
    // `[` and the first element can be attached to the first element via
    // `attach_pre_body_comments`.
    let elem_snapshot = p.pending_comments_snapshot();
    p.skip_whitespace();

    if matches!(p.peek(), Token::RightBracket) {
        p.advance();
        return Ok(Step::Done(p.spanned_from(
            start,
            Expr::List {
                elements: Vec::new(),
                element_inline_comments: Vec::new(),
                trailing_comments: Vec::new(),
            },
        )));
    }

    // Set the application context column to the bracket's column so that
    // function args at any column past the bracket are collected.
    p.app_context_col = Some(bracket_col);

    // Need first element
    Ok(Step::NeedExpr(Box::new(move |p, first| {
        list_after_element(
            p,
            start,
            bracket_col,
            Vec::new(),
            Vec::new(),
            first,
            elem_snapshot,
        )
    })))
}

/// Capture an inline trailing `-- comment` sitting on the same source line
/// as `elem` (between `elem` end and the next `,` / `]`). Such a comment
/// would otherwise be parked in `pending_comments` and attach to the next
/// element as a leading comment, which breaks elm-format's layout:
///     [ a -- inline
///     , b
///     ]
fn take_element_inline_trailing(p: &mut Parser, elem: &Spanned<Expr>) -> Option<Spanned<Comment>> {
    let elem_end_line = elem.span.end.line;
    let all = p.take_pending_comments_since(0);
    let mut inline: Option<Spanned<Comment>> = None;
    let mut keep: Vec<Spanned<Comment>> = Vec::with_capacity(all.len());
    for c in all {
        if inline.is_none()
            && c.span.start.line == elem_end_line
            && c.span.start.offset >= elem.span.end.offset
        {
            inline = Some(c);
        } else {
            keep.push(c);
        }
    }
    p.restore_pending_comments(keep);
    inline
}

fn list_after_element(
    p: &mut Parser,
    start: Position,
    bracket_col: u32,
    mut elements: Vec<Spanned<Expr>>,
    mut element_inline_comments: Vec<Option<Spanned<Comment>>>,
    mut elem: Spanned<Expr>,
    elem_snapshot: usize,
) -> ParseResult<Step> {
    // Snapshot BEFORE `attach_pre_body_comments`. The `attach_pre_body_comments`
    // call may push post-body comments back into pending (via
    // `restore_pending_comments`) at index `elem_snapshot..`; we want the
    // next element's snapshot to cover those. Any pre-body comments were
    // moved onto `elem`, so they won't leak into the next element.
    let next_snapshot = elem_snapshot;
    attach_pre_body_comments(p, &mut elem, elem_snapshot);
    // Peek for an inline same-line trailing comment on this element before
    // looking at `,` / `]`.
    let inline = take_element_inline_trailing(p, &elem);
    element_inline_comments.push(inline);
    elements.push(elem);
    if p.eat(&Token::Comma) {
        // Re-set the context column for the next element.
        p.app_context_col = Some(bracket_col);
        Ok(Step::NeedExpr(Box::new(move |p, next| {
            list_after_element(
                p,
                start,
                bracket_col,
                elements,
                element_inline_comments,
                next,
                next_snapshot,
            )
        })))
    } else {
        // Capture comments between the last element's end and the closing `]`
        // as trailing comments on the List. Only comments strictly on a later
        // line than the last element qualify — inline same-line comments
        // have already been claimed as element_inline_comments above.
        //     [ a
        //     , b
        //     -- trailing     <- captured here
        //     ]
        let last_line = elements.last().map(|e| e.span.end.line).unwrap_or(0);
        let rbracket_line = p.peek_span().start.line;
        let all = p.take_pending_comments_since(0);
        let (trailing, keep): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|c| c.span.start.line > last_line && c.span.end.line < rbracket_line);
        p.restore_pending_comments(keep);
        p.expect(&Token::RightBracket)?;
        // Drop the inline vec entirely if all None, to keep ASTs stable for
        // files that don't use this feature.
        let element_inline_comments = if element_inline_comments.iter().all(|c| c.is_none()) {
            Vec::new()
        } else {
            element_inline_comments
        };
        Ok(Step::Done(p.spanned_from(
            start,
            Expr::List {
                elements,
                element_inline_comments,
                trailing_comments: trailing,
            },
        )))
    }
}

// ── Record / Record update (CPS) ────────────────────────────────────

fn parse_record_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    p.expect(&Token::LeftBrace)?;
    p.skip_whitespace();

    // Empty record: `{}`
    if matches!(p.peek(), Token::RightBrace) {
        p.advance();
        return Ok(Step::Done(p.spanned_from(start, Expr::Record(Vec::new()))));
    }

    // Check for record update: `{ name | ... }`
    if matches!(p.peek(), Token::LowerName(_)) {
        let save_pos = p.pos;
        if let Ok(base_name) = p.expect_lower_name() {
            p.skip_whitespace();
            if matches!(p.peek(), Token::Pipe) {
                p.advance(); // consume `|`
                // Boundary just past `|` so comments between `|` and the
                // first field can be claimed as leading on that setter.
                let first_boundary = p.prev_token_end_offset();
                return record_parse_setter(
                    p,
                    start,
                    Vec::new(),
                    RecordContext::Update(base_name),
                    first_boundary,
                );
            }
            // Not a record update — backtrack.
            p.pos = save_pos;
        } else {
            p.pos = save_pos;
        }
    }

    // Regular record expression. The initial boundary sits just past the
    // opening `{` so that a comment between `{` and the first field can be
    // claimed as leading on that setter.
    let first_boundary = start.offset + 1;
    record_parse_setter(p, start, Vec::new(), RecordContext::Plain, first_boundary)
}

fn record_parse_setter(
    p: &mut Parser,
    rec_start: Position,
    setters: Vec<Spanned<RecordSetter>>,
    context: RecordContext,
    prev_boundary_offset: usize,
) -> ParseResult<Step> {
    let setter_start = p.current_pos();
    let field = p.expect_lower_name()?;
    p.expect(&Token::Equals)?;

    // Set the application context column to the brace's column so that
    // function args at any column past the brace are collected.
    p.app_context_col = Some(rec_start.column);

    // Need field value
    Ok(Step::NeedExpr(Box::new(move |p, value| {
        record_after_value(
            p,
            rec_start,
            setters,
            setter_start,
            field,
            value,
            context,
            prev_boundary_offset,
        )
    })))
}

#[allow(clippy::too_many_arguments)]
fn record_after_value(
    p: &mut Parser,
    rec_start: Position,
    mut setters: Vec<Spanned<RecordSetter>>,
    setter_start: Position,
    field: Spanned<String>,
    value: Spanned<Expr>,
    context: RecordContext,
    prev_boundary_offset: usize,
) -> ParseResult<Step> {
    let value_end_offset = value.span.end.offset;
    // Claim a trailing inline comment on the same source line as the value end,
    // e.g. `field = expr -- comment`. `binary_loop`'s post-value `skip_whitespace`
    // will already have collected it into `collected_comments`.
    let trailing_comment = {
        let value_end_line = value.span.end.line;
        let pending = &p.collected_comments;
        if let Some(last) = pending.last()
            && last.span.start.line == value_end_line
            && last.span.start.offset >= value_end_offset
        {
            p.collected_comments.pop()
        } else {
            None
        }
    };
    // Claim any pending comments positioned between the previous record
    // boundary (the `{`, `|`, or the prior value's end) and this setter's
    // field start as leading comments on this setter.
    let field_start_offset = setter_start.offset;
    let mut leading: Vec<Spanned<Comment>> = Vec::new();
    let mut i = 0;
    while i < p.collected_comments.len() {
        let c = &p.collected_comments[i];
        if c.span.start.offset >= prev_boundary_offset && c.span.end.offset <= field_start_offset {
            leading.push(p.collected_comments.remove(i));
        } else {
            i += 1;
        }
    }
    // Claim any pending comments that appear between this setter's field
    // start and the value start as leading comments on the value, e.g.
    // `field =\n    -- comment\n    value`.
    let value_start_offset = value.span.start.offset;
    let mut value_leading: Vec<Spanned<Comment>> = Vec::new();
    let mut i = 0;
    while i < p.collected_comments.len() {
        let c = &p.collected_comments[i];
        if c.span.start.offset >= field_start_offset && c.span.end.offset <= value_start_offset {
            value_leading.push(p.collected_comments.remove(i));
        } else {
            i += 1;
        }
    }
    let mut value = value;
    if !value_leading.is_empty() {
        let mut merged = value_leading;
        merged.extend(value.comments);
        value.comments = merged;
    }
    let setter = RecordSetter {
        field,
        value,
        trailing_comment,
    };
    let mut spanned = p.spanned_from(setter_start, setter);
    if !leading.is_empty() {
        spanned.comments = leading;
    }
    setters.push(spanned);

    // Track the boundary offset (end of this value) so the next setter can
    // claim any comments that appear between this value and its own field
    // name as leading comments.
    let next_boundary_offset = value_end_offset;
    if p.eat(&Token::Comma) {
        record_parse_setter(p, rec_start, setters, context, next_boundary_offset)
    } else {
        p.expect(&Token::RightBrace)?;
        match context {
            RecordContext::Plain => {
                Ok(Step::Done(p.spanned_from(rec_start, Expr::Record(setters))))
            }
            RecordContext::Update(base) => Ok(Step::Done(p.spanned_from(
                rec_start,
                Expr::RecordUpdate {
                    base,
                    updates: setters,
                },
            ))),
        }
    }
}

// ── Qualified value (unchanged) ──────────────────────────────────────

/// Parse a possibly-qualified value reference: `foo`, `List.map`, `Maybe.Just`
fn parse_qualified_value(p: &mut Parser) -> ParseResult<(Vec<String>, String)> {
    let first = p.expect_upper_name()?;
    let mut parts: Vec<String> = vec![first.value];

    while matches!(p.peek(), Token::Dot) {
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
                break;
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

// ── Token predicates (unchanged) ─────────────────────────────────────

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

/// In application argument position, a `-` with no whitespace between it
/// and the following atomic token is unary negation consumed as an argument:
/// `f -x` parses as `f (-x)`, while `f - x` remains binary subtraction.
fn is_unary_minus_arg(p: &Parser) -> bool {
    if !matches!(p.peek(), Token::Minus) {
        return false;
    }
    let minus = p.current();
    let next = p.peek_raw_next();
    if !can_start_atomic_expr(&next.value) {
        return false;
    }
    minus.span.end.offset == next.span.start.offset
}
