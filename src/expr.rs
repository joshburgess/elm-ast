use crate::comment::Comment;
use crate::ident::{Ident, ModuleName};
use crate::literal::Literal;
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::pattern::Pattern;
use crate::type_annotation::TypeAnnotation;

/// An expression in Elm source code.
///
/// This covers every expression form in Elm 0.19.1.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// Unit expression: `()`
    Unit,

    /// A literal value: `42`, `"hello"`, `'c'`, `3.14`
    Literal(Literal),

    /// A reference to a value or constructor, possibly qualified.
    ///
    /// Examples:
    /// - `foo` → `FunctionOrValue { module_name: [], name: "foo" }`
    /// - `Just` → `FunctionOrValue { module_name: [], name: "Just" }`
    /// - `Maybe.Just` → `FunctionOrValue { module_name: ["Maybe"], name: "Just" }`
    FunctionOrValue {
        module_name: ModuleName,
        name: Ident,
    },

    /// An operator used as a prefix (in parentheses): `(+)`, `(::)`
    PrefixOperator(Ident),

    /// Operator application with resolved precedence and associativity:
    /// `a + b` → `OperatorApplication { operator: "+", direction: Left, left, right }`
    ///
    /// Note: in the source AST from elm/compiler this is `Binops`, a flat list.
    /// We use the resolved form from elm-syntax for ergonomics, but also provide
    /// `BinOps` below for representing the raw unresolved form.
    OperatorApplication {
        operator: Ident,
        direction: InfixDirection,
        left: Box<Spanned<Expr>>,
        right: Box<Spanned<Expr>>,
    },

    /// Raw unresolved binary operator chain, as in the source AST.
    ///
    /// `a + b * c` → `BinOps { operands_and_operators: [(a, +), (b, *)], final_operand: c }`
    ///
    /// This is the form directly from parsing, before operator precedence
    /// resolution. Corresponds to `Binops` in `AST/Source.hs`.
    BinOps {
        operands_and_operators: Vec<(Spanned<Expr>, Spanned<Ident>)>,
        final_operand: Box<Spanned<Expr>>,
    },

    /// Function application: `f x y` → `Application [f, x, y]`
    Application(Vec<Spanned<Expr>>),

    /// If-then-else expression: `if a then b else c`
    ///
    /// Chained if-else: `if a then b else if c then d else e`
    /// is represented as `IfElse { branches: [IfBranch(a, b), IfBranch(c, d)], else_branch: e }`
    IfElse {
        branches: Vec<IfBranch>,
        else_branch: Box<Spanned<Expr>>,
    },

    /// Negation: `-expr`
    Negation(Box<Spanned<Expr>>),

    /// Tuple expression: `( a, b )` or `( a, b, c )`
    Tuple(Vec<Spanned<Expr>>),

    /// Parenthesized expression: `( expr )`
    Parenthesized(Box<Spanned<Expr>>),

    /// Let-in expression:
    /// ```elm
    /// let
    ///     x = 1
    ///     y = 2
    /// in
    ///     x + y
    /// ```
    ///
    /// `trailing_comments` captures any comments that appear between the
    /// last declaration and the `in` keyword. elm-format preserves them
    /// as a dangling block at the end of the let body.
    LetIn {
        declarations: Vec<Spanned<LetDeclaration>>,
        body: Box<Spanned<Expr>>,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Vec::is_empty")
        )]
        trailing_comments: Vec<Spanned<Comment>>,
    },

    /// Case-of expression:
    /// ```elm
    /// case msg of
    ///     Increment -> model + 1
    ///     Decrement -> model - 1
    /// ```
    CaseOf {
        expr: Box<Spanned<Expr>>,
        branches: Vec<CaseBranch>,
    },

    /// Lambda expression: `\x y -> x + y`
    Lambda {
        args: Vec<Spanned<Pattern>>,
        body: Box<Spanned<Expr>>,
    },

    /// Record expression: `{ name = "Alice", age = 30 }`
    Record(Vec<Spanned<RecordSetter>>),

    /// Record update expression: `{ model | count = model.count + 1 }`
    RecordUpdate {
        base: Spanned<Ident>,
        updates: Vec<Spanned<RecordSetter>>,
    },

    /// Record field access: `model.count`
    RecordAccess {
        record: Box<Spanned<Expr>>,
        field: Spanned<Ident>,
    },

    /// Record access function: `.name`
    RecordAccessFunction(Ident),

    /// List expression: `[ 1, 2, 3 ]`
    List(Vec<Spanned<Expr>>),

    /// GLSL shader block: `[glsl| ... |]`
    GLSLExpression(String),
}

// Manual Eq impl because Expr contains Literal which contains f64.
impl Eq for Expr {}

/// A single branch of an if-else chain: `if <condition> then <then_branch>`.
///
/// `trailing_comments` captures any comments that appear after `then_branch`
/// and before the following `else` keyword. elm-format emits them as
/// trailing comments on the branch body.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfBranch {
    pub condition: Spanned<Expr>,
    pub then_branch: Spanned<Expr>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub trailing_comments: Vec<Spanned<Comment>>,
}

/// A field setter in a record expression or record update.
///
/// `name = expr`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordSetter {
    pub field: Spanned<Ident>,
    pub value: Spanned<Expr>,
    /// Optional trailing inline comment: `field = value -- comment`.
    /// elm-format keeps a short comment attached at the end of the setter,
    /// before the next `,` or `}`. Preserved only when the comment appears
    /// on the same source line as the value.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub trailing_comment: Option<Spanned<Comment>>,
}

/// A branch in a case-of expression.
///
/// `pattern -> expr`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseBranch {
    pub pattern: Spanned<Pattern>,
    pub body: Spanned<Expr>,
}

/// A declaration within a let-in block.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LetDeclaration {
    /// A function definition within a let block.
    ///
    /// ```elm
    /// let
    ///     add x y = x + y
    /// in
    ///     ...
    /// ```
    Function(Box<Function>),

    /// A destructuring within a let block.
    ///
    /// ```elm
    /// let
    ///     ( x, y ) = point
    /// in
    ///     ...
    /// ```
    Destructuring {
        pattern: Box<Spanned<Pattern>>,
        body: Box<Spanned<Expr>>,
    },
}

/// A function definition (used in both top-level declarations and let blocks).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// Optional documentation comment.
    pub documentation: Option<Spanned<String>>,

    /// Optional type signature: `add : Int -> Int -> Int`
    pub signature: Option<Spanned<Signature>>,

    /// The function implementation.
    pub declaration: Spanned<FunctionImplementation>,
}

/// A type signature: `name : type`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: Spanned<Ident>,
    pub type_annotation: Spanned<TypeAnnotation>,
}

/// The implementation part of a function definition: `name args = body`
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionImplementation {
    pub name: Spanned<Ident>,
    pub args: Vec<Spanned<Pattern>>,
    pub body: Spanned<Expr>,
}
