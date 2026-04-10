use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::fs;

fn load_fixture(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| {
        // Return a minimal fallback if fixtures aren't cloned.
        "module Main exposing (..)\n\nx = 1\n".to_string()
    })
}

fn bench_parse(c: &mut Criterion) {
    let files = [
        ("small (Maybe.elm)", "test-fixtures/core/src/Maybe.elm"),
        ("medium (Dict.elm)", "test-fixtures/core/src/Dict.elm"),
        ("large (Css.elm)", "test-fixtures/elm-css/src/Css.elm"),
    ];

    let mut group = c.benchmark_group("parse");
    for (label, path) in &files {
        let source = load_fixture(path);
        let lines = source.lines().count();
        group.bench_with_input(BenchmarkId::new(*label, lines), &source, |b, src| {
            b.iter(|| elm_ast_rs::parse(black_box(src)).unwrap());
        });
    }
    group.finish();
}

fn bench_lex(c: &mut Criterion) {
    let files = [
        ("small (Maybe.elm)", "test-fixtures/core/src/Maybe.elm"),
        ("medium (Dict.elm)", "test-fixtures/core/src/Dict.elm"),
        ("large (Css.elm)", "test-fixtures/elm-css/src/Css.elm"),
    ];

    let mut group = c.benchmark_group("lex");
    for (label, path) in &files {
        let source = load_fixture(path);
        let lines = source.lines().count();
        group.bench_with_input(BenchmarkId::new(*label, lines), &source, |b, src| {
            b.iter(|| elm_ast_rs::Lexer::new(black_box(src)).tokenize());
        });
    }
    group.finish();
}

fn bench_print(c: &mut Criterion) {
    let files = [
        ("small (Maybe.elm)", "test-fixtures/core/src/Maybe.elm"),
        ("medium (Dict.elm)", "test-fixtures/core/src/Dict.elm"),
        ("large (Css.elm)", "test-fixtures/elm-css/src/Css.elm"),
    ];

    let mut group = c.benchmark_group("print");
    for (label, path) in &files {
        let source = load_fixture(path);
        let module = elm_ast_rs::parse(&source).unwrap();
        let lines = source.lines().count();
        group.bench_with_input(BenchmarkId::new(*label, lines), &module, |b, m| {
            b.iter(|| elm_ast_rs::print(black_box(m)));
        });
    }
    group.finish();
}

fn bench_round_trip(c: &mut Criterion) {
    let files = [
        ("small (Maybe.elm)", "test-fixtures/core/src/Maybe.elm"),
        ("medium (Dict.elm)", "test-fixtures/core/src/Dict.elm"),
        ("large (Css.elm)", "test-fixtures/elm-css/src/Css.elm"),
    ];

    let mut group = c.benchmark_group("round_trip");
    for (label, path) in &files {
        let source = load_fixture(path);
        let lines = source.lines().count();
        group.bench_with_input(BenchmarkId::new(*label, lines), &source, |b, src| {
            b.iter(|| {
                let m = elm_ast_rs::parse(black_box(src)).unwrap();
                let _ = elm_ast_rs::print(&m);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_lex,
    bench_parse,
    bench_print,
    bench_round_trip
);
criterion_main!(benches);
