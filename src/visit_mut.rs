//! Mutable visitor trait for in-place AST transformation.
//!
//! Like [`Visit`](crate::visit::Visit) but takes `&mut` references,
//! allowing in-place modification of AST nodes.

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

/// Mutable visitor trait. Override methods to modify AST nodes in place;
/// call the corresponding `walk_*_mut` function to continue descent.
#[allow(unused_variables)]
pub trait VisitMut {
    fn visit_module_mut(&mut self, module: &mut ElmModule) {
        walk_module_mut(self, module);
    }

    fn visit_module_header_mut(&mut self, header: &mut Spanned<ModuleHeader>) {
        walk_module_header_mut(self, header);
    }

    fn visit_import_mut(&mut self, import: &mut Spanned<Import>) {
        walk_import_mut(self, import);
    }

    fn visit_exposing_mut(&mut self, exposing: &mut Spanned<Exposing>) {
        walk_exposing_mut(self, exposing);
    }

    fn visit_exposed_item_mut(&mut self, item: &mut Spanned<ExposedItem>) {}

    fn visit_declaration_mut(&mut self, decl: &mut Spanned<Declaration>) {
        walk_declaration_mut(self, decl);
    }

    fn visit_function_mut(&mut self, func: &mut Function) {
        walk_function_mut(self, func);
    }

    fn visit_signature_mut(&mut self, sig: &mut Spanned<Signature>) {
        walk_signature_mut(self, sig);
    }

    fn visit_function_implementation_mut(&mut self, imp: &mut Spanned<FunctionImplementation>) {
        walk_function_implementation_mut(self, imp);
    }

    fn visit_type_alias_mut(&mut self, alias: &mut TypeAlias) {
        walk_type_alias_mut(self, alias);
    }

    fn visit_custom_type_mut(&mut self, ct: &mut CustomType) {
        walk_custom_type_mut(self, ct);
    }

    fn visit_value_constructor_mut(&mut self, ctor: &mut Spanned<ValueConstructor>) {
        walk_value_constructor_mut(self, ctor);
    }

    fn visit_infix_def_mut(&mut self, infix: &mut InfixDef) {}

    fn visit_expr_mut(&mut self, expr: &mut Spanned<Expr>) {
        walk_expr_mut(self, expr);
    }

    fn visit_pattern_mut(&mut self, pattern: &mut Spanned<Pattern>) {
        walk_pattern_mut(self, pattern);
    }

    fn visit_type_annotation_mut(&mut self, ty: &mut Spanned<TypeAnnotation>) {
        walk_type_annotation_mut(self, ty);
    }

    fn visit_record_field_mut(&mut self, field: &mut Spanned<RecordField>) {
        walk_record_field_mut(self, field);
    }

    fn visit_let_declaration_mut(&mut self, decl: &mut Spanned<LetDeclaration>) {
        walk_let_declaration_mut(self, decl);
    }

    fn visit_case_branch_mut(&mut self, branch: &mut CaseBranch) {
        walk_case_branch_mut(self, branch);
    }

    fn visit_record_setter_mut(&mut self, setter: &mut Spanned<RecordSetter>) {
        walk_record_setter_mut(self, setter);
    }

    fn visit_literal_mut(&mut self, lit: &mut Literal) {}

    fn visit_comment_mut(&mut self, comment: &mut Spanned<Comment>) {}

    fn visit_ident_mut(&mut self, name: &mut String) {}
}

// ── Walk functions ───────────────────────────────────────────────────

pub fn walk_module_mut<V: VisitMut + ?Sized>(v: &mut V, module: &mut ElmModule) {
    v.visit_module_header_mut(&mut module.header);
    for import in &mut module.imports {
        v.visit_import_mut(import);
    }
    for decl in &mut module.declarations {
        v.visit_declaration_mut(decl);
    }
    for comment in &mut module.comments {
        v.visit_comment_mut(comment);
    }
}

pub fn walk_module_header_mut<V: VisitMut + ?Sized>(v: &mut V, header: &mut Spanned<ModuleHeader>) {
    match &mut header.value {
        ModuleHeader::Normal { exposing, .. } | ModuleHeader::Port { exposing, .. } => {
            v.visit_exposing_mut(exposing);
        }
        ModuleHeader::Effect { exposing, .. } => {
            v.visit_exposing_mut(exposing);
        }
    }
}

