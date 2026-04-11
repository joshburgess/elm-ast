# elm-assist

A plan for expanding elm-lint into a fast, single-binary replacement for elm-review with built-in rules only. No plugin system, no Node.js dependency, no Elm runtime.

## Motivation

elm-review runs an Elm program inside Node.js to analyze Elm code. This works but is slow (cold start, GC pauses, IPC overhead) and requires a Node.js installation. A native Rust binary using elm-ast can be 10-50x faster and ship as a single executable with zero dependencies.

The goal is not to replicate elm-review's plugin architecture. Instead, ship a comprehensive set of built-in rules covering the most commonly used elm-review rule packages. This delivers ~90% of the value for ~20% of the effort.

## What we have today

- **elm-ast**: 100% parse/round-trip/idempotency on 291 real-world files from 50 packages
- **elm-lint**: 14 built-in rules, `Rule` trait, `LintContext`, `LintError` with optional fix suggestions
- **elm-unused**: cross-module dead code analysis (unused imports, functions, exports, constructors, types)
- **Visit/VisitMut/Fold traits**: full AST traversal infrastructure
- **Pretty printer**: idempotent elm-format-style output for auto-fix rewrites

## Architecture

### Rule trait (existing, needs extension)

```rust
pub trait Rule {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn check(&self, ctx: &LintContext) -> Vec<LintError>;
}
```

### Extensions needed

1. **Project-level rules**: some rules need cross-module information (e.g., unused exports, missing module docs). Add a `ProjectRule` trait or extend `LintContext` with project-wide data.

2. **Auto-fix support**: `LintError` already has an `Option<String>` fix field. Extend this to a structured `Fix` type:
   ```rust
   pub struct Fix {
       pub edits: Vec<Edit>,
   }

   pub enum Edit {
       Replace { span: Span, replacement: String },
       InsertAfter { span: Span, text: String },
       Remove { span: Span },
   }
   ```

3. **Rule configuration**: some rules need parameters (e.g., max line length, forbidden modules). Add an optional config method:
   ```rust
   fn configure(&mut self, config: &toml::Value) -> Result<(), String> { Ok(()) }
   ```

4. **Severity levels**: allow rules to report warnings vs errors:
   ```rust
   pub enum Severity { Error, Warning }
   ```

### CLI design

```
elm-assist [options] [src-directory]

Options:
  --fix              Apply auto-fixes
  --fix-all          Apply all auto-fixes without prompting
  --watch            Re-run on file changes
  --rules <list>     Enable only specific rules (comma-separated)
  --disable <list>   Disable specific rules
  --config <path>    Path to elm-assist.toml config file
  --json             Output findings as JSON
  --color            Force colored output (auto-detected by default)
  --no-color         Disable colored output
```

### Config file (elm-assist.toml)

```toml
[rules]
# Disable specific rules
disable = ["NoMissingTypeAnnotation"]

# Rule-specific configuration
[rules.NoUnusedExports]
ignore_modules = ["Main", "Ports"]

[rules.NoMaxLineLength]
max_length = 120
```

### Caching

Hash each file's contents. On re-run, skip modules whose hash hasn't changed and whose rule set is the same. Store cache in `.elm-assist-cache/` or similar. Invalidate project-level rule caches when any file changes.

### Watch mode

Use the `notify` crate to watch the source directory. On file change, re-parse only changed files, invalidate caches, re-run rules, and report incrementally.

## Rules to implement

Organized by the elm-review packages they replace. Rules marked with (fix) support auto-fix.

### From jfmengels/elm-review-unused (elm-unused already covers most of these)

- [x] **NoUnusedImports** (fix) — import statement where nothing from the module is used
- [x] **NoUnusedVariables** — defined but never referenced
- [x] **NoUnusedExports** — exported but never imported by any other module in the project
- [x] **NoUnusedCustomTypeConstructors** — constructor never used in patterns or expressions
- [x] **NoUnusedCustomTypeConstructorArgs** — constructor argument that is always ignored with `_`
- [x] **NoUnusedModules** — module that is never imported by any other module
- [x] **NoUnusedParameters** (fix) — function parameter always matched with `_`

### From jfmengels/elm-review-simplify

