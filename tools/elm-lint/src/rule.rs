use elm_ast_rs::file::ElmModule;
use elm_ast_rs::span::Span;

/// A lint error reported by a rule.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LintError {
    /// The rule that produced this error.
    pub rule: &'static str,
    /// Human-readable error message.
    pub message: String,
    /// Source location of the error.
    pub span: Span,
    /// Optional fix suggestion.
    pub fix: Option<String>,
}

/// The context passed to rules, containing the module and project-level info.
#[allow(dead_code)]
pub struct LintContext<'a> {
    pub module: &'a ElmModule,
    pub source: &'a str,
    pub file_path: &'a str,
    /// All module names in the project (for cross-module checks).
    pub project_modules: &'a [String],
}

/// A lint rule.
///
/// Each rule inspects a parsed module and returns zero or more errors.
pub trait Rule: Send + Sync {
    /// The rule's unique identifier, e.g. "NoUnusedImports".
    fn name(&self) -> &'static str;

    /// A short description of what this rule checks.
    fn description(&self) -> &'static str;

    /// Run the rule against a module and return any findings.
    fn check(&self, ctx: &LintContext) -> Vec<LintError>;
}
