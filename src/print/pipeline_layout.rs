//! Operator-chain flatteners used by the pretty-print layout for pipelines
//! (`|>` / `|.` / `|=`), right-associative cons/append chains (`::` / `++`),
//! and function composition (`>>` / `<<`).
//!
//! elm-format lays these out vertically (one operator per line) when any
//! operand is multi-line. Flattening lets us walk the chain uniformly
//! instead of relying on the nested AST shape, which stair-steps under
//! right-associative operators.

use crate::expr::Expr;

/// Flatten a mixed pipe chain (`|>`, `|.`, `|=`) into a list of
/// `(operator, operand)` pairs plus the first operand. Returns `None` if
/// `expr` is not a pipe-chain.
pub(super) fn flatten_mixed_pipe_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
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
                flatten_mixed_pipe_chain(&left.value).unwrap_or((&left.value, Vec::new()));
            tail.push((operator.as_str(), &right.value));
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
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if pred(operator) => {
            let (head, mut tail) =
                flatten_left_assoc_pred(&left.value, pred).unwrap_or((&left.value, Vec::new()));
            tail.push((operator.as_str(), &right.value));
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
) -> Option<Vec<&'a Expr>> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if operator == target_op => {
            let mut chain = vec![&left.value];
            match flatten_right_assoc_chain(&right.value, target_op) {
                Some(mut rest) => chain.append(&mut rest),
                None => chain.push(&right.value),
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
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
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
            let mut rest: Vec<(&'a str, &'a Expr)> = Vec::new();
            let (head, tail_rest) = match flatten_mixed_cons_append_chain(&right.value) {
                Some((head, rest_r)) => {
                    rest.push((operator.as_str(), head));
                    for (op, e) in rest_r {
                        rest.push((op, e));
                    }
                    (&left.value, rest)
                }
                None => {
                    rest.push((operator.as_str(), &right.value));
                    (&left.value, rest)
                }
            };
            Some((head, tail_rest))
        }
        _ => None,
    }
}
