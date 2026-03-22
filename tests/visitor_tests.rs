use elm_ast_rs::expr::Expr;
use elm_ast_rs::fold::Fold;
use elm_ast_rs::literal::Literal;
use elm_ast_rs::node::Spanned;
use elm_ast_rs::pattern::Pattern;
use elm_ast_rs::type_annotation::TypeAnnotation;
use elm_ast_rs::visit::Visit;
use elm_ast_rs::visit_mut::VisitMut;
use elm_ast_rs::{parse, print};

// ── Visit: counting ──────────────────────────────────────────────────

struct ExprCounter(usize);

impl Visit for ExprCounter {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        self.0 += 1;
        elm_ast_rs::visit::walk_expr(self, expr);
    }
}

#[test]
fn visit_count_expressions() {
    let m = parse(
        "\
module Main exposing (..)

x = 1 + 2 * 3
",
    )
    .unwrap();

    let mut counter = ExprCounter(0);
    counter.visit_module(&m);
    // Expressions: (1 + (2 * 3)), 1, (2 * 3), 2, 3 = 5
    assert_eq!(counter.0, 5);
}

// ── Visit: collecting identifiers ────────────────────────────────────

struct IdentCollector(Vec<String>);

impl Visit for IdentCollector {
    fn visit_ident(&mut self, name: &str) {
        self.0.push(name.to_string());
    }
}

#[test]
fn visit_collect_identifiers() {
    let m = parse(
        "\
module Main exposing (..)

add x y = x + y
",
    )
    .unwrap();

    let mut collector = IdentCollector(Vec::new());
    collector.visit_module(&m);
    // Identifiers: "add" (from impl name), "x" (pattern), "y" (pattern),
    // "x" (expr ref), "y" (expr ref)
    assert!(collector.0.contains(&"add".to_string()));
    assert!(collector.0.contains(&"x".to_string()));
    assert!(collector.0.contains(&"y".to_string()));
}

// ── Visit: counting specific node types ──────────────────────────────

struct LambdaCounter(usize);

impl Visit for LambdaCounter {
    fn visit_expr(&mut self, expr: &Spanned<Expr>) {
        if matches!(&expr.value, Expr::Lambda { .. }) {
            self.0 += 1;
        }
        elm_ast_rs::visit::walk_expr(self, expr);
    }
}

#[test]
fn visit_count_lambdas() {
    let m = parse(
        "\
module Main exposing (..)

x = \\a -> \\b -> a + b
",
    )
    .unwrap();

    let mut counter = LambdaCounter(0);
    counter.visit_module(&m);
    assert_eq!(counter.0, 2);
}

// ── Visit: counting pattern types ────────────────────────────────────

struct WildcardCounter(usize);

impl Visit for WildcardCounter {
    fn visit_pattern(&mut self, pattern: &Spanned<Pattern>) {
        if matches!(&pattern.value, Pattern::Anything) {
            self.0 += 1;
        }
        elm_ast_rs::visit::walk_pattern(self, pattern);
    }
}

#[test]
fn visit_count_wildcards() {
    let m = parse(
        "\
module Main exposing (..)

f _ _ = 0
",
    )
    .unwrap();

    let mut counter = WildcardCounter(0);
    counter.visit_module(&m);
    assert_eq!(counter.0, 2);
}

// ── Visit: collecting type names ─────────────────────────────────────

struct TypeNameCollector(Vec<String>);

impl Visit for TypeNameCollector {
    fn visit_type_annotation(&mut self, ty: &Spanned<TypeAnnotation>) {
        if let TypeAnnotation::Typed { name, .. } = &ty.value {
            self.0.push(name.value.clone());
        }
        elm_ast_rs::visit::walk_type_annotation(self, ty);
    }
}

#[test]
fn visit_collect_type_names() {
    let m = parse(
        "\
module Main exposing (..)

foo : Maybe (List Int) -> String
foo x = \"hello\"
",
    )
    .unwrap();

    let mut collector = TypeNameCollector(Vec::new());
    collector.visit_module(&m);
    assert!(collector.0.contains(&"Maybe".to_string()));
    assert!(collector.0.contains(&"List".to_string()));
    assert!(collector.0.contains(&"Int".to_string()));
    assert!(collector.0.contains(&"String".to_string()));
}

// ── Visit: counting declarations ─────────────────────────────────────

struct DeclCounter(usize);

impl Visit for DeclCounter {
    fn visit_declaration(&mut self, _decl: &Spanned<elm_ast_rs::declaration::Declaration>) {
        self.0 += 1;
        elm_ast_rs::visit::walk_declaration(self, _decl);
    }
}

#[test]
fn visit_count_declarations() {
    let m = parse(
        "\
module Main exposing (..)

type Msg = A | B

type alias Model = Int

update msg model = model
",
    )
    .unwrap();

    let mut counter = DeclCounter(0);
    counter.visit_module(&m);
    assert_eq!(counter.0, 3);
}

// ── VisitMut: renaming variables ─────────────────────────────────────

struct Renamer {
    from: String,
    to: String,
}

impl VisitMut for Renamer {
    fn visit_ident_mut(&mut self, name: &mut String) {
        if *name == self.from {
            *name = self.to.clone();
        }
    }
}

