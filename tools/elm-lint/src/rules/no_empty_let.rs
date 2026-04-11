use elm_ast::expr::Expr;
use elm_ast::node::Spanned;
use elm_ast::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule, Severity};

/// Reports `let in` expressions with no declarations.
pub struct NoEmptyLet;

impl Rule for NoEmptyLet {
    fn name(&self) -> &'static str {
        "NoEmptyLet"
    }

    fn description(&self) -> &'static str {
        "Let expressions with no declarations are useless"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = LetVisitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor
            .0
            .into_iter()
            .map(|span| LintError {
                rule: self.name(),
                    severity: Severity::Warning,
                message: "Empty `let` expression — remove the `let ... in` wrapper".into(),
                span,
                fix: None,
            })
            .collect()
    }
}

struct LetVisitor(Vec<elm_ast::span::Span>);

impl Visit for LetVisitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::LetIn { declarations, .. } = &expr.value {
            if declarations.is_empty() {
                self.0.push(expr.span);
            }
        }
        visit::walk_expr(self, expr);
    }
}
