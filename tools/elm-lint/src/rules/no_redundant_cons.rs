use elm_ast_rs::expr::Expr;
use elm_ast_rs::node::Spanned;
use elm_ast_rs::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports `x :: []` which should be `[ x ]`.
pub struct NoRedundantCons;

impl Rule for NoRedundantCons {
    fn name(&self) -> &'static str {
        "NoRedundantCons"
    }

    fn description(&self) -> &'static str {
        "`x :: []` should be written as `[ x ]`"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = Visitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor
            .0
            .into_iter()
            .map(|span| LintError {
                rule: self.name(),
                message: "`x :: []` can be simplified to `[ x ]`".into(),
                span,
                fix: None,
            })
            .collect()
    }
}

struct Visitor(Vec<elm_ast_rs::span::Span>);

impl Visit for Visitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::OperatorApplication {
            operator, right, ..
        } = &expr.value
        {
            if operator == "::" {
                if let Expr::List(elems) = &right.value {
                    if elems.is_empty() {
                        self.0.push(expr.span);
                    }
                }
            }
        }
        visit::walk_expr(self, expr);
    }
}
