//! Immutable visitor trait for traversing Elm ASTs.
//!
//! Implement [`Visit`] and override specific `visit_*` methods to inspect nodes.
//! Default implementations recursively descend into child nodes.
//!
//! ```
//! use elm_ast::visit::{Visit, walk_expr};
//! use elm_ast::expr::Expr;
//! use elm_ast::node::Spanned;
//!
//! struct FunctionCounter(usize);
//!
//! impl Visit for FunctionCounter {
//!     fn visit_expr(&mut self, expr: &Spanned<Expr>) {
//!         if matches!(&expr.value, Expr::Lambda { .. }) {
//!             self.0 += 1;
//!         }
//!         walk_expr(self, expr);
//!     }
//! }
//! ```

use crate::comment::Comment;
use crate::declaration::{CustomType, Declaration, InfixDef, TypeAlias, ValueConstructor};
use crate::exposing::{ExposedItem, Exposing};
use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, LetDeclaration, RecordSetter, Signature,
};
use crate::file::ElmModule;
use crate::import::Import;
use crate::literal::Literal;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::pattern::Pattern;
use crate::type_annotation::{RecordField, TypeAnnotation};

/// Immutable visitor trait. Override methods to inspect AST nodes;
/// call the corresponding `walk_*` function to continue descent.
#[allow(unused_variables)]
pub trait Visit {
    fn visit_module(&mut self, module: &ElmModule) {
        walk_module(self, module);
    }

    fn visit_module_header(&mut self, header: &Spanned<ModuleHeader>) {
        walk_module_header(self, header);
    }

    fn visit_import(&mut self, import: &Spanned<Import>) {
        walk_import(self, import);
    }

    fn visit_exposing(&mut self, exposing: &Spanned<Exposing>) {
        walk_exposing(self, exposing);
    }

    fn visit_exposed_item(&mut self, item: &Spanned<ExposedItem>) {}

    fn visit_declaration(&mut self, decl: &Spanned<Declaration>) {
        walk_declaration(self, decl);
    }

    fn visit_function(&mut self, func: &Function) {
        walk_function(self, func);
    }

    fn visit_signature(&mut self, sig: &Spanned<Signature>) {
        walk_signature(self, sig);
    }

    fn visit_function_implementation(&mut self, imp: &Spanned<FunctionImplementation>) {
        walk_function_implementation(self, imp);
    }

    fn visit_type_alias(&mut self, alias: &TypeAlias) {
        walk_type_alias(self, alias);
    }

    fn visit_custom_type(&mut self, ct: &CustomType) {
        walk_custom_type(self, ct);
    }

    fn visit_value_constructor(&mut self, ctor: &Spanned<ValueConstructor>) {
        walk_value_constructor(self, ctor);
    }

    fn visit_infix_def(&mut self, infix: &InfixDef) {}

    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        walk_expr(self, expr);
    }

    fn visit_pattern(&mut self, pattern: &Spanned<Pattern>) {
        walk_pattern(self, pattern);
    }

    fn visit_type_annotation(&mut self, ty: &Spanned<TypeAnnotation>) {
        walk_type_annotation(self, ty);
    }

    fn visit_record_field(&mut self, field: &Spanned<RecordField>) {
        walk_record_field(self, field);
    }

    fn visit_let_declaration(&mut self, decl: &Spanned<LetDeclaration>) {
        walk_let_declaration(self, decl);
    }

    fn visit_case_branch(&mut self, branch: &CaseBranch) {
        walk_case_branch(self, branch);
    }

    fn visit_record_setter(&mut self, setter: &Spanned<RecordSetter>) {
        walk_record_setter(self, setter);
    }

    fn visit_literal(&mut self, lit: &Literal) {}

    fn visit_comment(&mut self, comment: &Spanned<Comment>) {}

    fn visit_ident(&mut self, name: &str) {}
}

// ── Walk functions ───────────────────────────────────────────────────

pub fn walk_module<V: Visit + ?Sized>(v: &mut V, module: &ElmModule) {
    v.visit_module_header(&module.header);
    for import in &module.imports {
        v.visit_import(import);
    }
    for decl in &module.declarations {
        v.visit_declaration(decl);
    }
    for comment in &module.comments {
        v.visit_comment(comment);
    }
}

