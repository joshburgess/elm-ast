//! Builder helpers for constructing Elm AST nodes programmatically.
//!
//! These create nodes with dummy spans — useful for code generation
//! where source locations don't matter.

use crate::declaration::{CustomType, Declaration, TypeAlias, ValueConstructor};
use crate::exposing::Exposing;
use crate::expr::{Expr, Function, FunctionImplementation, IfBranch, RecordSetter, Signature};
use crate::file::ElmModule;
use crate::import::Import;
use crate::literal::Literal;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::pattern::Pattern;
use crate::span::Span;
use crate::type_annotation::TypeAnnotation;

/// Wrap a value in a `Spanned` with a dummy span.
pub fn spanned<T>(value: T) -> Spanned<T> {
    Spanned::dummy(value)
}

// ── Expressions ──────────────────────────────────────────────────────

/// Create an integer literal expression.
pub fn int(n: i64) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Int(n)))
}

/// Create a float literal expression.
pub fn float(n: f64) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Float(n, None)))
}

/// Create a string literal expression.
pub fn string(s: impl Into<String>) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::String(s.into())))
}

/// Create a char literal expression.
pub fn char_lit(c: char) -> Spanned<Expr> {
    spanned(Expr::Literal(Literal::Char(c)))
}

/// Create a reference to a value or function: `foo`
pub fn var(name: impl Into<String>) -> Spanned<Expr> {
    spanned(Expr::FunctionOrValue {
        module_name: Vec::new(),
        name: name.into(),
    })
}

/// Create a qualified reference: `List.map`, `Maybe.Just`
pub fn qualified(module: &[&str], name: impl Into<String>) -> Spanned<Expr> {
    spanned(Expr::FunctionOrValue {
        module_name: module.iter().map(|s| s.to_string()).collect(),
        name: name.into(),
    })
}

/// Create a function application: `f a b c`
pub fn app(func: Spanned<Expr>, args: Vec<Spanned<Expr>>) -> Spanned<Expr> {
    let mut all = vec![func];
    all.extend(args);
    spanned(Expr::Application(all))
}

/// Create a binary operator application: `a + b`
pub fn binop(op: impl Into<String>, left: Spanned<Expr>, right: Spanned<Expr>) -> Spanned<Expr> {
    spanned(Expr::OperatorApplication {
        operator: op.into(),
        direction: crate::operator::InfixDirection::Left,
        left: Box::new(left),
        right: Box::new(right),
    })
}

/// Create a list expression: `[ a, b, c ]`
pub fn list(elements: Vec<Spanned<Expr>>) -> Spanned<Expr> {
    spanned(Expr::List(elements))
}

/// Create a tuple expression: `( a, b )`
pub fn tuple(elements: Vec<Spanned<Expr>>) -> Spanned<Expr> {
    spanned(Expr::Tuple(elements))
}

/// Create a record expression: `{ name = "Alice", age = 30 }`
pub fn record(fields: Vec<(impl Into<String>, Spanned<Expr>)>) -> Spanned<Expr> {
    spanned(Expr::Record(
        fields
            .into_iter()
            .map(|(name, value)| {
                spanned(RecordSetter {
                    field: spanned(name.into()),
                    value,
                    trailing_comment: None,
                })
            })
            .collect(),
    ))
}

/// Create an if-then-else expression.
pub fn if_else(
    condition: Spanned<Expr>,
    then_branch: Spanned<Expr>,
    else_branch: Spanned<Expr>,
) -> Spanned<Expr> {
    spanned(Expr::IfElse {
        branches: vec![IfBranch {
            condition,
            then_branch,
            trailing_comments: Vec::new(),
        }],
        else_branch: Box::new(else_branch),
    })
}

/// Create a lambda expression: `\a b -> body`
pub fn lambda(args: Vec<Spanned<Pattern>>, body: Spanned<Expr>) -> Spanned<Expr> {
    spanned(Expr::Lambda {
        args,
        body: Box::new(body),
    })
}

/// Unit expression: `()`
pub fn unit() -> Spanned<Expr> {
    spanned(Expr::Unit)
}

// ── Patterns ─────────────────────────────────────────────────────────

/// Create a variable pattern: `x`
pub fn pvar(name: impl Into<String>) -> Spanned<Pattern> {
    spanned(Pattern::Var(name.into()))
}

/// Create a wildcard pattern: `_`
pub fn pwild() -> Spanned<Pattern> {
    spanned(Pattern::Anything)
}

/// Create an integer literal pattern.
pub fn pint(n: i64) -> Spanned<Pattern> {
    spanned(Pattern::Literal(Literal::Int(n)))
}

