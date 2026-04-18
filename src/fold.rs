//! Owned transformation trait for rewriting Elm ASTs.
//!
//! [`Fold`] takes ownership of nodes and returns transformed versions.
//! Override `fold_*` methods to rewrite specific node types; default
//! implementations recursively fold children.

use crate::comment::Comment;
use crate::declaration::{CustomType, Declaration, InfixDef, TypeAlias, ValueConstructor};
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

/// Owned transformation trait. Override methods to rewrite AST nodes;
/// call the corresponding `fold_*` function to continue descent.
#[allow(unused_variables)]
pub trait Fold {
    fn fold_module(&mut self, module: ElmModule) -> ElmModule {
        fold_module(self, module)
    }

    fn fold_module_header(&mut self, header: Spanned<ModuleHeader>) -> Spanned<ModuleHeader> {
        header
    }

    fn fold_import(&mut self, import: Spanned<Import>) -> Spanned<Import> {
        import
    }

    fn fold_declaration(&mut self, decl: Spanned<Declaration>) -> Spanned<Declaration> {
        fold_declaration(self, decl)
    }

    fn fold_function(&mut self, func: Function) -> Function {
        fold_function(self, func)
    }

    fn fold_signature(&mut self, sig: Spanned<Signature>) -> Spanned<Signature> {
        fold_signature(self, sig)
    }

    fn fold_function_implementation(
        &mut self,
        imp: Spanned<FunctionImplementation>,
    ) -> Spanned<FunctionImplementation> {
        fold_function_implementation(self, imp)
    }

    fn fold_type_alias(&mut self, alias: TypeAlias) -> TypeAlias {
        fold_type_alias(self, alias)
    }

    fn fold_custom_type(&mut self, ct: CustomType) -> CustomType {
        fold_custom_type(self, ct)
    }

    fn fold_value_constructor(
        &mut self,
        ctor: Spanned<ValueConstructor>,
    ) -> Spanned<ValueConstructor> {
        fold_value_constructor(self, ctor)
    }

    fn fold_infix_def(&mut self, infix: InfixDef) -> InfixDef {
        infix
    }

    fn fold_expr(&mut self, expr: Spanned<Expr>) -> Spanned<Expr> {
        fold_expr(self, expr)
    }

    fn fold_pattern(&mut self, pattern: Spanned<Pattern>) -> Spanned<Pattern> {
        fold_pattern(self, pattern)
    }

    fn fold_type_annotation(&mut self, ty: Spanned<TypeAnnotation>) -> Spanned<TypeAnnotation> {
        fold_type_annotation(self, ty)
    }

    fn fold_record_field(&mut self, field: Spanned<RecordField>) -> Spanned<RecordField> {
        fold_record_field(self, field)
    }

    fn fold_let_declaration(&mut self, decl: Spanned<LetDeclaration>) -> Spanned<LetDeclaration> {
        fold_let_declaration(self, decl)
    }

    fn fold_case_branch(&mut self, branch: CaseBranch) -> CaseBranch {
        fold_case_branch(self, branch)
    }

    fn fold_record_setter(&mut self, setter: Spanned<RecordSetter>) -> Spanned<RecordSetter> {
        fold_record_setter(self, setter)
    }

    fn fold_literal(&mut self, lit: Literal) -> Literal {
        lit
    }

    fn fold_comment(&mut self, comment: Spanned<Comment>) -> Spanned<Comment> {
        comment
    }

    fn fold_ident(&mut self, name: String) -> String {
        name
    }
}

// ── Fold functions ───────────────────────────────────────────────────

pub fn fold_module<F: Fold + ?Sized>(f: &mut F, module: ElmModule) -> ElmModule {
    let header = f.fold_module_header(module.header);
    let imports = module
        .imports
        .into_iter()
        .map(|i| f.fold_import(i))
        .collect();
    let declarations = module
        .declarations
        .into_iter()
        .map(|d| f.fold_declaration(d))
        .collect();
    let comments = module
        .comments
        .into_iter()
        .map(|c| f.fold_comment(c))
        .collect();
    ElmModule {
        header,
        module_documentation: module.module_documentation,
        imports,
        declarations,
        comments,
    }
}