pub fn walk_module_header<V: Visit + ?Sized>(v: &mut V, header: &Spanned<ModuleHeader>) {
    match &header.value {
        ModuleHeader::Normal { exposing, .. } | ModuleHeader::Port { exposing, .. } => {
            v.visit_exposing(exposing);
        }
        ModuleHeader::Effect { exposing, .. } => {
            v.visit_exposing(exposing);
        }
    }
}

pub fn walk_import<V: Visit + ?Sized>(v: &mut V, import: &Spanned<Import>) {
    if let Some(exposing) = &import.value.exposing {
        v.visit_exposing(exposing);
    }
}

pub fn walk_exposing<V: Visit + ?Sized>(v: &mut V, exposing: &Spanned<Exposing>) {
    if let Exposing::Explicit(items) = &exposing.value {
        for item in items {
            v.visit_exposed_item(item);
        }
    }
}

pub fn walk_declaration<V: Visit + ?Sized>(v: &mut V, decl: &Spanned<Declaration>) {
    match &decl.value {
        Declaration::FunctionDeclaration(func) => v.visit_function(func),
        Declaration::AliasDeclaration(alias) => v.visit_type_alias(alias),
        Declaration::CustomTypeDeclaration(ct) => v.visit_custom_type(ct),
        Declaration::PortDeclaration(sig) => {
            v.visit_ident(&sig.name.value);
            v.visit_type_annotation(&sig.type_annotation);
        }
        Declaration::InfixDeclaration(infix) => v.visit_infix_def(infix),
        Declaration::Destructuring { pattern, body } => {
            v.visit_pattern(pattern);
            v.visit_expr(body);
        }
    }
}

pub fn walk_function<V: Visit + ?Sized>(v: &mut V, func: &Function) {
    if let Some(sig) = &func.signature {
        v.visit_signature(sig);
    }
    v.visit_function_implementation(&func.declaration);
}

pub fn walk_signature<V: Visit + ?Sized>(v: &mut V, sig: &Spanned<Signature>) {
    v.visit_ident(&sig.value.name.value);
    v.visit_type_annotation(&sig.value.type_annotation);
}

pub fn walk_function_implementation<V: Visit + ?Sized>(
    v: &mut V,
    imp: &Spanned<FunctionImplementation>,
) {
    v.visit_ident(&imp.value.name.value);
    for arg in &imp.value.args {
        v.visit_pattern(arg);
    }
    v.visit_expr(&imp.value.body);
}

pub fn walk_type_alias<V: Visit + ?Sized>(v: &mut V, alias: &TypeAlias) {
    v.visit_type_annotation(&alias.type_annotation);
}

pub fn walk_custom_type<V: Visit + ?Sized>(v: &mut V, ct: &CustomType) {
    for ctor in &ct.constructors {
        v.visit_value_constructor(ctor);
    }
}

pub fn walk_value_constructor<V: Visit + ?Sized>(v: &mut V, ctor: &Spanned<ValueConstructor>) {
    for arg in &ctor.value.args {
        v.visit_type_annotation(arg);
    }
}

pub fn walk_expr<V: Visit + ?Sized>(v: &mut V, expr: &Spanned<Expr>) {
    // Visit node-attached comments (expression-level comment preservation).
    for c in &expr.comments {
        v.visit_comment(c);
    }
    match &expr.value {
        Expr::Unit | Expr::GLSLExpression(_) | Expr::RecordAccessFunction(_) => {}

        Expr::Literal(lit) => v.visit_literal(lit),

        Expr::FunctionOrValue { name, .. } => v.visit_ident(name),

        Expr::PrefixOperator(op) => v.visit_ident(op),

        Expr::OperatorApplication { left, right, .. } => {
            v.visit_expr(left);
            v.visit_expr(right);
        }

        Expr::BinOps {
            operands_and_operators,
            final_operand,
        } => {
            for (operand, _op) in operands_and_operators {
                v.visit_expr(operand);
            }
            v.visit_expr(final_operand);
        }

        Expr::Application(args) => {
            for arg in args {
                v.visit_expr(arg);
            }
        }

        Expr::IfElse {
            branches,
            else_branch,
        } => {
            for branch in branches {
                v.visit_expr(&branch.condition);
                v.visit_expr(&branch.then_branch);
            }
            v.visit_expr(else_branch);
        }

        Expr::Negation(inner) => v.visit_expr(inner),

        Expr::Tuple(elems) | Expr::List(elems) => {
            for elem in elems {
                v.visit_expr(elem);
            }
        }

        Expr::Parenthesized(inner) => v.visit_expr(inner),

        Expr::LetIn {
            declarations,
            body,
            trailing_comments: _,
        } => {
            for decl in declarations {
                v.visit_let_declaration(decl);
            }
            v.visit_expr(body);
        }

        Expr::CaseOf {
            expr: subject,
            branches,
        } => {
            v.visit_expr(subject);
            for branch in branches {
                v.visit_case_branch(branch);
            }
        }

        Expr::Lambda { args, body } => {
            for arg in args {
                v.visit_pattern(arg);
            }
            v.visit_expr(body);
        }

        Expr::Record(fields) => {
            for field in fields {
                v.visit_record_setter(field);
            }
        }

        Expr::RecordUpdate { updates, .. } => {
            for field in updates {
                v.visit_record_setter(field);
            }
        }

        Expr::RecordAccess { record, .. } => {
            v.visit_expr(record);
        }
    }
}

