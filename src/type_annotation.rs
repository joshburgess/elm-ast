use crate::ident::{Ident, ModuleName};
use crate::node::Spanned;

/// A type annotation in Elm source code.
///
/// Represents the syntax of types as written by the programmer, before any
/// resolution or canonicalization.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeAnnotation {
    /// A type variable: `a`, `msg`, `comparable`
    GenericType(Ident),

    /// A named type, possibly qualified, with type arguments.
    ///
    /// Examples:
    /// - `Int` → `Typed { module_name: [], name: "Int", args: [] }`
    /// - `Maybe Int` → `Typed { module_name: [], name: "Maybe", args: [Int] }`
    /// - `Dict.Dict String Int` → `Typed { module_name: ["Dict"], name: "Dict", args: [String, Int] }`
    Typed {
        module_name: ModuleName,
        name: Spanned<Ident>,
        args: Vec<Spanned<TypeAnnotation>>,
    },

    /// The unit type: `()`
    Unit,

    /// A tuple type: `( Int, String )` or `( Int, String, Bool )`
    ///
    /// Elm only supports 2-tuples and 3-tuples.
    Tupled(Vec<Spanned<TypeAnnotation>>),

    /// A record type: `{ name : String, age : Int }`
    Record(Vec<Spanned<RecordField>>),

    /// An extensible record type: `{ a | name : String, age : Int }`
    GenericRecord {
        base: Spanned<Ident>,
        fields: Vec<Spanned<RecordField>>,
    },

    /// A function type: `Int -> String`
    FunctionType {
        from: Box<Spanned<TypeAnnotation>>,
        to: Box<Spanned<TypeAnnotation>>,
    },
}

/// A single field in a record type: `name : String`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordField {
    pub name: Spanned<Ident>,
    pub type_annotation: Spanned<TypeAnnotation>,
}