pub fn walk_import_mut<V: VisitMut + ?Sized>(v: &mut V, import: &mut Spanned<Import>) {
    if let Some(exposing) = &mut import.value.exposing {
        v.visit_exposing_mut(exposing);
    }
}

pub fn walk_exposing_mut<V: VisitMut + ?Sized>(v: &mut V, exposing: &mut Spanned<Exposing>) {
    if let Exposing::Explicit(items) = &mut exposing.value {
        for item in items {
            v.visit_exposed_item_mut(item);
        }
    }
}

pub fn walk_declaration_mut<V: VisitMut + ?Sized>(v: &mut V, decl: &mut Spanned<Declaration>) {
    match &mut decl.value {
        Declaration::FunctionDeclaration(func) => v.visit_function_mut(func),
        Declaration::AliasDeclaration(alias) => v.visit_type_alias_mut(alias),
        Declaration::CustomTypeDeclaration(ct) => v.visit_custom_type_mut(ct),
        Declaration::PortDeclaration(sig) => {
            v.visit_ident_mut(&mut sig.name.value);
            v.visit_type_annotation_mut(&mut sig.type_annotation);
        }
        Declaration::InfixDeclaration(infix) => v.visit_infix_def_mut(infix),
        Declaration::Destructuring { pattern, body } => {
            v.visit_pattern_mut(pattern);
            v.visit_expr_mut(body);
        }
    }
}

pub fn walk_function_mut<V: VisitMut + ?Sized>(v: &mut V, func: &mut Function) {
    if let Some(sig) = &mut func.signature {
        v.visit_signature_mut(sig);
    }
    v.visit_function_implementation_mut(&mut func.declaration);
}

pub fn walk_signature_mut<V: VisitMut + ?Sized>(v: &mut V, sig: &mut Spanned<Signature>) {
    v.visit_ident_mut(&mut sig.value.name.value);
    v.visit_type_annotation_mut(&mut sig.value.type_annotation);
}

pub fn walk_function_implementation_mut<V: VisitMut + ?Sized>(
    v: &mut V,
    imp: &mut Spanned<FunctionImplementation>,
) {
    v.visit_ident_mut(&mut imp.value.name.value);
    for arg in &mut imp.value.args {
        v.visit_pattern_mut(arg);
    }
    v.visit_expr_mut(&mut imp.value.body);
}

pub fn walk_type_alias_mut<V: VisitMut + ?Sized>(v: &mut V, alias: &mut TypeAlias) {
    v.visit_type_annotation_mut(&mut alias.type_annotation);
}

pub fn walk_custom_type_mut<V: VisitMut + ?Sized>(v: &mut V, ct: &mut CustomType) {
    for ctor in &mut ct.constructors {
        v.visit_value_constructor_mut(ctor);
    }
}

pub fn walk_value_constructor_mut<V: VisitMut + ?Sized>(
    v: &mut V,
    ctor: &mut Spanned<ValueConstructor>,
) {
    for arg in &mut ctor.value.args {
        v.visit_type_annotation_mut(arg);
    }
}

pub fn walk_expr_mut<V: VisitMut + ?Sized>(v: &mut V, expr: &mut Spanned<Expr>) {
    for c in &mut expr.comments {
        v.visit_comment_mut(c);
    }
    match &mut expr.value {
        Expr::Unit | Expr::GLSLExpression(_) | Expr::RecordAccessFunction(_) => {}

        Expr::Literal(lit) => v.visit_literal_mut(lit),

        Expr::FunctionOrValue { name, .. } => v.visit_ident_mut(name),

        Expr::PrefixOperator(op) => v.visit_ident_mut(op),

        Expr::OperatorApplication { left, right, .. } => {
            v.visit_expr_mut(left);
            v.visit_expr_mut(right);
        }

        Expr::BinOps {
            operands_and_operators,
            final_operand,
        } => {
            for (operand, _op) in operands_and_operators {
                v.visit_expr_mut(operand);
            }
            v.visit_expr_mut(final_operand);
        }

        Expr::Application(args) => {
            for arg in args {
                v.visit_expr_mut(arg);
            }
        }

        Expr::IfElse {
            branches,
            else_branch,
        } => {
            for (cond, body) in branches {
                v.visit_expr_mut(cond);
                v.visit_expr_mut(body);
            }
            v.visit_expr_mut(else_branch);
        }

        Expr::Negation(inner) => v.visit_expr_mut(inner),

        Expr::Tuple(elems) | Expr::List(elems) => {
            for elem in elems {
                v.visit_expr_mut(elem);
            }
        }

        Expr::Parenthesized(inner) => v.visit_expr_mut(inner),

        Expr::LetIn {
            declarations,
            body,
            trailing_comments: _,
        } => {
            for decl in declarations {
                v.visit_let_declaration_mut(decl);
            }
            v.visit_expr_mut(body);
        }

        Expr::CaseOf {
            expr: subject,
            branches,
        } => {
            v.visit_expr_mut(subject);
            for branch in branches {
                v.visit_case_branch_mut(branch);
            }
        }

        Expr::Lambda { args, body } => {
            for arg in args {
                v.visit_pattern_mut(arg);
            }
            v.visit_expr_mut(body);
        }

        Expr::Record(fields) => {
            for field in fields {
                v.visit_record_setter_mut(field);
            }
        }

        Expr::RecordUpdate { updates, .. } => {
            for field in updates {
                v.visit_record_setter_mut(field);
            }
        }

        Expr::RecordAccess { record, .. } => {
            v.visit_expr_mut(record);
        }
    }
}