pub fn fold_declaration<F: Fold + ?Sized>(
    f: &mut F,
    decl: Spanned<Declaration>,
) -> Spanned<Declaration> {
    let span = decl.span;
    let value = match decl.value {
        Declaration::FunctionDeclaration(func) => {
            Declaration::FunctionDeclaration(Box::new(f.fold_function(*func)))
        }
        Declaration::AliasDeclaration(alias) => {
            Declaration::AliasDeclaration(f.fold_type_alias(alias))
        }
        Declaration::CustomTypeDeclaration(ct) => {
            Declaration::CustomTypeDeclaration(f.fold_custom_type(ct))
        }
        Declaration::PortDeclaration(sig) => {
            let sig_spanned = Spanned::new(span, sig);
            let folded = f.fold_signature(sig_spanned);
            Declaration::PortDeclaration(folded.value)
        }
        Declaration::InfixDeclaration(infix) => {
            Declaration::InfixDeclaration(f.fold_infix_def(infix))
        }
        Declaration::Destructuring { pattern, body } => Declaration::Destructuring {
            pattern: f.fold_pattern(pattern),
            body: f.fold_expr(body),
        },
    };
    Spanned::new(span, value)
}

pub fn fold_function<F: Fold + ?Sized>(f: &mut F, func: Function) -> Function {
    Function {
        documentation: func.documentation,
        signature: func.signature.map(|s| f.fold_signature(s)),
        declaration: f.fold_function_implementation(func.declaration),
    }
}

pub fn fold_signature<F: Fold + ?Sized>(f: &mut F, sig: Spanned<Signature>) -> Spanned<Signature> {
    let span = sig.span;
    let value = Signature {
        name: sig.value.name.map(|n| f.fold_ident(n)),
        type_annotation: f.fold_type_annotation(sig.value.type_annotation),
    };
    Spanned::new(span, value)
}

pub fn fold_function_implementation<F: Fold + ?Sized>(
    f: &mut F,
    imp: Spanned<FunctionImplementation>,
) -> Spanned<FunctionImplementation> {
    let span = imp.span;
    let value = FunctionImplementation {
        name: imp.value.name.map(|n| f.fold_ident(n)),
        args: imp
            .value
            .args
            .into_iter()
            .map(|a| f.fold_pattern(a))
            .collect(),
        body: f.fold_expr(imp.value.body),
    };
    Spanned::new(span, value)
}

pub fn fold_type_alias<F: Fold + ?Sized>(f: &mut F, alias: TypeAlias) -> TypeAlias {
    TypeAlias {
        documentation: alias.documentation,
        name: alias.name,
        generics: alias.generics,
        type_annotation: f.fold_type_annotation(alias.type_annotation),
    }
}

pub fn fold_custom_type<F: Fold + ?Sized>(f: &mut F, ct: CustomType) -> CustomType {
    CustomType {
        documentation: ct.documentation,
        name: ct.name,
        generics: ct.generics,
        constructors: ct
            .constructors
            .into_iter()
            .map(|c| f.fold_value_constructor(c))
            .collect(),
    }
}

pub fn fold_value_constructor<F: Fold + ?Sized>(
    f: &mut F,
    ctor: Spanned<ValueConstructor>,
) -> Spanned<ValueConstructor> {
    let span = ctor.span;
    let value = ValueConstructor {
        name: ctor.value.name,
        args: ctor
            .value
            .args
            .into_iter()
            .map(|a| f.fold_type_annotation(a))
            .collect(),
        pre_pipe_comments: ctor.value.pre_pipe_comments,
    };
    Spanned::new(span, value)
}