/// Create a constructor pattern: `Just x`
pub fn pctor(name: impl Into<String>, args: Vec<Spanned<Pattern>>) -> Spanned<Pattern> {
    spanned(Pattern::Constructor {
        module_name: Vec::new(),
        name: name.into(),
        args,
    })
}

/// Create a record destructuring pattern: `{ x, y }`
pub fn precord(fields: Vec<impl Into<String>>) -> Spanned<Pattern> {
    spanned(Pattern::Record(
        fields.into_iter().map(|f| spanned(f.into())).collect(),
    ))
}

// ── Type annotations ─────────────────────────────────────────────────

/// Create a named type: `Int`, `String`, `Maybe a`
pub fn tname(
    name: impl Into<String>,
    args: Vec<Spanned<TypeAnnotation>>,
) -> Spanned<TypeAnnotation> {
    spanned(TypeAnnotation::Typed {
        module_name: Vec::new(),
        name: spanned(name.into()),
        args,
    })
}

/// Create a type variable: `a`, `msg`
pub fn tvar(name: impl Into<String>) -> Spanned<TypeAnnotation> {
    spanned(TypeAnnotation::GenericType(name.into()))
}

/// Create a function type: `a -> b`
pub fn tfunc(
    from: Spanned<TypeAnnotation>,
    to: Spanned<TypeAnnotation>,
) -> Spanned<TypeAnnotation> {
    spanned(TypeAnnotation::FunctionType {
        from: Box::new(from),
        to: Box::new(to),
    })
}

/// Unit type: `()`
pub fn tunit() -> Spanned<TypeAnnotation> {
    spanned(TypeAnnotation::Unit)
}

// ── Declarations ─────────────────────────────────────────────────────

/// Create a simple function declaration (no type signature).
pub fn func(
    name: impl Into<String>,
    args: Vec<Spanned<Pattern>>,
    body: Spanned<Expr>,
) -> Spanned<Declaration> {
    spanned(Declaration::FunctionDeclaration(Box::new(Function {
        documentation: None,
        signature: None,
        declaration: spanned(FunctionImplementation {
            name: spanned(name.into()),
            args,
            body,
        }),
    })))
}

/// Create a function declaration with a type signature.
pub fn func_with_sig(
    name: impl Into<String>,
    args: Vec<Spanned<Pattern>>,
    body: Spanned<Expr>,
    type_ann: Spanned<TypeAnnotation>,
) -> Spanned<Declaration> {
    let name_str: String = name.into();
    spanned(Declaration::FunctionDeclaration(Box::new(Function {
        documentation: None,
        signature: Some(spanned(Signature {
            name: spanned(name_str.clone()),
            type_annotation: type_ann,
        })),
        declaration: spanned(FunctionImplementation {
            name: spanned(name_str),
            args,
            body,
        }),
    })))
}

/// Create a type alias declaration.
pub fn type_alias(
    name: impl Into<String>,
    generics: Vec<impl Into<String>>,
    type_ann: Spanned<TypeAnnotation>,
) -> Spanned<Declaration> {
    spanned(Declaration::AliasDeclaration(TypeAlias {
        documentation: None,
        name: spanned(name.into()),
        generics: generics.into_iter().map(|g| spanned(g.into())).collect(),
        type_annotation: type_ann,
    }))
}

/// Create a custom type declaration.
pub fn custom_type(
    name: impl Into<String>,
    generics: Vec<impl Into<String>>,
    constructors: Vec<(impl Into<String>, Vec<Spanned<TypeAnnotation>>)>,
) -> Spanned<Declaration> {
    spanned(Declaration::CustomTypeDeclaration(CustomType {
        documentation: None,
        name: spanned(name.into()),
        generics: generics.into_iter().map(|g| spanned(g.into())).collect(),
        constructors: constructors
            .into_iter()
            .map(|(name, args)| {
                spanned(ValueConstructor {
                    name: spanned(name.into()),
                    args,
                    pre_pipe_comments: Vec::new(),
                })
            })
            .collect(),
    }))
}

// ── Module ───────────────────────────────────────────────────────────

/// Create a module with the given name and declarations.
pub fn module(name: Vec<impl Into<String>>, declarations: Vec<Spanned<Declaration>>) -> ElmModule {
    ElmModule {
        header: spanned(ModuleHeader::Normal {
            name: spanned(name.into_iter().map(|s| s.into()).collect()),
            exposing: spanned(Exposing::All(Span::dummy())),
        }),
        module_documentation: None,
        imports: Vec::new(),
        declarations,
        comments: Vec::new(),
    }
}

/// Create an import declaration.
pub fn import(module_name: Vec<impl Into<String>>) -> Spanned<Import> {
    spanned(Import {
        module_name: spanned(module_name.into_iter().map(|s| s.into()).collect()),
        alias: None,
        exposing: None,
    })
}
