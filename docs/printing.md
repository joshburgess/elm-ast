# Printing

`elm-ast` ships with a pretty-printer inspired by [elm-format](https://github.com/avh4/elm-format)'s "Box" model: each expression eagerly decides whether it would produce multi-line output, and containers (lists, tuples, applications, operator chains) switch to vertical layout when any child is multi-line. See [ARCHITECTURE.md](../ARCHITECTURE.md#printer) for the layout machinery; this document covers the three modes that drive *how often* those breaks happen, and how to pick between them.

## The three modes

Every printer call goes through `Printer::print_module`, which reads a `PrintStyle` off the `PrintConfig`:

```rust
pub enum PrintStyle {
    Compact,              // default
    ElmFormat,
    ElmFormatConverged,
}
```

The modes sit on a spectrum from "minimal changes to the source" (`Compact`) to "maximal agreement with elm-format" (`ElmFormat` / `ElmFormatConverged`).

### `Compact` — round-trip-safe minimal breaking

- **Convenience fn**: `print(&module)`. Also what `Display` uses.
- **What it breaks**: only expressions that are *inherently* multi-line. `case`, `if`, `let`, and lambdas with multi-line bodies produce multiple lines. Everything else stays as compact as possible.
- **What it preserves**: single-line pipelines stay single-line; short lists and records stay single-line; if-else stays on one line when both branches are atomic.
- **Guarantee**: `print(parse(print(parse(src)))) == print(parse(src))` (full round-trip idempotency) on all elm-format-compliant input. Verified across 291 integration-test fixtures.
- **When to use**: round-tripping ASTs, codegen where you want compact output, or storing a canonical form that's stable under re-parse.

### `ElmFormat` — match elm-format byte-for-byte

- **Convenience fn**: `pretty_print(&module)`.
- **What it breaks**: everything elm-format breaks. Pipelines (`|>`, `|.`, `|=`) are always vertical. Records and lists with 2+ entries are always multi-line. `if`-`else` is always multi-line. Type annotations break the same way elm-format breaks them.
- **Span-based preservation** (since 0.1.5): when consecutive items in a container sit on different source lines, the container stays multi-line even when the children are individually single-line. This mirrors elm-format's behavior of preserving source-visible line breaks in containers and pipeline chains.
- **Goal**: `pretty_print(source) == elm-format(source)` on real-world packages.
- **When to use**: you want elm-format's output but without shelling out to a Haskell binary, or you're integrating with tooling that expects elm-format formatting.

### `ElmFormatConverged` — elm-format style, pre-converged to a stable form

- **Convenience fn**: `pretty_print_converged(&module)`.
- **What it does**: everything `ElmFormat` does, plus it pre-applies the mutations elm-format would make on a *second* pass over its own output.

#### Why this mode exists

elm-format is not fully idempotent. On one specific shape, a second pass changes the output:

- A line comment (`-- …`)
- followed by a blank line
- followed by an `import` statement,
- appearing inside a doc-comment code block (`{-| … -}`).

On that shape, elm-format's first pass keeps 1 blank line. A second pass inserts a second blank line. Running `elm-format` twice on such input produces different output than running it once; a third pass produces the same output as the second, and so on.

`ElmFormatConverged` skips straight to the 2-blank form that elm-format would settle on after repeated passes.

#### What "converged" means in practice

- `pretty_print_converged(src) == elm-format(pretty_print_converged(src))` — re-running elm-format over the output is a no-op.
- On every input *except* the 1-blank doc-comment shape, the output is identical to `ElmFormat`.
- On the 1-blank shape, the output differs from `elm-format(source)` on the *first* pass (agreeing with the second pass instead).

#### When to use it

Use `ElmFormatConverged` when downstream tooling re-runs elm-format on your output (CI formatters, editor integrations, git hooks) and you need the re-format to be a no-op. Prefer `ElmFormat` when you need byte-for-byte agreement with a single `elm-format` invocation on the original source.

## Idempotency guarantees at a glance

| Mode | `print(parse(print(parse(x)))) == print(parse(x))` | `elm-format(print(x)) == print(x)` |
|---|---|---|
| `Compact` | yes | no (different formatting style) |
| `ElmFormat` | yes | yes, except on the 1-blank doc-comment shape |
| `ElmFormatConverged` | yes | yes, on all input |

## Configuration

The convenience functions cover the common cases. For custom indent width, or to configure the mode without using the convenience re-exports:

```rust
use elm_ast::print::{PrintConfig, PrintStyle, Printer};

let output = Printer::new(PrintConfig {
    indent_width: 2,
    style: PrintStyle::ElmFormatConverged,
}).print_module(&module);
```

`PrintConfig::default()` uses `indent_width: 4` and `PrintStyle::Compact`.

## Printing nodes other than a module

`Printer` exposes `write_expr`, `write_type`, `write_pattern`, `write_declaration`, and `write_spanned_expr`. Call one or more of these, then `finish()` to get the buffer:

```rust
use elm_ast::print::{PrintConfig, Printer};

let mut p = Printer::new(PrintConfig::default());
p.write_expr(&expr);
let s = p.finish();
```

The configured `PrintStyle` applies to line-breaking decisions.

**Caveat for `ElmFormatConverged`**: the convergence mutations (the extra blank line inside doc-comment code blocks) are applied only by `print_module`, not by the per-node `write_*` methods. If you call `write_declaration` directly on a `Printer` configured with `ElmFormatConverged`, line breaking still follows `ElmFormat`, but doc-comment code blocks will not be pre-converged. For full converged output, go through `print_module` (or the `pretty_print_converged` convenience function).
