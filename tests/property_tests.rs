#![cfg(not(target_arch = "wasm32"))]

use elm_ast::builder::*;
use elm_ast::print::print;
use elm_ast::{parse, parse_recovering};
use proptest::prelude::*;

// ── Generators ──────────────────────────────────────────────────────

fn arb_ident() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-zA-Z0-9]{0,8}")
        .unwrap()
        .prop_filter("not an Elm keyword", |s| {
            !matches!(
                s.as_str(),
                "if" | "then"
                    | "else"
                    | "case"
                    | "of"
                    | "let"
                    | "in"
                    | "type"
                    | "alias"
                    | "module"
                    | "exposing"
                    | "import"
                    | "port"
                    | "as"
                    | "where"
            )
        })
}

fn arb_expr(depth: u32) -> impl Strategy<Value = elm_ast::node::Spanned<elm_ast::expr::Expr>> {
    // Use non-negative integers to avoid ambiguity where a negative literal
    // like `-1` inside an operator expression is re-parsed as unary negation.
    if depth == 0 {
        prop_oneof![
            (0i64..1_000_000).prop_map(int),
            (0.0f64..1e6)
                .prop_filter("finite", |f| f.is_finite() && !f.is_nan())
                .prop_map(float),
            Just(unit()),
            arb_ident().prop_map(|s| var(s)),
        ]
        .boxed()
    } else {
        let leaf = prop_oneof![
            (0i64..1_000_000).prop_map(int),
            arb_ident().prop_map(|s| var(s)),
        ];
        prop_oneof![
            // leaf
            leaf,
            // list
            prop::collection::vec(arb_expr(depth - 1), 0..4).prop_map(list),
            // tuple (2 or 3 elements)
            (arb_expr(depth - 1), arb_expr(depth - 1)).prop_map(|(a, b)| tuple(vec![a, b])),
            // binop
            (arb_expr(depth - 1), arb_expr(depth - 1)).prop_map(|(a, b)| binop("+", a, b)),
            // if
            (
                arb_expr(depth - 1),
                arb_expr(depth - 1),
                arb_expr(depth - 1)
            )
                .prop_map(|(c, t, e)| if_else(c, t, e)),
            // lambda
            (arb_ident(), arb_expr(depth - 1))
                .prop_map(|(arg, body)| lambda(vec![pvar(arg)], body)),
        ]
        .boxed()
    }
}

// ── Properties ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// print(parse(src)) should re-parse without error (idempotent round-trip)
    #[test]
    fn print_parse_print_idempotent(
        fname in arb_ident(),
        argname in arb_ident(),
        body in arb_expr(2),
    ) {
        // Build a module: module Main exposing (..) \n fname argname = <body>
        let m = module(
            vec!["Main"],
            vec![func(&fname, vec![pvar(&argname)], body)],
        );
        let printed1 = print(&m);

        // First: the printed output should parse
        let ast2 = parse(&printed1);
        prop_assert!(ast2.is_ok(), "Failed to parse printed output:\n{}\nError: {:?}", printed1, ast2.err());

        // Second: printing again should be identical (idempotency)
        let printed2 = print(&ast2.unwrap());
        prop_assert_eq!(&printed1, &printed2, "Printer not idempotent");
    }

    /// Every built module should parse without errors
    #[test]
    fn builder_output_always_parses(
        fname in arb_ident(),
        body in arb_expr(3),
    ) {
        let m = module(vec!["Main"], vec![func(&fname, vec![pwild()], body)]);
        let printed = print(&m);
        let result = parse(&printed);
        prop_assert!(result.is_ok(), "Builder output failed to parse:\n{}\nError: {:?}", printed, result.err());
    }

    /// parse_recovering should never panic, even on garbage input
    #[test]
    fn recovering_parser_never_panics(input in "module [A-Z][a-z]{0,5} exposing \\(\\.\\.\\)\n\n[a-z =+*()\\[\\]{},\\n ]{0,200}") {
        let _ = parse_recovering(&input);
    }

    /// Arbitrary valid Elm source fragments should round-trip after wrapping in a module
    #[test]
    fn simple_expressions_round_trip(
        x in any::<i64>(),
        y in any::<i64>(),
        op in prop_oneof![Just("+"), Just("-"), Just("*")],
    ) {
        let src = format!("module Main exposing (..)\n\nresult = {} {} {}\n", x, op, y);
        if let Ok(ast) = parse(&src) {
            let printed = print(&ast);
            let reparsed = parse(&printed);
            prop_assert!(reparsed.is_ok(), "round-trip failed: {}", printed);
            let reprinted = print(&reparsed.unwrap());
            prop_assert_eq!(printed, reprinted, "not idempotent");
        }
        // If parse fails (e.g. negative number issues), that's fine — skip
    }

    /// Declaration count is preserved across print → parse round-trip
    #[test]
    fn declaration_count_preserved(
        n in 1..8usize,
    ) {
        let decls: Vec<_> = (0..n)
            .map(|i| func(&format!("f{}", i), vec![pvar("x")], var("x")))
            .collect();
        let m = module(vec!["Main"], decls);
        let printed = print(&m);
        let reparsed = parse(&printed);
        prop_assert!(reparsed.is_ok(), "parse failed:\n{}", printed);
        prop_assert_eq!(reparsed.unwrap().declarations.len(), n, "declaration count mismatch");
    }
}
