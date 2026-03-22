/// The associativity of an infix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InfixDirection {
    Left,
    Right,
    Non,
}

/// Operator precedence (0–9 in Elm).
pub type Precedence = u8;

/// An infix operator declaration.
///
/// Corresponds to: `infix left 6 (+) = add`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfixDeclaration {
    /// The operator symbol, e.g. `+`
    pub operator: String,
    /// The function it desugars to, e.g. `add`
    pub function: String,
    /// Associativity direction.
    pub direction: InfixDirection,
    /// Precedence level (0–9).
    pub precedence: Precedence,
}
