use elm_ast_rs::expr::Expr;
use elm_ast_rs::node::Spanned;
use elm_ast_rs::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports `{ record | }` with no actual updates, which is just `record`.
pub struct NoEmptyRecordUpdate;

impl Rule for NoEmptyRecordUpdate {
    fn name(&self) -> &'static str {
        "NoEmptyRecordUpdate"
    }

    fn description(&self) -> &'static str {
        "Record update with no fields is equivalent to the record itself"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = Visitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor
            .0
            .into_iter()
            .map(|span| LintError {
                rule: self.name(),
                message: "Record update with no fields — just use the record directly".into(),
                span,
                fix: None,
            })
            .collect()
    }
}

struct Visitor(Vec<elm_ast_rs::span::Span>);

impl Visit for Visitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::RecordUpdate { updates, .. } = &expr.value {
            if updates.is_empty() {
                self.0.push(expr.span);
            }
        }
        visit::walk_expr(self, expr);
    }
}
