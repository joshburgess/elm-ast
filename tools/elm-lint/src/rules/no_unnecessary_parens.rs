use elm_ast::expr::Expr;
use elm_ast::node::Spanned;
use elm_ast::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports unnecessary parentheses around simple expressions.
pub struct NoUnnecessaryParens;

impl Rule for NoUnnecessaryParens {
    fn name(&self) -> &'static str {
        "NoUnnecessaryParens"
    }

    fn description(&self) -> &'static str {
        "Reports parentheses around expressions that don't need them"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = ParenVisitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor
            .0
            .into_iter()
            .map(|span| LintError {
                rule: self.name(),
                message: "Unnecessary parentheses".into(),
                span,
                fix: None,
            })
            .collect()
    }
}

struct ParenVisitor(Vec<elm_ast::span::Span>);

impl Visit for ParenVisitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::Parenthesized(inner) = &expr.value {
            // Parens around simple atoms are unnecessary.
            let is_simple = matches!(
                &inner.value,
                Expr::Unit
                    | Expr::Literal(_)
                    | Expr::FunctionOrValue { .. }
                    | Expr::RecordAccessFunction(_)
            );
            if is_simple {
                self.0.push(expr.span);
            }
        }
        visit::walk_expr(self, expr);
    }
}
