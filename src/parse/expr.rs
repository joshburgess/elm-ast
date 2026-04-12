use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, LetDeclaration, RecordSetter, Signature,
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

struct IfBranch {
    start: Position,
    condition: Spanned<Expr>,
    then_branch: Spanned<Expr>,
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
        p.skip_whitespace();

        let (op, prec, assoc) = match extract_operator(p) {
            Some(info) => info,
            None => break,
        };

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
            Step::Done(expr) => {
                left = expr;
            }
            Step::NeedExpr(cont) => {
                return Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
                    let step = cont(p, sub_expr)?;
                    binary_after_operand(p, step, pending)
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
        p.skip_whitespace();
        if !can_start_atomic_expr(p.peek()) {
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
        let step = parse_atomic_expr_cps(p)?;
        match step {
            Step::Done(arg) => args.push(arg),
            Step::NeedExpr(cont) => {
                return Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
                    let step = cont(p, sub_expr)?;
                    app_after_arg(p, step, start, first_line, args, ctx_col)
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
fn app_after_arg(
    p: &mut Parser,
    step: Step,
    start: Position,
    first_line: u32,
    args: Vec<Spanned<Expr>>,
    ctx_col: Option<u32>,
) -> ParseResult<Step> {
    match step {
        Step::NeedExpr(cont) => Ok(Step::NeedExpr(Box::new(move |p, sub_expr| {
            let step = cont(p, sub_expr)?;
            app_after_arg(p, step, start, first_line, args, ctx_col)
        }))),
        Step::Done(arg) => {
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
    // Need condition
    Ok(Step::NeedExpr(Box::new(move |p, condition| {
        if_after_condition(p, start, Vec::new(), condition)
    })))
}

fn if_after_condition(
    p: &mut Parser,
    start: Position,
    chain: Vec<IfBranch>,
    condition: Spanned<Expr>,
) -> ParseResult<Step> {
    p.expect(&Token::Then)?;
    // Need then-branch
    Ok(Step::NeedExpr(Box::new(move |p, then_branch| {
        if_after_then(p, start, chain, condition, then_branch)
    })))
}

fn if_after_then(
    p: &mut Parser,
    start: Position,
    mut chain: Vec<IfBranch>,
    condition: Spanned<Expr>,
    then_branch: Spanned<Expr>,
) -> ParseResult<Step> {
    chain.push(IfBranch {
        start,
        condition,
        then_branch,
    });
    p.expect(&Token::Else)?;
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
            // Fold from right to left to build nested IfElse structure.
            let mut result = else_branch;
            for branch in chain.into_iter().rev() {
                result = p.spanned_from(
                    branch.start,
                    Expr::IfElse {
                        branches: vec![(branch.condition, branch.then_branch)],
                        else_branch: Box::new(result),
                    },
                );
            }
            Ok(Step::Done(result))
        })))
    }
}

// ── Case-of (CPS) ───────────────────────────────────────────────────

fn parse_case_expr_cps(p: &mut Parser) -> ParseResult<Step> {
    let start = p.current_pos();
    p.expect(&Token::Case)?;
    // Need subject
    Ok(Step::NeedExpr(Box::new(move |p, subject| {
        case_after_subject(p, start, subject)
    })))
}

fn case_after_subject(
    p: &mut Parser,
    start: Position,
    subject: Spanned<Expr>,
) -> ParseResult<Step> {
    p.expect(&Token::Of)?;
    p.skip_whitespace();
    let branch_col = p.current_column();

    case_next_branch(p, start, subject, Vec::new(), branch_col)
}

fn case_next_branch(
    p: &mut Parser,
    start: Position,
    subject: Spanned<Expr>,
    branches: Vec<CaseBranch>,
    branch_col: u32,
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

    // Attach comments to the branch pattern.
    let branch_comments = p.take_pending_comments();
    let mut pat = parse_pattern(p)?;
    if !branch_comments.is_empty() {
        pat.comments = branch_comments;
    }
    p.expect(&Token::Arrow)?;

    // Clear the application context column so that the branch body uses
    // normal column checking (prevents over-consuming the next branch pattern).
    p.app_context_col = None;

    // Need branch body
    Ok(Step::NeedExpr(Box::new(move |p, body| {
        let mut branches = branches;
        branches.push(CaseBranch { pattern: pat, body });
        case_next_branch(p, start, subject, branches, branch_col)
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
    p.expect(&Token::Let)?;

    let let_col = start.column;
    p.skip_whitespace();
    let decl_col = p.current_column();

    let_next_decl(p, start, let_col, decl_col, Vec::new())
}

fn let_next_decl(
    p: &mut Parser,
    start: Position,
    let_col: u32,
    decl_col: u32,
    declarations: Vec<Spanned<LetDeclaration>>,
) -> ParseResult<Step> {
    p.skip_whitespace();

    // Check if we're done with declarations.
    if p.is_eof()
        || matches!(p.peek(), Token::In)
        || (!p.in_paren_context() && p.current_column() < decl_col)
        || (!p.in_paren_context() && p.current_column() < let_col + 1)
    {
        p.expect(&Token::In)?;
        // Clear app_context_col for the `in` body.
        p.app_context_col = None;
        // Need body
        return Ok(Step::NeedExpr(Box::new(move |p, body| {
            Ok(Step::Done(p.spanned_from(
                start,
                Expr::LetIn {
                    declarations,
                    body: Box::new(body),
                },
            )))
        })));
    }

    // Attach comments before this let declaration.
    let let_decl_comments = p.take_pending_comments();
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

                // Need function body
                Ok(Step::NeedExpr(Box::new(move |p, body| {
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
                    let_next_decl(p, start, let_col, decl_col, declarations)
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

                // Need function body
                Ok(Step::NeedExpr(Box::new(move |p, body| {
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
                    let_next_decl(p, start, let_col, decl_col, declarations)
                })))
            }
        }
        _ => {
            // Destructuring pattern.
            let pattern = parse_pattern(p)?;
            p.expect(&Token::Equals)?;

            // Clear app_context_col for the destructuring body.
            p.app_context_col = None;

            // Need destructuring body
            Ok(Step::NeedExpr(Box::new(move |p, body| {
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
                let_next_decl(p, start, let_col, decl_col, declarations)
            })))
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

    // Need body
    Ok(Step::NeedExpr(Box::new(move |p, body| {
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
            p.advance();
            p.skip_whitespace();
            if matches!(p.peek(), Token::RightParen) {
                p.advance();
                return Ok(Step::Done(
                    p.spanned_from(start, Expr::PrefixOperator("-".into())),
                ));
            }
            // Negation inside parens: `(-expr)` or `(-expr, ...)`
            return Ok(Step::NeedExpr(Box::new(move |p, operand| {
                let neg = Expr::Negation(Box::new(operand));
                let neg_spanned = p.spanned_from(start, neg);
                paren_after_first(p, start, neg_spanned)
            })));
        }
        _ => {}
    }

    // Regular parenthesized expression or tuple.
    Ok(Step::NeedExpr(Box::new(move |p, first| {
        paren_after_first(p, start, first)
    })))
}

fn paren_after_first(p: &mut Parser, start: Position, first: Spanned<Expr>) -> ParseResult<Step> {
    p.skip_whitespace();
    match p.peek() {
        Token::Comma => {
            p.advance(); // consume comma
            let elements = vec![first];
            // Need next tuple element
            Ok(Step::NeedExpr(Box::new(move |p, next| {
                tuple_after_element(p, start, elements, next)
            })))
        }
        Token::RightParen => {
            p.advance();
            Ok(Step::Done(
                p.spanned_from(start, Expr::Parenthesized(Box::new(first))),
            ))
        }
        _ => Err(p.error("expected `,` or `)` in expression")),
    }
}

fn tuple_after_element(
    p: &mut Parser,
    start: Position,
    mut elements: Vec<Spanned<Expr>>,
    elem: Spanned<Expr>,
) -> ParseResult<Step> {
    elements.push(elem);
    p.skip_whitespace();
    if p.eat(&Token::Comma) {
        // More elements
        Ok(Step::NeedExpr(Box::new(move |p, next| {
            tuple_after_element(p, start, elements, next)
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
    p.skip_whitespace();

    if matches!(p.peek(), Token::RightBracket) {
        p.advance();
        return Ok(Step::Done(p.spanned_from(start, Expr::List(Vec::new()))));
    }

    // Set the application context column to the bracket's column so that
    // function args at any column past the bracket are collected.
    p.app_context_col = Some(bracket_col);

    // Need first element
    Ok(Step::NeedExpr(Box::new(move |p, first| {
        list_after_element(p, start, bracket_col, Vec::new(), first)
    })))
}

fn list_after_element(
    p: &mut Parser,
    start: Position,
    bracket_col: u32,
    mut elements: Vec<Spanned<Expr>>,
    elem: Spanned<Expr>,
) -> ParseResult<Step> {
    elements.push(elem);
    if p.eat(&Token::Comma) {
        // Re-set the context column for the next element.
        p.app_context_col = Some(bracket_col);
        Ok(Step::NeedExpr(Box::new(move |p, next| {
            list_after_element(p, start, bracket_col, elements, next)
        })))
    } else {
        p.expect(&Token::RightBracket)?;
        Ok(Step::Done(p.spanned_from(start, Expr::List(elements))))
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
                return record_parse_setter(p, start, Vec::new(), RecordContext::Update(base_name));
            }
            // Not a record update — backtrack.
            p.pos = save_pos;
        } else {
            p.pos = save_pos;
        }
    }

    // Regular record expression.
    record_parse_setter(p, start, Vec::new(), RecordContext::Plain)
}

fn record_parse_setter(
    p: &mut Parser,
    rec_start: Position,
    setters: Vec<Spanned<RecordSetter>>,
    context: RecordContext,
) -> ParseResult<Step> {
    let setter_start = p.current_pos();
    let field = p.expect_lower_name()?;
    p.expect(&Token::Equals)?;

    // Set the application context column to the brace's column so that
    // function args at any column past the brace are collected.
    p.app_context_col = Some(rec_start.column);

    // Need field value
    Ok(Step::NeedExpr(Box::new(move |p, value| {
        record_after_value(p, rec_start, setters, setter_start, field, value, context)
    })))
}

fn record_after_value(
    p: &mut Parser,
    rec_start: Position,
    mut setters: Vec<Spanned<RecordSetter>>,
    setter_start: Position,
    field: Spanned<String>,
    value: Spanned<Expr>,
    context: RecordContext,
) -> ParseResult<Step> {
    let setter = RecordSetter { field, value };
    setters.push(p.spanned_from(setter_start, setter));

    if p.eat(&Token::Comma) {
        record_parse_setter(p, rec_start, setters, context)
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
