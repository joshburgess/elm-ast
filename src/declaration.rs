use crate::expr::{Function, Signature};
use crate::ident::Ident;
use crate::node::Spanned;
use crate::operator::{InfixDirection, Precedence};
use crate::pattern::Pattern;
use crate::type_annotation::TypeAnnotation;

/// A top-level declaration in an Elm module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Declaration {
    /// A function or value definition.
    ///
    /// ```elm
    /// add : Int -> Int -> Int
    /// add x y = x + y
    /// ```
    FunctionDeclaration(Box<Function>),

    /// A type alias declaration.
    ///
    /// ```elm
    /// type alias Model =
    ///     { count : Int
    ///     , name : String
    ///     }
    /// ```
    AliasDeclaration(TypeAlias),

    /// A custom type (ADT / tagged union) declaration.
    ///
    /// ```elm
    /// type Msg
    ///     = Increment
    ///     | Decrement
    ///     | SetName String
    /// ```
    CustomTypeDeclaration(CustomType),

    /// A port declaration.
    ///
    /// ```elm
    /// port sendMessage : String -> Cmd msg
    /// ```
    PortDeclaration(Signature),

    /// An infix operator declaration (only in core packages).
    ///
    /// ```elm
    /// infix left 6 (+) = add
    /// ```
    InfixDeclaration(InfixDef),

    /// A top-level destructuring (rare, but valid).
    ///
    /// ```elm
    /// { name, age } = person
    /// ```
    Destructuring {
        pattern: Spanned<Pattern>,
        body: Spanned<crate::expr::Expr>,
    },
}

/// A type alias definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAlias {
    /// Optional documentation comment.
    pub documentation: Option<Spanned<String>>,

    /// The alias name: `Model`
    pub name: Spanned<Ident>,

    /// Type parameters: `a`, `b` in `type alias Result a b = ...`
    pub generics: Vec<Spanned<Ident>>,

    /// The type being aliased.
    pub type_annotation: Spanned<TypeAnnotation>,
}

/// A custom type (ADT / tagged union) definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomType {
    /// Optional documentation comment.
    pub documentation: Option<Spanned<String>>,

    /// The type name: `Msg`
    pub name: Spanned<Ident>,

    /// Type parameters: `a` in `type Maybe a = ...`
    pub generics: Vec<Spanned<Ident>>,

    /// The constructors: `Just a | Nothing`
    pub constructors: Vec<Spanned<ValueConstructor>>,
}

/// A value constructor in a custom type definition.
///
/// `Just a` → `ValueConstructor { name: "Just", args: [a] }`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueConstructor {
    pub name: Spanned<Ident>,
    pub args: Vec<Spanned<TypeAnnotation>>,
}

/// An infix operator definition.
///
/// `infix left 6 (+) = add`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfixDef {
    pub direction: Spanned<InfixDirection>,
    pub precedence: Spanned<Precedence>,
    pub operator: Spanned<Ident>,
    pub function: Spanned<Ident>,
}
