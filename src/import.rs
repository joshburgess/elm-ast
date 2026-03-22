use crate::exposing::Exposing;
use crate::ident::ModuleName;
use crate::node::Spanned;

/// An import declaration.
///
/// Examples:
/// - `import Html`
/// - `import Html exposing (div, text)`
/// - `import Json.Decode as Decode exposing (Decoder)`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// The module being imported: `Html.Attributes`
    pub module_name: Spanned<ModuleName>,

    /// Optional alias: `as HA`
    pub alias: Option<Spanned<ModuleName>>,

    /// Optional exposing list: `exposing (style, class)`
    pub exposing: Option<Spanned<Exposing>>,
}