pub fn fold_expr<F: Fold + ?Sized>(f: &mut F, expr: Spanned<Expr>) -> Spanned<Expr> {
    let span = expr.span;
    let comments: Vec<_> = expr
        .comments
        .into_iter()
        .map(|c| f.fold_comment(c))
        .collect();
    let value = match expr.value {
        Expr::Unit | Expr::GLSLExpression(_) | Expr::RecordAccessFunction(_) => expr.value,

        Expr::Literal(lit) => Expr::Literal(f.fold_literal(lit)),

        Expr::FunctionOrValue { module_name, name } => Expr::FunctionOrValue {
            module_name,
            name: f.fold_ident(name),
        },

        Expr::PrefixOperator(op) => Expr::PrefixOperator(f.fold_ident(op)),

        Expr::OperatorApplication {
            operator,
            direction,
            left,
            right,
        } => Expr::OperatorApplication {
            operator,
            direction,
            left: Box::new(f.fold_expr(*left)),
            right: Box::new(f.fold_expr(*right)),
        },

        Expr::BinOps {
            operands_and_operators,
            final_operand,
        } => Expr::BinOps {
            operands_and_operators: operands_and_operators
                .into_iter()
                .map(|(e, op)| (f.fold_expr(e), op))
                .collect(),
            final_operand: Box::new(f.fold_expr(*final_operand)),
        },

        Expr::Application(args) => {
            Expr::Application(args.into_iter().map(|a| f.fold_expr(a)).collect())
        }

        Expr::IfElse {
            branches,
            else_branch,
        } => Expr::IfElse {
            branches: branches
                .into_iter()
                .map(|(c, b)| (f.fold_expr(c), f.fold_expr(b)))
                .collect(),
            else_branch: Box::new(f.fold_expr(*else_branch)),
        },

        Expr::Negation(inner) => Expr::Negation(Box::new(f.fold_expr(*inner))),

        Expr::Tuple(elems) => Expr::Tuple(elems.into_iter().map(|e| f.fold_expr(e)).collect()),

        Expr::List(elems) => Expr::List(elems.into_iter().map(|e| f.fold_expr(e)).collect()),

        Expr::Parenthesized(inner) => Expr::Parenthesized(Box::new(f.fold_expr(*inner))),

        Expr::LetIn {
            declarations,
            body,
            trailing_comments,
        } => Expr::LetIn {
            declarations: declarations
                .into_iter()
                .map(|d| f.fold_let_declaration(d))
                .collect(),
            body: Box::new(f.fold_expr(*body)),
            trailing_comments,
        },

        Expr::CaseOf {
            expr: subject,
            branches,
        } => Expr::CaseOf {
            expr: Box::new(f.fold_expr(*subject)),
            branches: branches
                .into_iter()
                .map(|b| f.fold_case_branch(b))
                .collect(),
        },

        Expr::Lambda { args, body } => Expr::Lambda {
            args: args.into_iter().map(|a| f.fold_pattern(a)).collect(),
            body: Box::new(f.fold_expr(*body)),
        },

        Expr::Record(fields) => Expr::Record(
            fields
                .into_iter()
                .map(|s| f.fold_record_setter(s))
                .collect(),
        ),

        Expr::RecordUpdate { base, updates } => Expr::RecordUpdate {
            base,
            updates: updates
                .into_iter()
                .map(|s| f.fold_record_setter(s))
                .collect(),
        },

        Expr::RecordAccess { record, field } => Expr::RecordAccess {
            record: Box::new(f.fold_expr(*record)),
            field,
        },
    };
    Spanned::new(span, value).with_comments(comments)
}

pub fn fold_pattern<F: Fold + ?Sized>(f: &mut F, pattern: Spanned<Pattern>) -> Spanned<Pattern> {
    let span = pattern.span;
    let comments: Vec<_> = pattern
        .comments
        .into_iter()
        .map(|c| f.fold_comment(c))
        .collect();
    let value = match pattern.value {
        Pattern::Anything | Pattern::Unit | Pattern::Hex(_) | Pattern::Literal(_) => pattern.value,

        Pattern::Var(name) => Pattern::Var(f.fold_ident(name)),

        Pattern::Tuple(elems) => {
            Pattern::Tuple(elems.into_iter().map(|e| f.fold_pattern(e)).collect())
        }

        Pattern::List(elems) => {
            Pattern::List(elems.into_iter().map(|e| f.fold_pattern(e)).collect())
        }

        Pattern::Constructor {
            module_name,
            name,
            args,
        } => Pattern::Constructor {
            module_name,
            name,
            args: args.into_iter().map(|a| f.fold_pattern(a)).collect(),
        },

        Pattern::Record(fields) => Pattern::Record(fields),

        Pattern::Cons { head, tail } => Pattern::Cons {
            head: Box::new(f.fold_pattern(*head)),
            tail: Box::new(f.fold_pattern(*tail)),
        },

        Pattern::As {
            pattern: inner,
            name,
        } => Pattern::As {
            pattern: Box::new(f.fold_pattern(*inner)),
            name,
        },

        Pattern::Parenthesized(inner) => Pattern::Parenthesized(Box::new(f.fold_pattern(*inner))),
    };
    Spanned::new(span, value).with_comments(comments)
}