- [x] **NoIfTrueFalse** (fix) — `if x then True else False` -> `x`
- [x] **NoBooleanCase** (fix) — `case x of True -> ... ; False -> ...` -> `if x then ... else ...`
- [x] **NoAlwaysIdentity** (fix) — `always identity` -> `\_ -> identity` or simplified
- [x] **NoRedundantCons** (fix) — `x :: []` -> `[x]`
- [x] **NoUnnecessaryParens** (fix) — `(x)` -> `x` when parens aren't needed
- [x] **NoNegationOfBooleanOperator** (fix) — `not (a == b)` -> `a /= b`
- [x] **NoFullyAppliedPrefixOperator** (fix) — `(+) 1 2` -> `1 + 2`
- [x] **NoIdentityFunction** (fix) — `\x -> x` passed as argument -> `identity`
- [x] **NoListLiteralConcat** (fix) — `[a] ++ [b]` -> `[a, b]`
- [x] **NoEmptyListConcat** (fix) — `[] ++ list` -> `list`
- [x] **NoStringConcat** (fix) — `"a" ++ "b"` -> `"ab"` for string literals
- [x] **NoBoolOperatorSimplify** (fix) — `x && True` -> `x`, `x || False` -> `x`
- [x] **NoMaybeMapWithNothing** (fix) — `Maybe.map f Nothing` -> `Nothing`
- [x] **NoResultMapWithErr** (fix) — `Result.map f (Err e)` -> `Err e`
- [x] **NoPipelineSimplify** (fix) — `x |> identity` -> `x`

### From jfmengels/elm-review-debug

- [x] **NoDebug** (fix) — `Debug.log`, `Debug.todo`, `Debug.toString`

### From jfmengels/elm-review-common

- [x] **NoMissingTypeAnnotation** — top-level function without a type signature
- [x] **NoSinglePatternCase** (fix) — `case x of _ -> ...` -> `let _ = x in ...`
- [x] **NoExposingAll** (fix) — `module Foo exposing (..)` -> explicit exposing list
- [x] **NoImportExposingAll** (fix) — `import Foo exposing (..)` -> explicit exposing
- [x] **NoDeprecated** — usage of functions/types marked as deprecated
- [x] **NoMissingDocumentation** — public function/type without a doc comment

### From jfmengels/elm-review-code-style

- [x] **NoUnnecessaryTrailingUnderscore** — `foo_` when `foo` is not in scope
- [x] **NoPrematureLetComputation** — let binding used only in one branch of if/case
- [x] **NoSimpleLetBody** (fix) — `let x = expr in x` -> `expr`
- [x] **NoUnnecessaryPortModule** (fix) — `port module` with no port declarations

### New rules (no elm-review equivalent)

- [x] **NoEmptyLet** — `let in expr` with no bindings
- [x] **NoEmptyRecordUpdate** — `{ record | }` with no fields
- [x] **NoNestedNegation** — `not (not x)` -> `x`
- [x] **NoWildcardPatternLast** — catch-all `_` that shadows more specific patterns
- [x] **NoMaxLineLength** — configurable line length limit
- [x] **NoTodoComment** — `-- TODO` or `{- TODO -}` in source
- [x] **NoRecordPatternInFunctionArgs** — `foo { x, y } = ...` -> `foo record = ... record.x ... record.y`
- [x] **NoUnusedLetBinding** — let binding that is never referenced in the body
- [x] **NoShadowing** — local binding that shadows an outer name

### Batch 2: Popular elm-review rules

- [x] **NoUnusedPatterns** — case branch pattern variable that is never referenced in the branch body
- [x] **CognitiveComplexity** — function exceeds configurable complexity threshold (default 15)
- [x] **NoMissingTypeAnnotationInLetIn** — let-in function binding without a type annotation
- [x] **NoConfusingPrefixOperator** — non-commutative operator used in prefix form, e.g. `(-) a b`
- [x] **NoMissingTypeExpose** — type referenced in a public function signature but not exposed from the module
- [x] **NoRedundantlyQualifiedType** (fix) — `Set.Set` -> `Set` when type name matches module name
- [x] **NoUnoptimizedRecursion** — recursive function where not all recursive calls are in tail position
- [x] **NoRecursiveUpdate** — `update` function calling itself recursively

## Implementation phases

### Phase 1: Consolidate existing tools

Merge elm-unused analysis into elm-lint's rule system as project-level rules. This eliminates the separate elm-unused binary and gives users one tool.

- Add `ProjectRule` trait or extend `LintContext` with cross-module info
- Port elm-unused's `collect_module_info` + `analyze` into project-level lint rules
- Existing elm-unused unit tests become lint rule tests

