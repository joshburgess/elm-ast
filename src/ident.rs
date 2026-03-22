use crate::node::Spanned;

/// An identifier — a name in Elm source code.
///
/// Elm distinguishes between lowercase identifiers (values, type variables)
/// and uppercase identifiers (types, constructors, modules).
pub type Ident = String;

/// A module name: a dot-separated sequence of uppercase identifiers.
///
/// Example: `Html.Attributes` → `["Html", "Attributes"]`
pub type ModuleName = Vec<Ident>;

/// A qualified reference to a value or type.
///
/// Example: `Maybe.Just` → `QualifiedName { module_name: ["Maybe"], name: "Just" }`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub module_name: ModuleName,
    pub name: Ident,
}

/// A spanned identifier.
pub type SpannedIdent = Spanned<Ident>;

/// A spanned module name.
pub type SpannedModuleName = Spanned<ModuleName>;
