use elm_ast_rs::expr::Expr;
use elm_ast_rs::node::Spanned;
use elm_ast_rs::visit::{self, Visit};

use crate::rule::{LintContext, LintError, Rule};

/// Reports `always identity` which is just `identity`.
/// Also reports `identity >> f` or `f >> identity` which is just `f`,
/// and `always x` in places where a simple value would suffice.
pub struct NoAlwaysIdentity;

impl Rule for NoAlwaysIdentity {
    fn name(&self) -> &'static str {
        "NoAlwaysIdentity"
    }

    fn description(&self) -> &'static str {
        "Simplify `always identity` and identity composition"
    }

    fn check(&self, ctx: &LintContext) -> Vec<LintError> {
        let mut visitor = AlwaysVisitor(Vec::new());
        visitor.visit_module(ctx.module);
        visitor.0
    }
}

struct AlwaysVisitor(Vec<LintError>);

impl Visit for AlwaysVisitor {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        // `always identity` → `identity`
        if let Expr::Application(args) = &expr.value {
            if args.len() == 2 && is_name(&args[0].value, "always") && is_name(&args[1].value, "identity") {
                self.0.push(LintError {
                    rule: "NoAlwaysIdentity",
                    message: "`always identity` is equivalent to `identity`".into(),
                    span: expr.span,
                    fix: None,
                });
            }
        }

        // `identity >> f` or `f >> identity` → `f`
        // `identity << f` or `f << identity` → `f`
        if let Expr::OperatorApplication { operator, left, right, .. } = &expr.value {
            if operator == ">>" || operator == "<<" {
                if is_name(&left.value, "identity") || is_name(&right.value, "identity") {
                    self.0.push(LintError {
                        rule: "NoAlwaysIdentity",
                        message: format!("Composing with `identity` using `{operator}` has no effect"),
                        span: expr.span,
                        fix: None,
                    });
                }
            }
        }

        visit::walk_expr(self, expr);
    }
}

fn is_name(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::FunctionOrValue { module_name, name: n } if module_name.is_empty() && n == name)
}