#[test]
fn visit_mut_rename_variable() {
    let mut m = parse(
        "\
module Main exposing (..)

foo x = x + 1
",
    )
    .unwrap();

    let mut renamer = Renamer {
        from: "foo".into(),
        to: "bar".into(),
    };
    renamer.visit_module_mut(&mut m);

    let output = print::print(&m);
    assert!(output.contains("bar x ="));
    assert!(!output.contains("foo x ="));
}

#[test]
fn visit_mut_rename_all_occurrences() {
    let mut m = parse(
        "\
module Main exposing (..)

x = x + x
",
    )
    .unwrap();

    let mut renamer = Renamer {
        from: "x".into(),
        to: "y".into(),
    };
    renamer.visit_module_mut(&mut m);

    let output = print::print(&m);
    // Both uses of x in the body should be renamed.
    assert!(output.contains("y + y"));
}

// ── VisitMut: incrementing all integer literals ──────────────────────

struct IntIncrementer;

impl VisitMut for IntIncrementer {
    fn visit_literal_mut(&mut self, lit: &mut Literal) {
        if let Literal::Int(n) = lit {
            *n += 1;
        }
    }
}

#[test]
fn visit_mut_increment_integers() {
    let mut m = parse(
        "\
module Main exposing (..)

x = [ 1, 2, 3 ]
",
    )
    .unwrap();

    IntIncrementer.visit_module_mut(&mut m);

    let output = print::print(&m);
    assert!(output.contains("[ 2, 3, 4 ]"));
}

// ── Fold: replacing all string literals ──────────────────────────────

struct StringReplacer;

impl Fold for StringReplacer {
    fn fold_literal(&mut self, lit: Literal) -> Literal {
        match lit {
            Literal::String(_) => Literal::String("REDACTED".into()),
            other => other,
        }
    }
}

#[test]
fn fold_replace_strings() {
    let m = parse(
        r#"
module Main exposing (..)

x = "secret"

y = "also secret"
"#,
    )
    .unwrap();

    let mut folder = StringReplacer;
    let m2 = folder.fold_module(m);

    let output = print::print(&m2);
    assert!(output.contains("\"REDACTED\""));
    assert!(!output.contains("\"secret\""));
    assert!(!output.contains("\"also secret\""));
}

// ── Fold: rewriting identifiers ──────────────────────────────────────

struct IdentPrefixer(String);

impl Fold for IdentPrefixer {
    fn fold_ident(&mut self, name: String) -> String {
        format!("{}_{}", self.0, name)
    }
}

#[test]
fn fold_prefix_identifiers() {
    let m = parse(
        "\
module Main exposing (..)

add x y = x + y
",
    )
    .unwrap();

    let mut folder = IdentPrefixer("my".into());
    let m2 = folder.fold_module(m);

    let output = print::print(&m2);
    assert!(output.contains("my_add"));
    assert!(output.contains("my_x"));
    assert!(output.contains("my_y"));
}

// ── Fold: identity fold preserves structure ──────────────────────────

struct IdentityFold;
impl Fold for IdentityFold {}

#[test]
fn fold_identity_preserves_structure() {
    let src = r#"
module Main exposing (..)

type Msg = Increment | Decrement

type alias Model = Int

update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            model + 1
        Decrement ->
            model - 1
"#;

    let m = parse(src).unwrap();
    let original_output = print::print(&m);

    let mut folder = IdentityFold;
    let m2 = folder.fold_module(m);
    let folded_output = print::print(&m2);

    assert_eq!(original_output, folded_output);
}

// ── Fold: transforming expressions ───────────────────────────────────

struct NegationRemover;

impl Fold for NegationRemover {
    fn fold_expr(&mut self, expr: Spanned<Expr>) -> Spanned<Expr> {
        let expr = elm_ast_rs::fold::fold_expr(self, expr);
        match &expr.value {
            Expr::Negation(inner) => {
                // Replace -x with x
                Spanned::new(expr.span, inner.value.clone())
            }
            _ => expr,
        }
    }
}

#[test]
fn fold_remove_negation() {
    let m = parse(
        "\
module Main exposing (..)

x = -42
",
    )
    .unwrap();

    let mut folder = NegationRemover;
    let m2 = folder.fold_module(m);

    let output = print::print(&m2);
    // Should have 42 without negation.
    assert!(output.contains("42"));
    assert!(!output.contains("-42"));
}

// ── Combined: visit then mutate ──────────────────────────────────────

#[test]
fn visit_then_mutate() {
    let src = "\
module Main exposing (..)

add a b = a + b

sub a b = a - b
";
    let mut m = parse(src).unwrap();

    // First, count functions.
    let mut counter = DeclCounter(0);
    counter.visit_module(&m);
    assert_eq!(counter.0, 2);

    // Then rename 'a' to 'x'.
    let mut renamer = Renamer {
        from: "a".into(),
        to: "x".into(),
    };
    renamer.visit_module_mut(&mut m);

    let output = print::print(&m);
    assert!(output.contains("x + b"));
    assert!(output.contains("x - b"));
    assert!(!output.contains("a +"));
}
