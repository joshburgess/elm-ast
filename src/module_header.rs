use crate::exposing::Exposing;
use crate::ident::ModuleName;
use crate::node::Spanned;

/// The module header declaration at the top of an Elm file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleHeader {
    /// `module Foo.Bar exposing (..)`
    Normal {
        name: Spanned<ModuleName>,
        exposing: Spanned<Exposing>,
    },

    /// `port module Foo.Bar exposing (..)`
    Port {
        name: Spanned<ModuleName>,
        exposing: Spanned<Exposing>,
    },

    /// `effect module Foo.Bar where { command = MyCmd } exposing (..)`
    ///
    /// Effect modules are internal to elm/core and elm/browser.
    Effect {
        name: Spanned<ModuleName>,
        exposing: Spanned<Exposing>,
        command: Option<Spanned<String>>,
        subscription: Option<Spanned<String>>,
    },
}
