//! `Display` implementations for AST types.
//!
//! These use the printer to produce valid Elm source text.
//! Requires the `printing` feature.

use std::fmt;

use crate::declaration::Declaration;
use crate::expr::Expr;
use crate::file::ElmModule;
use crate::node::Spanned;
use crate::pattern::Pattern;
use crate::print::{PrintConfig, Printer};
use crate::type_annotation::TypeAnnotation;

impl fmt::Display for ElmModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = Printer::new(PrintConfig::default()).print_module(self);
        f.write_str(&output)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer::new(PrintConfig::default());
        p.write_expr(self);
        f.write_str(&p.finish())
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer::new(PrintConfig::default());
        p.write_pattern(self);
        f.write_str(&p.finish())
    }
}

impl fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer::new(PrintConfig::default());
        p.write_type(self);
        f.write_str(&p.finish())
    }
}

impl fmt::Display for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer::new(PrintConfig::default());
        p.write_declaration(self);
        f.write_str(&p.finish())
    }
}

// Display for Spanned<T> delegates to the inner value.
impl<T: fmt::Display> fmt::Display for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}
