use elm_ast::expr::Expr;
use elm_ast::node::Spanned;
use elm_ast::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports `if x then True else False` (should be just `x`)
/// and `if x then False else True` (should be `not x`).
pub struct NoIfTrueFalse;

impl Rule for NoIfTrueFalse {
    fn name(&self) -> &'static str {
        "NoIfTrueFalse"
    }

    fn description(&self) -> &'static str {
        "Simplify `if x then True else False` to `x`"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = Visitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor.0
    }
}

struct Visitor(Vec<LintError>);

impl Visit for Visitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::IfElse {
            branches,
            else_branch,
        } = &expr.value
        {
            if branches.len() == 1 {
                let then_val = is_bool_literal(&branches[0].1.value);
                let else_val = is_bool_literal(&else_branch.value);

                match (then_val, else_val) {
                    (Some(true), Some(false)) => {
                        self.0.push(LintError {
                            rule: "NoIfTrueFalse",
                            message: "`if x then True else False` is equivalent to `x`".into(),
                            span: expr.span,
                            fix: None,
                        });
                    }
                    (Some(false), Some(true)) => {
                        self.0.push(LintError {
                            rule: "NoIfTrueFalse",
                            message: "`if x then False else True` is equivalent to `not x`".into(),
                            span: expr.span,
                            fix: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        visit::walk_expr(self, expr);
    }
}

fn is_bool_literal(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::FunctionOrValue { module_name, name } if module_name.is_empty() => {
            match name.as_str() {
                "True" => Some(true),
                "False" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}
