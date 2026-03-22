use elm_ast_rs::expr::Expr;
use elm_ast_rs::node::Spanned;
use elm_ast_rs::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports double negation like `not (not x)` or `-(-(x))`.
pub struct NoNestedNegation;

impl Rule for NoNestedNegation {
    fn name(&self) -> &'static str {
        "NoNestedNegation"
    }

    fn description(&self) -> &'static str {
        "Double negation is redundant"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = NegVisitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor
            .0
            .into_iter()
            .map(|span| LintError {
                rule: self.name(),
                message: "Double negation — simplify by removing both".into(),
                span,
                fix: None,
            })
            .collect()
    }
}

struct NegVisitor(Vec<elm_ast_rs::span::Span>);

impl Visit for NegVisitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if let Expr::Negation(inner) = &expr.value {
            if matches!(&inner.value, Expr::Negation(_)) {
                self.0.push(expr.span);
            }
        }
        // Also check `not (not x)` pattern.
        if let Expr::Application(args) = &expr.value {
            if args.len() == 2 {
                if let Expr::FunctionOrValue {
                    module_name,
                    name,
                } = &args[0].value
                {
                    if module_name.is_empty() && name == "not" {
                        if let Expr::Parenthesized(inner) = &args[1].value {
                            if let Expr::Application(inner_args) = &inner.value {
                                if inner_args.len() == 2 {
                                    if let Expr::FunctionOrValue {
                                        module_name: m2,
                                        name: n2,
                                    } = &inner_args[0].value
                                    {
                                        if m2.is_empty() && n2 == "not" {
                                            self.0.push(expr.span);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        visit::walk_expr(self, expr);
    }
}