pub fn walk_pattern_mut<V: VisitMut + ?Sized>(v: &mut V, pattern: &mut Spanned<Pattern>) {
    for c in &mut pattern.comments {
        v.visit_comment_mut(c);
    }
    match &mut pattern.value {
        Pattern::Anything | Pattern::Unit | Pattern::Hex(_) => {}

        Pattern::Var(name) => v.visit_ident_mut(name),

        Pattern::Literal(lit) => v.visit_literal_mut(lit),

        Pattern::Tuple(elems) | Pattern::List(elems) => {
            for elem in elems {
                v.visit_pattern_mut(elem);
            }
        }

        Pattern::Constructor { args, .. } => {
            for arg in args {
                v.visit_pattern_mut(arg);
            }
        }

        Pattern::Record(fields) => {
            for field in fields {
                v.visit_ident_mut(&mut field.value);
            }
        }

        Pattern::Cons { head, tail } => {
            v.visit_pattern_mut(head);
            v.visit_pattern_mut(tail);
        }

        Pattern::As {
            pattern: inner,
            name,
        } => {
            v.visit_pattern_mut(inner);
            v.visit_ident_mut(&mut name.value);
        }

        Pattern::Parenthesized(inner) => v.visit_pattern_mut(inner),
    }
}

pub fn walk_type_annotation_mut<V: VisitMut + ?Sized>(v: &mut V, ty: &mut Spanned<TypeAnnotation>) {
    match &mut ty.value {
        TypeAnnotation::GenericType(_) | TypeAnnotation::Unit => {}

        TypeAnnotation::Typed { args, .. } => {
            for arg in args {
                v.visit_type_annotation_mut(arg);
            }
        }

        TypeAnnotation::Tupled(elems) => {
            for elem in elems {
                v.visit_type_annotation_mut(elem);
            }
        }

        TypeAnnotation::Record(fields) => {
            for field in fields {
                v.visit_record_field_mut(field);
            }
        }

        TypeAnnotation::GenericRecord { fields, .. } => {
            for field in fields {
                v.visit_record_field_mut(field);
            }
        }

        TypeAnnotation::FunctionType { from, to } => {
            v.visit_type_annotation_mut(from);
            v.visit_type_annotation_mut(to);
        }
    }
}

pub fn walk_record_field_mut<V: VisitMut + ?Sized>(v: &mut V, field: &mut Spanned<RecordField>) {
    v.visit_type_annotation_mut(&mut field.value.type_annotation);
}

pub fn walk_let_declaration_mut<V: VisitMut + ?Sized>(
    v: &mut V,
    decl: &mut Spanned<LetDeclaration>,
) {
    for c in &mut decl.comments {
        v.visit_comment_mut(c);
    }
    match &mut decl.value {
        LetDeclaration::Function(func) => v.visit_function_mut(func),
        LetDeclaration::Destructuring { pattern, body } => {
            v.visit_pattern_mut(pattern);
            v.visit_expr_mut(body);
        }
    }
}

pub fn walk_case_branch_mut<V: VisitMut + ?Sized>(v: &mut V, branch: &mut CaseBranch) {
    v.visit_pattern_mut(&mut branch.pattern);
    v.visit_expr_mut(&mut branch.body);
}

pub fn walk_record_setter_mut<V: VisitMut + ?Sized>(v: &mut V, setter: &mut Spanned<RecordSetter>) {
    v.visit_expr_mut(&mut setter.value.value);
}
