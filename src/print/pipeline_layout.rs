//! Operator-chain flatteners used by the pretty-print layout for pipelines
//! (`|>` / `|.` / `|=`), right-associative cons/append chains (`::` / `++`),
//! and function composition (`>>` / `<<`).
//!
//! elm-format lays these out vertically (one operator per line) when any
//! operand is multi-line. Flattening lets us walk the chain uniformly
//! instead of relying on the nested AST shape, which stair-steps under
//! right-associative operators.

use crate::expr::Expr;
use crate::node::Spanned;

/// Flatten a mixed pipe chain (`|>`, `|.`, `|=`) into a list of
/// `(operator, operand)` pairs plus the first operand. Returns `None` if
/// `expr` is not a pipe-chain.
pub(super) fn flatten_mixed_pipe_chain(
    expr: &Expr,
) -> Option<(&Spanned<Expr>, Vec<(&str, &Spanned<Expr>)>)> {
    fn is_pipe(op: &str) -> bool {
        matches!(op, "|>" | "|." | "|=")
    }
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if is_pipe(operator) => {
            let (head, mut tail) =
                flatten_mixed_pipe_chain(&left.value).unwrap_or((left.as_ref(), Vec::new()));
            tail.push((operator.as_str(), right.as_ref()));
            Some((head, tail))
        }
        _ => None,
    }
}

/// Flatten a left-associative chain, with an operator predicate that decides
/// which operators continue the chain. Returns the initial operand and a list
/// of `(operator, operand)` pairs.
pub(super) fn flatten_left_assoc_pred<'a>(
    expr: &'a Expr,
    pred: &impl Fn(&str) -> bool,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if pred(operator) => {
            let (head, mut tail) =
                flatten_left_assoc_pred(&left.value, pred).unwrap_or((left.as_ref(), Vec::new()));
            tail.push((operator.as_str(), right.as_ref()));
            Some((head, tail))
        }
        _ => None,
    }
}

/// Flatten a right-associative operator chain into a list of expressions.
/// `a :: b :: c` (parsed as `a :: (b :: c)`) becomes `[a, b, c]`.
pub(super) fn flatten_right_assoc_chain<'a>(
    expr: &'a Expr,
    target_op: &str,
) -> Option<Vec<&'a Spanned<Expr>>> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if operator == target_op => {
            let mut chain = vec![left.as_ref()];
            match flatten_right_assoc_chain(&right.value, target_op) {
                Some(mut rest) => chain.append(&mut rest),
                None => chain.push(right.as_ref()),
            }
            Some(chain)
        }
        _ => None,
    }
}

/// Flatten a right-associative chain where operators may mix between `::` and
/// `++` (same precedence 5, right-associative in Elm). elm-format unifies
/// such chains into one vertical layout.
pub(super) fn flatten_mixed_cons_append_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    fn is_cons_or_append(op: &str) -> bool {
        matches!(op, "::" | "++")
    }
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if is_cons_or_append(operator) => {
            let mut rest: Vec<(&'a str, &'a Spanned<Expr>)> = Vec::new();
            let (head, tail_rest) = match flatten_mixed_cons_append_chain(&right.value) {
                Some((head, rest_r)) => {
                    rest.push((operator.as_str(), head));
                    for (op, e) in rest_r {
                        rest.push((op, e));
                    }
                    (left.as_ref(), rest)
                }
                None => {
                    rest.push((operator.as_str(), right.as_ref()));
                    (left.as_ref(), rest)
                }
            };
            Some((head, tail_rest))
        }
        _ => None,
    }
}

/// Flatten a chain where operators from a given set may mix at different
/// precedences, descending through both sides. elm-format lays out such
/// mixed chains as a single vertical sequence with each operator aligned at
/// the same indent column, regardless of the parse-tree grouping.
fn flatten_mixed_bidir_chain<'a>(
    expr: &'a Expr,
    pred: &impl Fn(&str) -> bool,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if pred(operator) => {
            let (lhead, mut rest) = match flatten_mixed_bidir_chain(&left.value, pred) {
                Some((h, r)) => (h, r),
                None => (left.as_ref(), Vec::new()),
            };
            match flatten_mixed_bidir_chain(&right.value, pred) {
                Some((rhead, rrest)) => {
                    rest.push((operator.as_str(), rhead));
                    for (op, e) in rrest {
                        rest.push((op, e));
                    }
                }
                None => {
                    rest.push((operator.as_str(), right.as_ref()));
                }
            };
            Some((lhead, rest))
        }
        _ => None,
    }
}

/// Flatten a chain of mixed `&&` / `||` operators. `&&` and `||` sit at
/// different precedences (3 and 2), so grouping can favor either direction,
/// but elm-format visually aligns all logical operators at the same column.
pub(super) fn flatten_mixed_logical_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    flatten_mixed_bidir_chain(expr, &|op| matches!(op, "&&" | "||"))
}

/// Flatten a chain of mixed arithmetic operators (`+`, `-`, `*`, `/`, `//`).
/// These sit at precedences 6 and 7 and are left-associative, so grouping
/// can nest either way depending on how the source was written. elm-format
/// collapses all such mixed chains into a single vertical layout with every
/// operator at the same indent column.
pub(super) fn flatten_mixed_arithmetic_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    flatten_mixed_bidir_chain(expr, &|op| matches!(op, "+" | "-" | "*" | "/" | "//"))
}

/// Try to further flatten `expr` as any operator chain (cons/append,
/// logical, arithmetic, compose). Used by the pipe-chain printer: when a
/// pipe chain goes vertical and its head is itself a binop chain,
/// elm-format lays out every operator at the same indent column. Returns
/// `None` if `expr` isn't a chainable operator application.
pub(super) fn flatten_as_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Spanned<Expr>, Vec<(&'a str, &'a Spanned<Expr>)>)> {
    match expr {
        Expr::OperatorApplication { operator, .. } => {
            let op = operator.as_str();
            if matches!(op, "::" | "++") {
                flatten_mixed_cons_append_chain(expr)
            } else if matches!(op, "&&" | "||") {
                flatten_mixed_logical_chain(expr)
            } else if matches!(op, "+" | "-" | "*" | "/" | "//") {
                flatten_mixed_arithmetic_chain(expr)
            } else if matches!(op, ">>" | "<<") {
                flatten_right_assoc_chain(expr, op).map(|chain| {
                    let first = chain[0];
                    let rest: Vec<(&str, &Spanned<Expr>)> =
                        chain[1..].iter().map(|e| (op, *e)).collect();
                    (first, rest)
                })
            } else {
                None
            }
        }
        _ => None,
    }
}