### Phase 2: Expand rule set to ~30 rules

Implement the most impactful rules from the list above. Priority order:
1. Rules with auto-fixes (highest user value)
2. Rules from elm-review-simplify (most commonly requested)
3. Rules from elm-review-unused (already partially implemented)
4. Rules from elm-review-common and elm-review-code-style

### Phase 3: Auto-fix infrastructure

- Implement `Fix` type with `Edit` variants
- Apply fixes to source text (not AST) to preserve formatting
- `--fix` mode: show each fix, prompt for confirmation
- `--fix-all` mode: apply all fixes without prompting
- Verify fixes by re-parsing the modified source

### Phase 4: CLI and developer experience

- Proper CLI with `clap`
- Colored terminal output with source context (like elm compiler errors)
- `elm-assist.toml` config file
- `--json` output for editor integration
- Exit codes: 0 = no findings, 1 = findings, 2 = error

### Phase 5: Performance

- File hashing and caching
- Watch mode with `notify`
- Parallel rule execution (rules are independent per-file)
- Parallel file parsing with `rayon`

### Phase 6: LSP and editor integration

The biggest advantage over elm-review: real-time lint diagnostics in the editor with click-to-fix code actions. elm-review has no editor integration — you run it in the terminal and read output. An LSP makes the tool feel native.

#### LSP server

A single `elm-assist-lsp` binary (or `elm-assist --lsp` flag) that speaks the Language Server Protocol. Built on `tower-lsp` or `lsp-server`.

Core loop:
1. Client opens/changes a file -> LSP receives `textDocument/didOpen` or `textDocument/didChange`
2. Re-parse the changed file (our parser is fast enough for keystroke-level latency)
3. Run all enabled rules on the changed module (and project-level rules if needed)
4. Publish diagnostics via `textDocument/publishDiagnostics`

LSP capabilities to implement:

| Capability | Maps to | Notes |
|---|---|---|
| `textDocument/publishDiagnostics` | `LintError` -> `Diagnostic` | Squiggly underlines with rule name and message |
| `textDocument/codeAction` | `Fix` -> `CodeAction` | Click-to-fix in the editor lightbulb menu |
| `workspace/executeCommand` | Fix-all, disable rule | Batch operations |
| `textDocument/hover` | Rule description | Show rule docs on hover over diagnostic |
| `workspace/didChangeConfiguration` | `elm-assist.toml` reload | Live config changes without restart |

The LSP and CLI share all parsing, rule, and fix logic. The LSP is just a different frontend to the same rule engine — it receives file contents from editor buffers instead of reading from disk, and reports via the LSP protocol instead of terminal output.

#### Incremental analysis

- Track which files are open and their in-memory contents (LSP text sync)
- On change, only re-parse and re-lint the changed file
- For project-level rules (e.g., unused exports), maintain a cached project index and update it incrementally when a file changes
- Debounce rapid keystrokes (e.g., 100ms delay before re-analyzing)

#### VS Code extension

A thin TypeScript extension that:
- Bundles or locates the `elm-assist-lsp` binary
- Spawns it as a language server child process
- Provides configuration UI in VS Code settings (enable/disable rules, set severity)
- Registers the server for `elm` language files

This is a minimal wrapper — all intelligence lives in the Rust LSP binary.

#### Other editors

Because it's a standard LSP server, it works out of the box with:
- **Neovim**: `nvim-lspconfig` entry
- **Emacs**: `lsp-mode` or `eglot` configuration
- **Helix**: `languages.toml` entry
- **Zed**: extension or built-in LSP support
- **Sublime Text**: LSP package configuration

No editor-specific code needed beyond VS Code (which gets a dedicated extension for discoverability/configuration).

## Non-goals

- **Plugin system**: no dynamic loading, no WASM rules, no scripting. All rules are built into the binary. This can be revisited later if demand exists.
- **elm-review config compatibility**: we use `elm-assist.toml`, not `ReviewConfig.elm`. Migration guide can be provided.
- **100% rule parity**: some niche elm-review rules won't be replicated. Focus on the most commonly used rules.

## Success criteria

- Single binary, no runtime dependencies
- Sub-second analysis on projects with 100+ modules
- 50 rules covering the most popular elm-review packages
- Auto-fix for 23 rules
- Drop-in usable for most Elm projects without configuration
- LSP server with real-time diagnostics and code actions
- VS Code extension published on the marketplace
