use crate::comment::Comment;
use crate::expr::{Function, Signature};
use crate::ident::Ident;
use crate::node::Spanned;
use crate::operator::{InfixDirection, Precedence};
use crate::pattern::Pattern;
use crate::type_annotation::TypeAnnotation;

/// A top-level declaration in an Elm module.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueConstructor {
    pub name: Spanned<Ident>,
    pub args: Vec<Spanned<TypeAnnotation>>,
    /// Comments that appeared BEFORE the preceding `|` separator (or for the
    /// first constructor, before `=`). elm-format preserves these as
    /// "trailing on previous constructor" style: they appear on their own
    /// line(s) at the constructor-name column, with the `|` for THIS
    /// constructor coming afterward on a new line. Leave empty for the
    /// first constructor.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pre_pipe_comments: Vec<Spanned<Comment>>,
    /// Optional trailing inline comment on the same source line as the
    /// constructor: `| Ctor args -- comment`. elm-format keeps the comment
    /// on the same line after the last arg.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub trailing_comment: Option<Spanned<Comment>>,
}

/// An infix operator definition.
///
/// `infix left 6 (+) = add`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfixDef {
    pub direction: Spanned<InfixDirection>,
    pub precedence: Spanned<Precedence>,
    pub operator: Spanned<Ident>,
    pub function: Spanned<Ident>,
}