pub fn walk_pattern<V: Visit + ?Sized>(v: &mut V, pattern: &Spanned<Pattern>) {
    for c in &pattern.comments {
        v.visit_comment(c);
    }
    match &pattern.value {
        Pattern::Anything | Pattern::Unit | Pattern::Hex(_) => {}

        Pattern::Var(name) => v.visit_ident(name),

        Pattern::Literal(lit) => v.visit_literal(lit),

        Pattern::Tuple(elems) | Pattern::List(elems) => {
            for elem in elems {
                v.visit_pattern(elem);
            }
        }

        Pattern::Constructor { args, .. } => {
            for arg in args {
                v.visit_pattern(arg);
            }
        }

        Pattern::Record(fields) => {
            for field in fields {
                v.visit_ident(&field.value);
            }
        }

        Pattern::Cons { head, tail } => {
            v.visit_pattern(head);
            v.visit_pattern(tail);
        }

        Pattern::As {
            pattern: inner,
            name,
        } => {
            v.visit_pattern(inner);
            v.visit_ident(&name.value);
        }

        Pattern::Parenthesized(inner) => v.visit_pattern(inner),
    }
}

pub fn walk_type_annotation<V: Visit + ?Sized>(v: &mut V, ty: &Spanned<TypeAnnotation>) {
    match &ty.value {
        TypeAnnotation::GenericType(_) | TypeAnnotation::Unit => {}

        TypeAnnotation::Typed { args, .. } => {
            for arg in args {
                v.visit_type_annotation(arg);
            }
        }

        TypeAnnotation::Tupled(elems) => {
            for elem in elems {
                v.visit_type_annotation(elem);
            }
        }

        TypeAnnotation::Record(fields) => {
            for field in fields {
                v.visit_record_field(field);
            }
        }

        TypeAnnotation::GenericRecord { fields, .. } => {
            for field in fields {
                v.visit_record_field(field);
            }
        }

        TypeAnnotation::FunctionType { from, to } => {
            v.visit_type_annotation(from);
            v.visit_type_annotation(to);
        }
    }
}

pub fn walk_record_field<V: Visit + ?Sized>(v: &mut V, field: &Spanned<RecordField>) {
    v.visit_type_annotation(&field.value.type_annotation);
}

pub fn walk_let_declaration<V: Visit + ?Sized>(v: &mut V, decl: &Spanned<LetDeclaration>) {
    for c in &decl.comments {
        v.visit_comment(c);
    }
    match &decl.value {
        LetDeclaration::Function(func) => v.visit_function(func),
        LetDeclaration::Destructuring { pattern, body } => {
            v.visit_pattern(pattern);
            v.visit_expr(body);
        }
    }
}

pub fn walk_case_branch<V: Visit + ?Sized>(v: &mut V, branch: &CaseBranch) {
    v.visit_pattern(&branch.pattern);
    v.visit_expr(&branch.body);
}

pub fn walk_record_setter<V: Visit + ?Sized>(v: &mut V, setter: &Spanned<RecordSetter>) {
    v.visit_expr(&setter.value.value);
}