pub fn fold_type_annotation<F: Fold + ?Sized>(
    f: &mut F,
    ty: Spanned<TypeAnnotation>,
) -> Spanned<TypeAnnotation> {
    let span = ty.span;
    let value = match ty.value {
        TypeAnnotation::GenericType(_) | TypeAnnotation::Unit => ty.value,

        TypeAnnotation::Typed {
            module_name,
            name,
            args,
        } => TypeAnnotation::Typed {
            module_name,
            name,
            args: args
                .into_iter()
                .map(|a| f.fold_type_annotation(a))
                .collect(),
        },

        TypeAnnotation::Tupled(elems) => TypeAnnotation::Tupled(
            elems
                .into_iter()
                .map(|e| f.fold_type_annotation(e))
                .collect(),
        ),

        TypeAnnotation::Record(fields) => TypeAnnotation::Record(
            fields
                .into_iter()
                .map(|field| f.fold_record_field(field))
                .collect(),
        ),

        TypeAnnotation::GenericRecord { base, fields } => TypeAnnotation::GenericRecord {
            base,
            fields: fields
                .into_iter()
                .map(|field| f.fold_record_field(field))
                .collect(),
        },

        TypeAnnotation::FunctionType { from, to } => TypeAnnotation::FunctionType {
            from: Box::new(f.fold_type_annotation(*from)),
            to: Box::new(f.fold_type_annotation(*to)),
        },
    };
    Spanned::new(span, value)
}

pub fn fold_record_field<F: Fold + ?Sized>(
    f: &mut F,
    field: Spanned<RecordField>,
) -> Spanned<RecordField> {
    let span = field.span;
    let value = RecordField {
        name: field.value.name,
        type_annotation: f.fold_type_annotation(field.value.type_annotation),
    };
    Spanned::new(span, value)
}

pub fn fold_let_declaration<F: Fold + ?Sized>(
    f: &mut F,
    decl: Spanned<LetDeclaration>,
) -> Spanned<LetDeclaration> {
    let span = decl.span;
    let comments: Vec<_> = decl
        .comments
        .into_iter()
        .map(|c| f.fold_comment(c))
        .collect();
    let value = match decl.value {
        LetDeclaration::Function(func) => {
            LetDeclaration::Function(Box::new(f.fold_function(*func)))
        }
        LetDeclaration::Destructuring { pattern, body } => LetDeclaration::Destructuring {
            pattern: Box::new(f.fold_pattern(*pattern)),
            body: Box::new(f.fold_expr(*body)),
        },
    };
    Spanned::new(span, value).with_comments(comments)
}

pub fn fold_case_branch<F: Fold + ?Sized>(f: &mut F, branch: CaseBranch) -> CaseBranch {
    CaseBranch {
        pattern: f.fold_pattern(branch.pattern),
        body: f.fold_expr(branch.body),
    }
}

pub fn fold_record_setter<F: Fold + ?Sized>(
    f: &mut F,
    setter: Spanned<RecordSetter>,
) -> Spanned<RecordSetter> {
    let span = setter.span;
    let value = RecordSetter {
        field: setter.value.field,
        value: f.fold_expr(setter.value.value),
        trailing_comment: setter.value.trailing_comment,
    };
    Spanned::new(span, value)
}
