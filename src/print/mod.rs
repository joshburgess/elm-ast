//! Elm pretty-printer.
//!
//! The formatting approach is inspired by [elm-format](https://github.com/avh4/elm-format)'s
//! "Box" model. The core idea:
//!
//! 1. **`is_multiline(expr)`** — eagerly check if an expression would produce
//!    multi-line output (block expressions like `case`/`if`/`let`/`lambda` are
//!    always multi-line; containers are multi-line if any child is).
//!
//! 2. **Containers adapt** — lists, tuples, applications, and operator chains
//!    switch to one-element-per-line formatting when any element is multi-line.
//!
//! 3. **Block expressions get parens in atomic position** — when a `case`/`if`/`let`
//!    appears as a function argument or infix operand, it's parenthesized.
//!    Multi-line parens put the closing `)` on its own line so the re-parser
//!    can find it.
//!
//! This produces idempotent output: `print(parse(print(parse(src)))) == print(parse(src))`.

mod block_comment;
mod comment_slots;
mod decl_emission;
mod doc_markdown;
mod import_layout;
mod pipeline_layout;

use block_comment::reindent_block_comment;
use comment_slots::CommentSlots;
use doc_markdown::*;
use import_layout::build_import_plan;
use pipeline_layout::*;

use crate::comment::Comment;
use crate::declaration::{CustomType, Declaration, InfixDef, TypeAlias, ValueConstructor};
use crate::exposing::{ExposedItem, Exposing};
use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, LetDeclaration, RecordSetter, Signature,
};
use crate::file::ElmModule;
use crate::import::Import;
use crate::literal::Literal;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::pattern::Pattern;
use crate::span::Span;
use crate::type_annotation::{RecordField, TypeAnnotation};

/// Controls how aggressively the printer breaks lines.
///
/// - `Compact` (default): only break lines for structurally multi-line
///   sub-expressions (case/if/let/lambda). This is the round-trip-safe mode —
///   `print(parse(print(parse(src)))) == print(parse(src))` holds for all
///   elm-format-compliant source files.
///
/// - `ElmFormat`: break lines in the same places elm-format does — pipelines
///   always vertical, records and lists with 2+ entries always multiline.
///   Designed for **code generation** where the AST is built from scratch and
///   readability matters more than exact round-tripping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PrintStyle {
    /// Round-trip-safe minimal line breaking.
    #[default]
    Compact,
    /// elm-format-style pretty printing.
    ElmFormat,
}

/// Configuration for the Elm printer.
#[derive(Clone, Debug)]
pub struct PrintConfig {
    /// Number of spaces per indentation level.
    pub indent_width: usize,
    /// Line-breaking strategy.
    pub style: PrintStyle,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self {
            indent_width: 4,
            style: PrintStyle::default(),
        }
    }
}

/// Pretty-printer for Elm AST nodes.
pub struct Printer {
    config: PrintConfig,
    buf: String,
    indent: usize,
    /// Extra spaces added to newline_indent at the current indent level only.
    /// Used to align `else`/`in` inside `(` by 1 space when a block expression
    /// is parenthesized. Cleared on indent(), restored on dedent().
    indent_extra: u32,
    /// Stack of saved indent_extra values, pushed by indent(), popped by dedent().
    indent_extra_stack: Vec<u32>,
    /// When true, the enclosing vertical operator chain has already bumped
    /// indent for its operators. Nested binop layouts should align at this
    /// same column rather than bumping another level, matching elm-format's
    /// rule that every operator in a multi-line chain sits at one column
    /// regardless of precedence or parse-tree grouping. Cleared whenever an
    /// operand crosses a paren, application, or block boundary.
    in_vertical_chain: bool,
    /// Groups of exposed names parsed from `@docs` directives in the module doc.
    /// Each inner Vec is one `@docs` line. Used by `write_exposing_pretty` to
    /// match elm-format's grouping of exposing items.
    doc_groups: Vec<Vec<String>>,
    /// Stack of the current expression's source span. Pushed/popped by
    /// `write_spanned_expr`. Used by List/Tuple/etc. to detect when the
    /// outer brackets spanned multiple source lines even if the inner
    /// elements are all single-line.
    expr_span_stack: Vec<Span>,
}

impl Printer {
    pub fn new(config: PrintConfig) -> Self {
        Self {
            config,
            buf: String::new(),
            indent: 0,
            indent_extra: 0,
            indent_extra_stack: Vec::new(),
            in_vertical_chain: false,
            doc_groups: Vec::new(),
            expr_span_stack: Vec::new(),
        }
    }

    /// Print a complete Elm module to a string.
    pub fn print_module(mut self, module: &ElmModule) -> String {
        self.write_module(module);
        self.buf
    }

    /// Print any single node to a string using `write_*` methods.
    pub fn finish(self) -> String {
        self.buf
    }

    fn is_pretty(&self) -> bool {
        self.config.style == PrintStyle::ElmFormat
    }

    /// True when the two spans sit on different source lines (both non-dummy).
    /// Used by container layout decisions so that multi-line source stays
    /// multi-line in pretty output — mirroring elm-format's "preserve source
    /// layout" behavior. Also lets synthesized ASTs (codegen) opt into
    /// multi-line layout by setting operand spans on different lines.
    fn spans_cross_lines(a: Span, b: Span) -> bool {
        a.start.line != 0 && b.end.line != 0 && b.end.line > a.start.line
    }

    /// True when a single span covers multiple source lines (non-dummy).
    fn span_crosses_lines(s: Span) -> bool {
        s.start.line != 0 && s.end.line > s.start.line
    }

    /// True when a sequence of spanned nodes spans multiple source lines.
    fn spans_multi_lines<T>(items: &[Spanned<T>]) -> bool {
        match (items.first(), items.last()) {
            (Some(first), Some(last)) => Self::spans_cross_lines(first.span, last.span),
            _ => false,
        }
    }

    /// Check if an expression will produce multi-line output.
    ///
    /// In ElmFormat mode, `if-else` is always multi-line (the printer never
    /// uses single-line if). This makes parent containers (Application,
    /// OperatorApplication, etc.) aware that their child will be multi-line,
    /// so they can choose vertical layout accordingly.
    fn is_multiline(&self, expr: &Expr) -> bool {
        match expr {
            Expr::IfElse {
                branches,
                else_branch,
                ..
            } => {
                // In ElmFormat mode, if-else is always multiline.
                if self.is_pretty() {
                    return true;
                }
                // In Compact mode, single-line when simple.
                if branches.len() == 1 {
                    let (c, b) = &branches[0];
                    if !self.is_multiline(&c.value)
                        && !self.is_multiline(&b.value)
                        && !self.is_multiline(&else_branch.value)
                    {
                        return false;
                    }
                }
                true
            }
            Expr::CaseOf { .. } | Expr::LetIn { .. } => true,
            Expr::Lambda { args, body } => {
                self.is_multiline(&body.value)
                    || (self.is_pretty() && Self::span_crosses_lines(body.span))
                    || (self.is_pretty()
                        && match args.last() {
                            Some(last) => {
                                last.span.end.line != 0
                                    && body.span.start.line != 0
                                    && body.span.start.line > last.span.end.line
                            }
                            None => false,
                        })
            }
            Expr::Application(args) => {
                args.iter().any(|a| self.is_multiline(&a.value))
                    || (self.is_pretty() && Self::spans_multi_lines(args))
            }
            Expr::List(elems) => {
                elems.iter().any(|e| self.is_multiline(&e.value))
                    || (self.is_pretty() && Self::spans_multi_lines(elems))
            }
            Expr::Tuple(elems) => {
                elems.iter().any(|e| self.is_multiline(&e.value))
                    || (self.is_pretty() && Self::spans_multi_lines(elems))
            }
            Expr::Record(fields) => {
                fields
                    .iter()
                    .any(|f| self.is_multiline(&f.value.value.value))
                    || (self.is_pretty() && Self::spans_multi_lines(fields))
            }
            Expr::RecordUpdate { updates, .. } => {
                updates
                    .iter()
                    .any(|f| self.is_multiline(&f.value.value.value))
                    || (self.is_pretty() && Self::spans_multi_lines(updates))
            }
            Expr::OperatorApplication { left, right, .. } => {
                self.is_multiline(&left.value)
                    || self.is_multiline(&right.value)
                    || (self.is_pretty() && Self::spans_cross_lines(left.span, right.span))
            }
            Expr::Parenthesized(inner) => self.is_multiline(&inner.value),
            Expr::Negation(inner) => self.is_multiline(&inner.value),
            // A `"""..."""` literal whose content spans multiple lines prints
            // with its closing `"""` at column 1. In a binary-op chain this
            // breaks Elm's layout (parser loses indentation context), so we
            // report it as multi-line and let the chain layout handle it.
            Expr::Literal(Literal::MultilineString(s)) if s.contains('\n') => true,
            _ => false,
        }
    }

    // ── Output helpers ───────────────────────────────────────────────

    fn write(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    fn write_char(&mut self, c: char) {
        self.buf.push(c);
    }

    fn newline(&mut self) {
        self.buf.push('\n');
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent * self.config.indent_width + self.indent_extra as usize {
            self.buf.push(' ');
        }
    }

    fn indent(&mut self) {
        self.indent += 1;
        // Save and clear indent_extra — it only applies at the base level.
        self.indent_extra_stack.push(self.indent_extra);
        self.indent_extra = 0;
    }

    fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
        // Restore indent_extra from the previous level.
        self.indent_extra = self.indent_extra_stack.pop().unwrap_or(0);
    }

    /// Returns the column position (0-based) of the cursor on the current line.
    fn current_column(&self) -> usize {
        match self.buf.rfind('\n') {
            Some(pos) => self.buf.len() - pos - 1,
            None => self.buf.len(),
        }
    }

    fn newline_indent(&mut self) {
        self.newline();
        self.write_indent();
    }

    // ── Module ───────────────────────────────────────────────────────

    pub fn write_module(&mut self, module: &ElmModule) {
        // Parse @docs groups from module documentation for exposing list layout.
        if self.is_pretty()
            && let Some(doc) = &module.module_documentation
        {
            self.doc_groups = parse_docs_groups(&doc.value);
        }

        // Assign comments to slots (one per anchor + one trailing) and hoist
        // trailing import line-comments onto the preceding import in pretty
        // mode. Compact mode keeps internal comments for round-trip fidelity.
        let mut slots = CommentSlots::build(module, self.is_pretty());
        if self.is_pretty() {
            slots.hoist_import_trailing_comments(&module.imports);
        }
        let num_imports = slots.num_imports;
        let total_anchors = slots.trailing_slot();

        self.write_module_header(&module.header.value);
        self.newline();

        // Module-level documentation comment.
        if let Some(doc) = &module.module_documentation {
            self.newline();
            self.write_doc_comment_text(&doc.value);
            self.newline();
        }

        if !module.imports.is_empty() {
            self.newline();
            if self.is_pretty() {
                // ElmFormat mode: emit imports sorted and merged by module
                // name. Leading comments for each import in a group are
                // flushed before the import line, separated by a blank line.
                for group in build_import_plan(&module.imports) {
                    let mut had_comments = false;
                    for &idx in &group.src_indices {
                        if !slots.slots[idx].is_empty() {
                            had_comments = true;
                            for c in &slots.slots[idx] {
                                self.write_comment(&c.value);
                                self.newline();
                            }
                        }
                    }
                    if had_comments {
                        self.newline();
                    }
                    if group.src_indices.len() == 1 {
                        self.write_import(&module.imports[group.src_indices[0]].value);
                    } else {
                        self.write_merged_imports(
                            &group
                                .src_indices
                                .iter()
                                .map(|&idx| &module.imports[idx].value)
                                .collect::<Vec<_>>(),
                        );
                    }
                    self.newline();
                }
            } else {
                for (i, imp) in module.imports.iter().enumerate() {
                    if !slots.slots[i].is_empty() {
                        for c in &slots.slots[i] {
                            self.write_comment(&c.value);
                            self.newline();
                        }
                    }
                    self.write_import(&imp.value);
                    self.newline();
                }
            }
        }

        self.write_declarations_with_comments(module, &slots, num_imports);
        self.write_trailing_orphan_comments(module, &slots, total_anchors);

        self.newline();
    }

    fn write_comment(&mut self, comment: &Comment) {
        match comment {
            Comment::Line(text) => {
                self.write("--");
                self.write(text);
            }
            Comment::Block(text) => {
                if self.is_pretty() && text.contains('\n') {
                    let brace_col = self.current_column();
                    self.write("{-");
                    // elm-format does NOT re-indent contents of "comment-out"
                    // style block comments `{-- ... -}` (text starts with a
                    // literal `-`). For normal `{- ... -}`, it aligns
                    // continuation lines so the min indent sits at
                    // `{-col + 3`.
                    let is_double_dash = text.starts_with('-');
                    let reindented = if is_double_dash {
                        text.to_string()
                    } else {
                        reindent_block_comment(text, brace_col)
                    };
                    let reindented = if self.is_pretty() && reindented.starts_with("- ") {
                        format!("-{}", &reindented[2..])
                    } else {
                        reindented
                    };
                    self.write(&reindented);
                    self.write("-}");
                } else {
                    self.write("{-");
                    self.write(text);
                    self.write("-}");
                }
            }
            Comment::Doc(text) => {
                self.write_doc_comment_text(text);
            }
        }
    }

    /// Write a doc comment (`{-| ... -}`), applying normalization in ElmFormat mode.
    fn write_doc_comment_text(&mut self, text: &str) {
        if self.is_pretty() {
            let normalized = normalize_doc_comment(text);
            let normalized = collapse_blank_lines_in_doc(&normalized);
            let normalized = normalize_doc_char_literals(&normalized);
            let normalized = normalize_emphasis(&normalized);
            let normalized = normalize_empty_link_refs(&normalized);
            let normalized = normalize_markdown_lists(&normalized);
            let normalized = normalize_fenced_code_blocks(&normalized);
            let normalized = normalize_code_block_indent(&normalized);
            let normalized = ensure_blank_before_code_block_with_trailing_comment(&normalized);
            let normalized = ensure_blank_before_docs_after_prose(&normalized);
            let normalized = normalize_docs_lines(&normalized);
            let normalized = strip_paragraph_leading_whitespace(&normalized);
            let normalized = collapse_prose_internal_spaces(&normalized);
            let normalized = strip_trailing_whitespace_in_doc(&normalized);
            self.write("{-|");
            self.write(&normalized);
            self.write("-}");
        } else {
            self.write("{-|");
            self.write(text);
            self.write("-}");
        }
    }

    fn write_module_header(&mut self, header: &ModuleHeader) {
        match header {
            ModuleHeader::Normal { name, exposing } => {
                self.write("module ");
                self.write_module_name(&name.value);
                self.write(" exposing ");
                self.write_exposing(&exposing.value, true);
            }
            ModuleHeader::Port { name, exposing } => {
                self.write("port module ");
                self.write_module_name(&name.value);
                self.write(" exposing ");
                self.write_exposing(&exposing.value, true);
            }
            ModuleHeader::Effect {
                name,
                exposing,
                command,
                subscription,
            } => {
                self.write("effect module ");
                self.write_module_name(&name.value);
                self.write(" where { ");
                let mut entries = Vec::new();
                if let Some(cmd) = command {
                    entries.push(format!("command = {}", cmd.value));
                }
                if let Some(sub) = subscription {
                    entries.push(format!("subscription = {}", sub.value));
                }
                self.write(&entries.join(", "));
                self.write(" } exposing ");
                self.write_exposing(&exposing.value, true);
            }
        }
    }

    fn write_module_name(&mut self, parts: &[String]) {
        self.write(&parts.join("."));
    }

    // ── Exposing ─────────────────────────────────────────────────────

    fn write_exposing(&mut self, exposing: &Exposing, is_module_header: bool) {
        match exposing {
            Exposing::All(_) => self.write("(..)"),
            Exposing::Explicit(items) => {
                if self.is_pretty() {
                    self.write_exposing_pretty(items, is_module_header);
                } else {
                    self.write_char('(');
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write_exposed_item(&item.value);
                    }
                    self.write_char(')');
                }
            }
        }
    }

    /// elm-format-style exposing list: multiline when long, grouped by `@docs`.
    ///
    /// When the module has `@docs` directives, items are reordered and grouped
    /// to match elm-format's layout:
    /// - 1 `@docs` group → single-line (regardless of length)
    /// - 2+ `@docs` groups → multiline with one group per line
    ///
    /// Without `@docs`, items are listed one per line when the list is long.
    fn write_exposing_pretty(&mut self, items: &[Spanned<ExposedItem>], is_module_header: bool) {
        let doc_groups = self.doc_groups.clone();

        if is_module_header && !doc_groups.is_empty() {
            // Build a lookup from item name to item reference.
            let item_map: std::collections::HashMap<String, &ExposedItem> = items
                .iter()
                .map(|i| (exposed_item_name(&i.value), &i.value))
                .collect();

            // Build ordered list of groups resolved to actual items.
            let mut resolved_groups: Vec<Vec<&ExposedItem>> = Vec::new();
            let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();

            for group in &doc_groups {
                let group_items: Vec<&ExposedItem> = group
                    .iter()
                    .filter_map(|name| {
                        // An item may appear in multiple @docs groups in the
                        // module doc. elm-format only places it in the first
                        // group; later mentions are ignored for layout.
                        if emitted.contains(name.as_str()) {
                            return None;
                        }
                        let item = item_map.get(name.as_str()).copied()?;
                        // elm-format's textToRef only recognizes operators with
                        // 1 or 2 symbol characters.  Operators with 3+ chars
                        // (e.g. </>, <?>, |=, |.) silently fail to match their
                        // @docs entry and end up as leftovers. Replicate that
                        // behavior so our output matches elm-format exactly.
                        if let ExposedItem::Infix(op) = item
                            && op.len() >= 3
                        {
                            return None;
                        }
                        Some(item)
                    })
                    .collect();
                if !group_items.is_empty() {
                    for item in &group_items {
                        emitted.insert(exposed_item_name(item));
                    }
                    resolved_groups.push(group_items);
                }
            }

            // Leftovers not in any @docs group — sorted alphabetically
            // to match elm-format's behavior.
            let mut leftovers: Vec<&ExposedItem> = items
                .iter()
                .filter(|i| !emitted.contains(&exposed_item_name(&i.value)))
                .map(|i| &i.value)
                .collect();
            leftovers.sort_by_key(|a| exposed_item_sort_key(a));
            if !leftovers.is_empty() {
                resolved_groups.push(leftovers);
            }

            if resolved_groups.len() <= 1 {
                // Single group (or all items in one group) → single-line.
                let all_items: Vec<&ExposedItem> = resolved_groups
                    .into_iter()
                    .flat_map(|g| g.into_iter())
                    .collect();
                self.write_char('(');
                for (j, item) in all_items.iter().enumerate() {
                    if j > 0 {
                        self.write(", ");
                    }
                    self.write_exposed_item(item);
                }
                self.write_char(')');
            } else {
                // Multiple groups → multiline, one group per line.
                if self.buf.ends_with(' ') {
                    self.buf.pop();
                }
                self.indent();
                let mut is_first = true;
                for group_items in &resolved_groups {
                    self.newline_indent();
                    if is_first {
                        self.write("( ");
                        is_first = false;
                    } else {
                        self.write(", ");
                    }
                    for (j, item) in group_items.iter().enumerate() {
                        if j > 0 {
                            self.write(", ");
                        }
                        self.write_exposed_item(item);
                    }
                }
                self.newline_indent();
                self.write_char(')');
                self.dedent();
            }
        } else if !is_module_header {
            // Import exposing: elm-format preserves source layout.
            // Always single-line since we don't have source layout info.
            self.write_char('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write_exposed_item(&item.value);
            }
            self.write_char(')');
        } else {
            // Module header without @docs: elm-format sorts alphabetically.
            let mut sorted_items: Vec<&ExposedItem> = items.iter().map(|i| &i.value).collect();
            sorted_items.sort_by_key(|a| exposed_item_sort_key(a));

            let single_line: String = {
                let parts: Vec<String> = sorted_items
                    .iter()
                    .map(|item| exposed_item_to_string(item))
                    .collect();
                format!("({})", parts.join(", "))
            };

            let source_was_multiline = Self::spans_multi_lines(items);
            if !source_was_multiline {
                // elm-format preserves single-line source layout regardless
                // of line length for module exposing.
                self.write(&single_line);
            } else {
                // Multiline: each item on its own indented line.
                if self.buf.ends_with(' ') {
                    self.buf.pop();
                }
                self.indent();
                for (i, item) in sorted_items.iter().enumerate() {
                    self.newline_indent();
                    if i == 0 {
                        self.write("( ");
                    } else {
                        self.write(", ");
                    }
                    self.write_exposed_item(item);
                }
                self.newline_indent();
                self.write_char(')');
                self.dedent();
            }
        }
    }

    fn write_exposed_item(&mut self, item: &ExposedItem) {
        match item {
            ExposedItem::Function(name) => self.write(name),
            ExposedItem::TypeOrAlias(name) => self.write(name),
            ExposedItem::TypeExpose { name, open } => {
                self.write(name);
                if open.is_some() {
                    self.write("(..)");
                }
            }
            ExposedItem::Infix(op) => {
                self.write_char('(');
                self.write(op);
                self.write_char(')');
            }
        }
    }

    // ── Import ───────────────────────────────────────────────────────

    fn write_import(&mut self, import: &Import) {
        self.write("import ");
        self.write_module_name(&import.module_name.value);
        if let Some(alias) = &import.alias {
            // elm-format strips redundant aliases where the alias equals
            // the module name (e.g., `import Foo as Foo` → `import Foo`).
            let is_redundant = self.is_pretty() && alias.value == import.module_name.value;
            if !is_redundant {
                self.write(" as ");
                self.write_module_name(&alias.value);
            }
        }
        if let Some(exposing) = &import.exposing {
            let source_multi = self.is_pretty()
                && exposing.span.end.line > exposing.span.start.line
                && exposing.span.start.line != 0;
            if source_multi {
                if let Exposing::Explicit(items) = &exposing.value {
                    self.indent();
                    self.newline_indent();
                    self.write("exposing");
                    self.indent();
                    self.newline_indent();
                    let mut sorted: Vec<&ExposedItem> =
                        items.iter().map(|i| &i.value).collect();
                    sorted.sort_by_key(|a| exposed_item_sort_key(a));
                    self.write_char('(');
                    self.write_char(' ');
                    for (i, item) in sorted.iter().enumerate() {
                        if i > 0 {
                            self.newline_indent();
                            self.write(", ");
                        }
                        self.write_exposed_item(item);
                    }
                    self.newline_indent();
                    self.write_char(')');
                    self.dedent();
                    self.dedent();
                    return;
                }
            }
            self.write(" exposing ");
            if self.is_pretty() {
                self.write_import_exposing_sorted(&exposing.value);
            } else {
                self.write_exposing(&exposing.value, false);
            }
        }
    }

    /// Merge multiple imports of the same module and write as a single import.
    /// elm-format merges duplicate imports by combining aliases and exposing lists.
    fn write_merged_imports(&mut self, imports: &[&Import]) {
        assert!(!imports.is_empty());
        let first = imports[0];

        self.write("import ");
        self.write_module_name(&first.module_name.value);

        // Merge alias: take the first non-None alias found (there should be
        // at most one alias across duplicates). Strip redundant aliases.
        let merged_alias = imports.iter().find_map(|imp| {
            imp.alias.as_ref().and_then(|a| {
                if a.value == imp.module_name.value {
                    None // redundant alias
                } else {
                    Some(&a.value)
                }
            })
        });
        if let Some(alias) = merged_alias {
            self.write(" as ");
            self.write_module_name(alias);
        }

        // Merge exposing lists: if any import has `exposing (..)`, use that.
        // Otherwise, combine all explicit exposing items.
        let has_expose_all = imports
            .iter()
            .any(|imp| matches!(&imp.exposing, Some(e) if matches!(e.value, Exposing::All(_))));
        if has_expose_all {
            self.write(" exposing (..)");
        } else {
            // Collect all exposed items from all imports.
            let mut all_items: Vec<&ExposedItem> = Vec::new();
            for imp in imports {
                if let Some(exposing) = &imp.exposing
                    && let Exposing::Explicit(items) = &exposing.value
                {
                    for item in items {
                        all_items.push(&item.value);
                    }
                }
            }
            if !all_items.is_empty() {
                // Deduplicate and sort.
                all_items.sort_by_key(|a| exposed_item_sort_key(a));
                all_items.dedup_by(|a, b| exposed_item_sort_key(a) == exposed_item_sort_key(b));
                self.write(" exposing (");
                for (i, item) in all_items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_exposed_item(item);
                }
                self.write_char(')');
            }
        }
    }

    /// Write an import's exposing list with items sorted alphabetically,
    /// matching elm-format's behavior. elm-format sorts by the string
    /// representation: `(op)` items come before alphabetic names since
    /// `(` sorts before letters in ASCII.
    fn write_import_exposing_sorted(&mut self, exposing: &Exposing) {
        match exposing {
            Exposing::All(_) => self.write("(..)"),
            Exposing::Explicit(items) => {
                let mut sorted: Vec<&ExposedItem> = items.iter().map(|i| &i.value).collect();
                sorted.sort_by_key(|a| exposed_item_sort_key(a));
                self.write_char('(');
                for (i, item) in sorted.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_exposed_item(item);
                }
                self.write_char(')');
            }
        }
    }

    // ── Declarations ─────────────────────────────────────────────────

    pub fn write_declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::FunctionDeclaration(func) => self.write_function(func),
            Declaration::AliasDeclaration(alias) => self.write_type_alias(alias),
            Declaration::CustomTypeDeclaration(ct) => self.write_custom_type(ct),
            Declaration::PortDeclaration(sig) => {
                self.write("port ");
                self.write_signature(sig);
            }
            Declaration::InfixDeclaration(infix) => self.write_infix_decl(infix),
            Declaration::Destructuring { pattern, body } => {
                self.write_pattern(&pattern.value);
                self.write(" =");
                self.indent();
                self.newline_indent();
                self.write_spanned_expr(&body);
                self.dedent();
            }
        }
    }

    fn write_function(&mut self, func: &Function) {
        if let Some(doc) = &func.documentation {
            self.write_doc_comment_text(&doc.value);
            self.newline();
        }
        if let Some(sig) = &func.signature {
            self.write_signature(&sig.value);
            self.newline();
        }
        self.write_function_impl(&func.declaration.value);
    }

    fn write_signature(&mut self, sig: &Signature) {
        self.write(&sig.name.value);
        if self.is_pretty() && Self::type_ann_spans_multi_lines(&sig.type_annotation) {
            self.write(" :");
            self.indent();
            self.newline_indent();
            self.write_type_multiline(&sig.type_annotation);
            self.dedent();
        } else {
            self.write(" : ");
            self.write_type(&sig.type_annotation.value);
        }
    }

    /// True when the type annotation's content crosses source lines.
    /// Uses the inner content span rather than the Spanned wrapper span,
    /// because the wrapper may include leading whitespace/newline after `=`
    /// or `:` (giving a misleading multi-line appearance for a single-line
    /// annotation like `    msg -> model -> ( model, Cmd msg )`).
    fn type_ann_spans_multi_lines(sp: &Spanned<TypeAnnotation>) -> bool {
        let (s, e) = Self::type_ann_content_lines(sp);
        s != 0 && e != 0 && e > s
    }

    /// Return `(start_line, end_line)` for the content of a type annotation,
    /// peeking through the wrapper span into inner sub-spans where wrappers
    /// may include leading whitespace.
    fn type_ann_content_lines(sp: &Spanned<TypeAnnotation>) -> (u32, u32) {
        match &sp.value {
            TypeAnnotation::FunctionType { from, to } => {
                let (fs, _fe) = Self::type_ann_content_lines(from);
                let (_ts, te) = Self::type_ann_content_lines(to);
                (fs, te)
            }
            TypeAnnotation::Typed { name, args, .. } => {
                let s = name.span.start.line;
                let e = args
                    .last()
                    .map(|a| Self::type_ann_content_lines(a).1)
                    .unwrap_or(name.span.end.line);
                (s, e)
            }
            TypeAnnotation::Tupled(elems) => {
                let s = elems
                    .first()
                    .map(|e| Self::type_ann_content_lines(e).0)
                    .unwrap_or(sp.span.start.line);
                (s, sp.span.end.line)
            }
            _ => (sp.span.start.line, sp.span.end.line),
        }
    }

    /// Write a type annotation broken across lines: function arrows on their
    /// own lines, record arguments expanded multi-line. Matches elm-format's
    /// layout when the source annotation spans multiple lines.
    fn write_type_multiline(&mut self, ty: &Spanned<TypeAnnotation>) {
        match &ty.value {
            TypeAnnotation::FunctionType { from, to } => {
                self.write_type_arm_multiline(from);
                let mut cur: &Spanned<TypeAnnotation> = to;
                loop {
                    // If the current RHS is a function type that had outer
                    // parens in source (e.g. `-> (a -> b)` on its own line),
                    // elm-format keeps the whole paren group on one arm line
                    // instead of descending into its arrows.
                    if self.is_pretty()
                        && matches!(cur.value, TypeAnnotation::FunctionType { .. })
                        && Self::type_ann_has_outer_parens(cur)
                    {
                        self.newline_indent();
                        self.write("-> (");
                        self.write_type(&cur.value);
                        self.write_char(')');
                        break;
                    }
                    match &cur.value {
                        TypeAnnotation::FunctionType { from, to } => {
                            self.write_function_arm_arrow(from);
                            cur = to;
                        }
                        _ => {
                            self.write_function_arm_arrow(cur);
                            break;
                        }
                    }
                }
            }
            TypeAnnotation::Record(fields) if !fields.is_empty() => {
                self.write_record_type_fields_multiline(fields, None);
            }
            TypeAnnotation::GenericRecord { base, fields } if !fields.is_empty() => {
                self.write_record_type_fields_multiline(fields, Some(&base.value));
            }
            TypeAnnotation::Typed { module_name, name, args } if !args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(&name.value);
                for arg in args {
                    if Self::type_ann_spans_multi_lines(arg) {
                        match &arg.value {
                            TypeAnnotation::Record(rfields) if !rfields.is_empty() => {
                                self.indent();
                                self.newline_indent();
                                self.write_record_type_fields_multiline(rfields, None);
                                self.dedent();
                                continue;
                            }
                            TypeAnnotation::GenericRecord { base, fields: rfields } => {
                                self.indent();
                                self.newline_indent();
                                self.write_record_type_fields_multiline(
                                    rfields,
                                    Some(&base.value),
                                );
                                self.dedent();
                                continue;
                            }
                            _ => {}
                        }
                    }
                    self.write_char(' ');
                    self.write_type_atomic(&arg.value);
                }
            }
            TypeAnnotation::Tupled(elems) if elems.len() >= 2 => {
                self.write_tupled_multiline(elems);
            }
            _ => self.write_type(&ty.value),
        }
    }

    /// Emit a multi-line tuple type:
    /// ```text
    /// ( A
    /// , B
    /// , C
    /// )
    /// ```
    /// Each element starts 2 columns past the opening paren. Nested
    /// records inside elements expand multi-line at their element column.
    fn write_tupled_multiline(&mut self, elems: &[Spanned<TypeAnnotation>]) {
        self.write("( ");
        self.write_tuple_elem(&elems[0]);
        for elem in &elems[1..] {
            self.newline_indent();
            self.write(", ");
            self.write_tuple_elem(elem);
        }
        self.newline_indent();
        self.write(")");
    }

    /// Write a single tuple element. If the element is a multi-line record
    /// type, expand it aligned to its own column (2 past the tuple's `(`).
    fn write_tuple_elem(&mut self, elem: &Spanned<TypeAnnotation>) {
        let elem_multi = Self::type_ann_spans_multi_lines(elem);
        match &elem.value {
            TypeAnnotation::Record(fields) if elem_multi && fields.len() >= 2 => {
                self.with_extra_indent(2, |p| p.write_record_type_fields_multiline(fields, None));
            }
            TypeAnnotation::GenericRecord { base, fields } if elem_multi => {
                self.with_extra_indent(2, |p| {
                    p.write_record_type_fields_multiline(fields, Some(&base.value))
                });
            }
            _ => self.write_type(&elem.value),
        }
    }

    /// Run `f` with indent_extra bumped by `delta` columns so that
    /// newline_indent within the closure aligns to the current column.
    fn with_extra_indent(&mut self, delta: u32, f: impl FnOnce(&mut Self)) {
        let saved = self.indent_extra;
        self.indent_extra = saved + delta;
        f(self);
        self.indent_extra = saved;
    }

    /// Emit one `-> <arm>` step of a multi-line function type. When the arm
    /// itself will expand onto multiple lines (e.g. a 2+ field record whose
    /// source span is multi-line), elm-format puts the arrow alone on its
    /// own line and indents the arm below it; otherwise the arrow and arm
    /// sit on the same line.
    fn write_function_arm_arrow(&mut self, arm: &Spanned<TypeAnnotation>) {
        self.newline_indent();
        if Self::type_arm_is_multiline(arm) {
            self.write("->");
            self.indent();
            self.newline_indent();
            self.write_type_arm_multiline(arm);
            self.dedent();
        } else {
            self.write("-> ");
            self.write_type_arm_multiline(arm);
        }
    }

    fn type_arm_is_multiline(arm: &Spanned<TypeAnnotation>) -> bool {
        let source_multi = arm.span.end.line > arm.span.start.line && arm.span.start.line != 0;
        match &arm.value {
            TypeAnnotation::Record(fields) if fields.len() >= 2 => source_multi,
            TypeAnnotation::GenericRecord { fields, .. } if !fields.is_empty() => source_multi,
            TypeAnnotation::Typed { args, .. } if !args.is_empty() && source_multi => {
                // A Typed constructor applied to a multi-line record arg
                // expands `-> Ctor { ... }` across multiple lines.
                args.iter().any(|a| match &a.value {
                    TypeAnnotation::Record(fs) if fs.len() >= 2 => {
                        Self::type_ann_spans_multi_lines(a)
                    }
                    TypeAnnotation::GenericRecord { fields: fs, .. } if !fs.is_empty() => {
                        Self::type_ann_spans_multi_lines(a)
                    }
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// Write a single arm of a multi-line function type. Records expand
    /// multi-line; other forms stay on one line (parenthesized if needed).
    fn write_type_arm_multiline(&mut self, arm: &Spanned<TypeAnnotation>) {
        let source_multi = arm.span.end.line > arm.span.start.line && arm.span.start.line != 0;
        match &arm.value {
            TypeAnnotation::Record(fields) if fields.len() >= 2 && source_multi => {
                self.write_record_type_fields_multiline(fields, None);
            }
            TypeAnnotation::GenericRecord { base, fields } if !fields.is_empty() && source_multi => {
                self.write_record_type_fields_multiline(fields, Some(&base.value));
            }
            TypeAnnotation::Typed { args, .. } if !args.is_empty() && source_multi => {
                self.write_type_multiline(arm);
            }
            _ => self.write_type_non_arrow(&arm.value),
        }
    }

    fn write_function_impl(&mut self, imp: &FunctionImplementation) {
        self.write(&imp.name.value);
        for arg in &imp.args {
            self.write_char(' ');
            self.write_pattern_atomic(&arg.value);
        }
        self.write(" =");
        self.indent();
        self.newline_indent();
        if !imp.body.comments.is_empty() {
            self.write_leading_comments(&imp.body.comments);
        }
        // At the top of a value definition RHS, parens around an operator
        // application are always redundant. elm-format strips them.
        let body = if self.is_pretty() {
            unwrap_parens(&imp.body.value)
        } else {
            &imp.body.value
        };
        self.write_expr(body);
        self.dedent();
    }

    fn write_type_alias(&mut self, alias: &TypeAlias) {
        if let Some(doc) = &alias.documentation {
            self.write_doc_comment_text(&doc.value);
            self.newline();
        }
        self.write("type alias ");
        self.write(&alias.name.value);
        for g in &alias.generics {
            self.write_char(' ');
            self.write(&g.value);
        }
        self.write(" =");
        self.indent();
        self.newline_indent();
        if self.is_pretty() {
            if Self::type_ann_spans_multi_lines(&alias.type_annotation) {
                self.write_type_multiline(&alias.type_annotation);
            } else {
                self.write_type_pretty_toplevel(&alias.type_annotation.value);
            }
        } else {
            self.write_type(&alias.type_annotation.value);
        }
        self.dedent();
    }

    fn write_custom_type(&mut self, ct: &CustomType) {
        if let Some(doc) = &ct.documentation {
            self.write_doc_comment_text(&doc.value);
            self.newline();
        }
        self.write("type ");
        self.write(&ct.name.value);
        for g in &ct.generics {
            self.write_char(' ');
            self.write(&g.value);
        }
        self.indent();
        for (i, ctor) in ct.constructors.iter().enumerate() {
            if i == 0 {
                self.newline_indent();
                self.write("= ");
                for c in &ctor.comments {
                    self.write_comment(&c.value);
                    self.newline();
                    self.write_indent();
                    self.write("  ");
                }
                self.write_value_constructor(&ctor.value);
            } else {
                // Post-pipe comments (ctor.comments) sit on their own line(s)
                // at the constructor-name column (indent + 2), BEFORE the `|`,
                // with no blank line above.
                for c in &ctor.comments {
                    self.newline();
                    self.write_indent();
                    self.write("  ");
                    self.write_comment(&c.value);
                }
                self.newline_indent();
                self.write("| ");
                // Pre-pipe comments (ctor.value.pre_pipe_comments) sit INLINE
                // with the `|`, each followed by a newline + indent + 2 so the
                // following constructor name aligns at the `|` column + 2.
                let pre = &ctor.value.pre_pipe_comments;
                for c in pre {
                    self.write_comment(&c.value);
                    self.newline();
                    self.write_indent();
                    self.write("  ");
                }
                self.write_value_constructor(&ctor.value);
            }
        }
        self.dedent();
    }

    fn write_value_constructor(&mut self, ctor: &ValueConstructor) {
        self.write(&ctor.name.value);
        // Only force multi-line constructor layout when an arg's printed
        // form will genuinely span multiple lines. Tuples and applied
        // types are always printed single-line via `write_type_atomic`,
        // so forcing a break would be rewrapped by elm-format. The two
        // arg kinds that reliably print multi-line are records with 2+
        // fields, and function types that span multiple source lines
        // (printed in the parenthesized aligned-arrow form).
        let any_multiline = self.is_pretty()
            && ctor.args.iter().any(|a| Self::ctor_arg_prints_multiline(a));
        if any_multiline {
            self.indent();
            for arg in &ctor.args {
                self.newline_indent();
                self.write_ctor_arg_multiline(arg);
            }
            self.dedent();
        } else {
            for arg in &ctor.args {
                self.write_char(' ');
                self.write_type_atomic(&arg.value);
            }
        }
    }

    fn ctor_arg_prints_multiline(a: &Spanned<TypeAnnotation>) -> bool {
        if !Self::type_ann_spans_multi_lines(a) {
            return false;
        }
        matches!(&a.value, TypeAnnotation::Record(fs) if fs.len() >= 2)
            || matches!(&a.value, TypeAnnotation::FunctionType { .. })
    }

    fn write_ctor_arg_multiline(&mut self, ty: &Spanned<TypeAnnotation>) {
        match &ty.value {
            TypeAnnotation::Record(fields) if fields.len() >= 2 => {
                self.write_record_type_fields_multiline(fields, None);
            }
            TypeAnnotation::FunctionType { .. } if Self::type_ann_spans_multi_lines(ty) => {
                self.write_parenthesized_function_type_multiline(&ty.value);
            }
            _ => self.write_type_atomic(&ty.value),
        }
    }

    /// Write a multi-line function type wrapped in parens, with each arrow
    /// aligned one column past the opening paren and the closing paren on
    /// its own line aligned with the opening. Used for custom-type ctor
    /// args whose source function type spanned multiple lines.
    ///
    /// ```text
    /// (Location
    ///  -> ResolvedNames
    ///  -> Result err a
    /// )
    /// ```
    fn write_parenthesized_function_type_multiline(&mut self, ty: &TypeAnnotation) {
        self.write_char('(');
        // Collect arms so we can emit the first inline with `(` and the
        // rest on subsequent lines.
        let mut arms: Vec<&TypeAnnotation> = Vec::new();
        let mut cur = ty;
        loop {
            if let TypeAnnotation::FunctionType { from, to } = cur {
                arms.push(&from.value);
                cur = &to.value;
            } else {
                arms.push(cur);
                break;
            }
        }
        if let Some((first, rest)) = arms.split_first() {
            self.write_type_atomic(first);
            for arm in rest {
                self.newline_indent();
                self.write_char(' ');
                self.write("-> ");
                self.write_type_non_arrow(arm);
            }
        }
        self.newline_indent();
        self.write_char(')');
    }

    fn write_infix_decl(&mut self, infix: &InfixDef) {
        self.write("infix ");
        if self.is_pretty() {
            // elm-format pads direction to 6 chars (including trailing space):
            // "left  ", "right ", "non   "
            match infix.direction.value {
                InfixDirection::Left => self.write("left  "),
                InfixDirection::Right => self.write("right "),
                InfixDirection::Non => self.write("non   "),
            }
        } else {
            match infix.direction.value {
                InfixDirection::Left => self.write("left"),
                InfixDirection::Right => self.write("right"),
                InfixDirection::Non => self.write("non"),
            }
            self.write_char(' ');
        }
        self.write(&infix.precedence.value.to_string());
        self.write(" (");
        self.write(&infix.operator.value);
        self.write(") = ");
        self.write(&infix.function.value);
    }

    // ── Type annotations ─────────────────────────────────────────────

    pub fn write_type(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::FunctionType { from, to } => {
                self.write_type_non_arrow(&from.value);
                self.write(" -> ");
                // elm-format preserves redundant parens around function types
                // on the RHS of an arrow (e.g. `a -> (b -> c)`). The parser
                // flags this by extending the inner value's span to cover
                // the parens, so `to.span.start` precedes the inner `from`'s
                // span start.
                let preserve_parens = self.is_pretty()
                    && matches!(to.value, TypeAnnotation::FunctionType { .. })
                    && Self::type_ann_has_outer_parens(to);
                if preserve_parens {
                    self.write_char('(');
                    self.write_type(&to.value);
                    self.write_char(')');
                } else {
                    self.write_type(&to.value);
                }
            }
            _ => self.write_type_non_arrow(ty),
        }
    }

    /// Detect whether a FunctionType Spanned was originally wrapped in parens
    /// in the source. The parser records this by setting the Spanned's span
    /// to cover the parens, so the outer span starts before the inner `from`'s
    /// span starts on the same line. A start on an earlier line could simply
    /// be from whitespace/newlines preceding the RHS of an arrow; require
    /// same-line to avoid that false positive.
    fn type_ann_has_outer_parens(ta: &Spanned<TypeAnnotation>) -> bool {
        if let TypeAnnotation::FunctionType { from, .. } = &ta.value {
            let outer = ta.span.start;
            let inner = from.span.start;
            if outer.line == 0 || inner.line == 0 {
                return false;
            }
            outer.line == inner.line && outer.column < inner.column
        } else {
            false
        }
    }

    /// Write a type annotation at the top level of a type alias body
    /// (ElmFormat mode only). Record types go multiline when the fields
    /// span multiple lines in the source; otherwise kept inline.
    fn write_type_pretty_toplevel(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::Record(fields) if fields.len() >= 2 => {
                // Check if the record spans multiple lines in the source.
                let spans_multi_lines = if fields.len() >= 2 {
                    let first_line = fields.first().map(|f| f.span.start.line).unwrap_or(0);
                    let last_line = fields.last().map(|f| f.span.end.line).unwrap_or(0);
                    last_line > first_line
                } else {
                    false
                };
                if spans_multi_lines {
                    self.write_record_type_fields_multiline(fields, None);
                } else {
                    self.write_type(ty);
                }
            }
            _ => self.write_type(ty),
        }
    }

    /// Multiline record type: elm-format style with first field on `{` line.
    fn write_record_type_fields_multiline(
        &mut self,
        fields: &[Spanned<RecordField>],
        base: Option<&str>,
    ) {
        if let Some(base_name) = base {
            self.write("{ ");
            self.write(base_name);
            self.indent();
            self.newline_indent();
            self.write("| ");
            for (i, field) in fields.iter().enumerate() {
                if i > 0 {
                    self.write_record_field_separator_with_comments(&field.comments);
                } else {
                    self.write_record_field_leading_comments_inline(&field.comments);
                }
                self.write(&field.value.name.value);
                self.write_field_type_with_possible_break(&field.value.type_annotation);
            }
            self.dedent();
        } else {
            self.write("{ ");
            self.write_record_field_leading_comments_inline(&fields[0].comments);
            self.write(&fields[0].value.name.value);
            self.write_field_type_with_possible_break(&fields[0].value.type_annotation);
            for field in &fields[1..] {
                self.write_record_field_separator_with_comments(&field.comments);
                self.write(&field.value.name.value);
                self.write_field_type_with_possible_break(&field.value.type_annotation);
            }
        }
        self.newline_indent();
        self.write("}");
    }

    /// Write ` : <type>` for a record field's type annotation. When the
    /// source spans multiple lines — either because the value itself is a
    /// multi-line record, or simply because the source put the value on a
    /// new line after `:` — break the value onto its own indented line:
    /// ` :\n    <type>`.
    fn write_field_type_with_possible_break(&mut self, ta: &Spanned<TypeAnnotation>) {
        // Use the WRAPPER span here (not content span) because the wrapper
        // includes leading whitespace after `:`; if that whitespace includes
        // a newline, the source put the value on its own line.
        let wrapper_multi = ta.span.start.line != 0
            && ta.span.end.line != 0
            && ta.span.end.line > ta.span.start.line;
        if self.is_pretty() && wrapper_multi {
            match &ta.value {
                TypeAnnotation::Record(rfields) if !rfields.is_empty() => {
                    self.write(" :");
                    self.indent();
                    self.newline_indent();
                    self.write_record_type_fields_multiline(rfields, None);
                    self.dedent();
                    return;
                }
                TypeAnnotation::GenericRecord { base, fields: rfields } => {
                    self.write(" :");
                    self.indent();
                    self.newline_indent();
                    self.write_record_type_fields_multiline(rfields, Some(&base.value));
                    self.dedent();
                    return;
                }
                TypeAnnotation::Typed { .. } => {
                    self.write(" :");
                    self.indent();
                    self.newline_indent();
                    self.write_type_multiline(ta);
                    self.dedent();
                    return;
                }
                _ => {
                    self.write(" :");
                    self.indent();
                    self.newline_indent();
                    self.write_type(&ta.value);
                    self.dedent();
                    return;
                }
            }
        }
        self.write(" : ");
        self.write_type(&ta.value);
    }

    fn write_type_non_arrow(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::FunctionType { .. } => {
                self.write_char('(');
                self.write_type(ty);
                self.write_char(')');
            }
            TypeAnnotation::Typed {
                module_name,
                name,
                args,
            } => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(&name.value);
                for arg in args {
                    self.write_char(' ');
                    self.write_type_atomic(&arg.value);
                }
            }
            _ => self.write_type_atomic(ty),
        }
    }

    fn write_type_atomic(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::GenericType(name) => self.write(name),
            TypeAnnotation::Unit => self.write("()"),
            TypeAnnotation::Typed {
                module_name,
                name,
                args,
            } if args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(&name.value);
            }
            TypeAnnotation::Tupled(elems) => {
                self.write("( ");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_type(&elem.value);
                }
                self.write(" )");
            }
            TypeAnnotation::Record(fields) => {
                self.write_record_type_fields(fields, None);
            }
            TypeAnnotation::GenericRecord { base, fields } => {
                self.write_record_type_fields(fields, Some(&base.value));
            }
            _ => {
                self.write_char('(');
                self.write_type(ty);
                self.write_char(')');
            }
        }
    }

    fn write_record_type_fields(&mut self, fields: &[Spanned<RecordField>], base: Option<&str>) {
        if fields.is_empty() && base.is_none() {
            self.write("{}");
            return;
        }
        self.write("{ ");
        if let Some(base_name) = base {
            self.write(base_name);
            self.write(" | ");
        }
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.write(&field.value.name.value);
            self.write(" : ");
            self.write_type(&field.value.type_annotation.value);
        }
        self.write(" }");
    }

    // ── Patterns ─────────────────────────────────────────────────────

    pub fn write_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::As { pattern, name } => {
                // elm-format wraps constructor patterns with args in parens
                // when used with `as`: `(Ctor a b c) as name`
                let needs_parens = self.is_pretty()
                    && matches!(
                        &pattern.value,
                        Pattern::Constructor { args, .. } if !args.is_empty()
                    );
                if needs_parens {
                    self.write_char('(');
                }
                self.write_pattern_cons(&pattern.value);
                if needs_parens {
                    self.write_char(')');
                }
                self.write(" as ");
                self.write(&name.value);
            }
            _ => self.write_pattern_cons(pat),
        }
    }

    fn write_pattern_cons(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Cons { head, tail } => {
                // elm-format wraps constructor patterns with args in parens
                // on the left side of `::`: `(Ctor a) :: rest`
                let needs_parens = self.is_pretty()
                    && matches!(
                        &head.value,
                        Pattern::Constructor { args, .. } if !args.is_empty()
                    );
                if needs_parens {
                    self.write_char('(');
                }
                self.write_pattern_app(&head.value);
                if needs_parens {
                    self.write_char(')');
                }
                self.write(" :: ");
                self.write_pattern_cons(&tail.value);
            }
            _ => self.write_pattern_app(pat),
        }
    }

    fn write_pattern_app(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Constructor {
                module_name,
                name,
                args,
            } if !args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
                for arg in args {
                    self.write_char(' ');
                    self.write_pattern_atomic(&arg.value);
                }
            }
            _ => self.write_pattern_atomic(pat),
        }
    }

    fn write_pattern_atomic(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Anything => self.write_char('_'),
            Pattern::Var(name) => self.write(name),
            Pattern::Literal(lit) => self.write_literal(lit),
            Pattern::Unit => self.write("()"),
            Pattern::Hex(n) => {
                self.write("0x");
                self.write(&format!("{n:X}"));
            }
            Pattern::Tuple(elems) => {
                self.write("( ");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_pattern(&elem.value);
                }
                self.write(" )");
            }
            Pattern::Constructor {
                module_name,
                name,
                args,
            } if args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
            }
            Pattern::Record(fields) => {
                self.write("{ ");
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(&field.value);
                }
                self.write(" }");
            }
            Pattern::List(elems) => {
                if elems.is_empty() {
                    self.write("[]");
                } else {
                    self.write("[ ");
                    for (i, elem) in elems.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write_pattern(&elem.value);
                    }
                    self.write(" ]");
                }
            }
            Pattern::Parenthesized(inner) => {
                self.write_char('(');
                self.write_pattern(&inner.value);
                self.write_char(')');
            }
            Pattern::Constructor { .. } | Pattern::Cons { .. } | Pattern::As { .. } => {
                self.write_char('(');
                self.write_pattern(pat);
                self.write_char(')');
            }
        }
    }

    // ── Expressions ──────────────────────────────────────────────────

    /// Write an expression in a "top-level body" context where block
    /// expressions (case/if/let/lambda) can appear directly.
    pub fn write_expr(&mut self, expr: &Expr) {
        self.write_expr_inner(expr);
    }

    /// Write a spanned expression, pushing its source span onto the span
    /// stack so children (List, Tuple, etc.) can consult it for multi-line
    /// detection based on the outer brackets' lines, not just element spans.
    pub fn write_spanned_expr(&mut self, spanned: &Spanned<Expr>) {
        self.expr_span_stack.push(spanned.span);
        self.write_expr_inner(&spanned.value);
        self.expr_span_stack.pop();
    }

    fn current_expr_span(&self) -> Option<Span> {
        self.expr_span_stack.last().copied()
    }

    /// Emit leading comments attached to a node.
    fn write_leading_comments(&mut self, comments: &[Spanned<Comment>]) {
        for c in comments {
            self.write_comment(&c.value);
            self.newline();
            self.write_indent();
        }
    }

    /// Emit inline leading comments for the FIRST record-type field (after
    /// `{ ` or `| `). Block comments stay inline; line comments get promoted
    /// to their own line at base indent + 2 so the field name aligns with the
    /// column after `{ `.
    fn write_record_field_leading_comments_inline(&mut self, comments: &[Spanned<Comment>]) {
        for c in comments {
            self.write_comment(&c.value);
            self.newline();
            self.write_indent();
            self.write("  ");
        }
    }

    /// Emit the separator between two record-type fields, placing any leading
    /// comments appropriately. elm-format handles block vs. line comments
    /// differently:
    ///   - Block comment: `, {- .. -}\n<indent>  field` (+2 col for field)
    ///   - Line comment: blank line, then `<indent>-- line\n<indent>, field`
    fn write_record_field_separator_with_comments(&mut self, comments: &[Spanned<Comment>]) {
        let has_line_comment = comments
            .iter()
            .any(|c| matches!(c.value, Comment::Line(_)));

        if has_line_comment {
            for (i, c) in comments.iter().enumerate() {
                if i == 0 {
                    self.newline();
                    self.newline();
                } else {
                    self.newline();
                }
                self.write_indent();
                self.write_comment(&c.value);
            }
            self.newline_indent();
            self.write(", ");
        } else {
            self.newline_indent();
            self.write(", ");
            for c in comments {
                self.write_comment(&c.value);
                self.newline();
                self.write_indent();
                self.write("  ");
            }
        }
    }

    /// The core expression dispatcher.
    fn write_expr_inner(&mut self, expr: &Expr) {
        let expr = if self.is_pretty() {
            unwrap_parens_non_block(expr)
        } else {
            expr
        };
        match expr {
            Expr::OperatorApplication {
                operator,
                left,
                right,
                ..
            } => {
                // elm-format strips redundant Parenthesized wrappers on the
                // right side of `<|` since `<|` is right-associative at the
                // lowest precedence. The only exception is `|>` (same
                // precedence, opposite associativity), which cannot be mixed
                // with `<|` without parens — stripping would produce invalid
                // Elm.
                let right_expr = if self.is_pretty() && operator == "<|" {
                    let inner = unwrap_parens(&right.value);
                    let inner_needs_parens = matches!(
                        inner,
                        Expr::OperatorApplication { operator: inner_op, .. }
                            if matches!(inner_op.as_str(), "|>" | "|." | "|=")
                    );
                    if inner_needs_parens {
                        &right.value
                    } else {
                        inner
                    }
                } else {
                    &right.value
                };

                // In pretty mode, flatten left-associative pipeline chains.
                // elm-format's rule: if ANY part of a binops expression
                // contains newlines (i.e., any operand is multiline), ALL
                // operators break to vertical. (`>>` / `<<` are handled below
                // via the right-associative path, since they right-associate
                // in Elm.)
                if self.is_pretty()
                    && matches!(operator.as_str(), "|>" | "|." | "|=")
                    && let Some((head, rest)) = flatten_mixed_pipe_chain(expr)
                {
                    let any_ml = self.is_multiline(&head.value)
                        || rest.iter().any(|(_, op)| self.is_multiline(&op.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        // When the pipe chain goes vertical and its head is
                        // itself a binop chain (e.g. `a ++ b |> f`), elm-
                        // format flattens the head's operators into the same
                        // vertical layout at the same indent column.
                        let (real_head, head_rest) = match flatten_as_chain(&head.value) {
                            Some(x) => x,
                            None => (head, Vec::new()),
                        };
                        let first_op = head_rest
                            .first()
                            .map(|(op, _)| *op)
                            .unwrap_or(operator.as_str());
                        let outer_chain = self.in_vertical_chain;
                        // Capture the column where the head begins so
                        // continuations align at head_col + indent_width,
                        // even when writing inside a list/tuple element
                        // where the ambient indent counter lags the cursor.
                        let head_col = self.current_column();
                        self.write_expr_operand(real_head, first_op, true);
                        let saved_indent = self.indent;
                        let saved_extra = self.indent_extra;
                        let saved_stack = self.indent_extra_stack.clone();
                        if !outer_chain {
                            let w = self.config.indent_width;
                            self.indent = head_col / w;
                            self.indent_extra = (head_col % w) as u32;
                            self.indent_extra_stack.clear();
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in head_rest.iter().chain(rest.iter()) {
                            for c in &operand.comments {
                                if matches!(c.value, Comment::Line(_)) {
                                    self.newline_indent();
                                    self.write_comment(&c.value);
                                }
                            }
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.indent = saved_indent;
                            self.indent_extra = saved_extra;
                            self.indent_extra_stack = saved_stack;
                        }
                        return;
                    }
                    // All operands are single-line; fall through to
                    // normal inline path (which handles recursion).
                }

                // Same rule for right-associative ::, ++, >>, << chains.
                // (`>>` / `<<` function composition is right-associative in
                // Elm, so we need `flatten_right_assoc_chain` to lay them out
                // at a single indent level instead of letting the AST shape
                // produce stair-stepped nesting.)
                //
                // `::` and `++` share precedence 5 right-assoc, and elm-format
                // unifies mixed chains (e.g. `a :: b :: xs ++ ys`) into a single
                // vertical layout. Use the mixed flattener for those operators.
                if self.is_pretty()
                    && matches!(operator.as_str(), "::" | "++")
                    && let Some((head, rest)) = flatten_mixed_cons_append_chain(expr)
                {
                    let any_ml = self.is_multiline(&head.value)
                        || rest.iter().any(|(_, e)| self.is_multiline(&e.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(head, operator, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in &rest {
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.dedent();
                        }
                        return;
                    }
                }

                if self.is_pretty()
                    && matches!(operator.as_str(), ">>" | "<<")
                    && let Some(chain) = flatten_right_assoc_chain(expr, operator)
                {
                    let any_ml = chain.iter().any(|op| self.is_multiline(&op.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(chain[0], operator, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for operand in &chain[1..] {
                            self.newline_indent();
                            self.write(operator);
                            self.write_char(' ');
                            self.write_expr_operand(operand, operator, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.dedent();
                        }
                        return;
                    }
                }

                // Mixed comparison+arithmetic chains (e.g. doc-comment
                // assertion reparses `14 / 4 == 3.5 - 1 / 4`). elm-format
                // lays every operator at the same indent column regardless
                // of precedence. Must run before the arithmetic-only
                // flattener since the top operator here is `==`.
                if self.is_pretty()
                    && matches!(
                        operator.as_str(),
                        "==" | "/=" | "<" | ">" | "<=" | ">=" | "+" | "-" | "*" | "/" | "//"
                    )
                    && let Some((head, rest)) = flatten_mixed_comparison_arithmetic_chain(expr)
                {
                    let any_ml = self.is_multiline(&head.value)
                        || rest.iter().any(|(_, e)| self.is_multiline(&e.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(head, operator, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in &rest {
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.dedent();
                        }
                        return;
                    }
                }

                // Arithmetic operators `+` `-` `*` `/` `//` sit at two
                // precedences (6 and 7). Mixed chains (`a - k * p`) parse
                // with the higher-precedence op nested inside, but
                // elm-format lays the whole thing out as a single vertical
                // sequence with every operator at the same indent column.
                if self.is_pretty()
                    && matches!(operator.as_str(), "+" | "-" | "*" | "/" | "//")
                    && let Some((head, rest)) = flatten_mixed_arithmetic_chain(expr)
                {
                    let any_ml = self.is_multiline(&head.value)
                        || rest.iter().any(|(_, op)| self.is_multiline(&op.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(head, operator, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in &rest {
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.dedent();
                        }
                        return;
                    }
                }

                // `&&` and `||` are right-associative in Elm but mix across
                // precedences 2 and 3. elm-format lays out such mixed chains
                // as a single vertical sequence with all operators aligned at
                // the same indent column, regardless of parse-tree grouping.
                if self.is_pretty()
                    && matches!(operator.as_str(), "&&" | "||")
                    && let Some((head, rest)) = flatten_mixed_logical_chain(expr)
                {
                    let any_ml = self.is_multiline(&head.value)
                        || rest.iter().any(|(_, e)| self.is_multiline(&e.value))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(head, operator, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in &rest {
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.in_vertical_chain = outer_chain;
                        if !outer_chain {
                            self.dedent();
                        }
                        return;
                    }
                }

                let is_pipe = matches!(operator.as_str(), "|>" | "|." | "|=");
                let right_has_line_leading = is_pipe
                    && right
                        .comments
                        .iter()
                        .any(|c| matches!(c.value, Comment::Line(_)));
                let use_vertical = if self.is_pretty() {
                    // elm-format: if either operand is multiline, break.
                    self.is_multiline(&left.value)
                        || self.is_multiline(right_expr)
                        || Self::spans_cross_lines(left.span, right.span)
                        || right_has_line_leading
                } else {
                    self.is_multiline(right_expr) || right_has_line_leading
                };
                self.write_leading_comments(&left.comments);
                let left_multiline = self.is_pretty() && self.is_multiline(&left.value);
                self.write_expr_operand(left.as_ref(), operator, true);
                if use_vertical && operator == "<|" {
                    // Left-pipe: operator stays on same line as left operand,
                    // right operand goes on a new indented line. `<|` does
                    // NOT flatten into an outer chain's indent column;
                    // cascading `<|` always adds one indent per level.
                    //
                    // When the LHS is itself multi-line, elm-format breaks
                    // `<|` to its own line at the ambient indent column
                    // rather than trailing the LHS's last line.
                    //
                    // When the RHS is itself a binop chain (`::`/`++`/`&&`/
                    // arithmetic/compose), elm-format forces that chain
                    // vertical too, aligned at the new indent column. This
                    // matches the behavior where the `<|` break cascades
                    // into any chain on the right.
                    if left_multiline {
                        self.newline_indent();
                        self.write("<|");
                    } else {
                        self.write(" <|");
                    }
                    let saved_chain =
                        std::mem::replace(&mut self.in_vertical_chain, false);
                    self.indent();
                    self.newline_indent();
                    self.write_leading_comments(&right.comments);
                    if let Some((rhead, rrest)) = flatten_as_chain(right_expr)
                        && !rrest.is_empty()
                    {
                        let first_op = rrest[0].0;
                        self.write_expr_operand(rhead, first_op, true);
                        self.indent();
                        self.in_vertical_chain = true;
                        for (op, operand) in &rrest {
                            self.newline_indent();
                            self.write(op);
                            self.write_char(' ');
                            self.write_expr_operand(operand, op, false);
                        }
                        self.dedent();
                    } else {
                        self.write_expr_inner(right_expr);
                    }
                    self.dedent();
                    self.in_vertical_chain = saved_chain;
                } else if use_vertical {
                    // Non-chain vertical binop (`==`, `<`, etc.). If the
                    // enclosing chain already bumped indent, align at that
                    // same column; otherwise bump one level.
                    let outer_chain = self.in_vertical_chain;
                    if !outer_chain {
                        self.indent();
                    }
                    // For pipe steps with leading line comments, emit them
                    // on their own indented line BEFORE the operator so that
                    // the output re-parses with the comments attached back
                    // to the same operand.
                    if right_has_line_leading {
                        for c in &right.comments {
                            if matches!(c.value, Comment::Line(_)) {
                                self.newline_indent();
                                self.write_comment(&c.value);
                            }
                        }
                        self.newline_indent();
                        self.write(operator);
                        self.write_char(' ');
                        self.write_expr_operand(right.as_ref(), operator, false);
                    } else {
                        self.newline_indent();
                        self.write(operator);
                        self.write_char(' ');
                        self.write_leading_comments(&right.comments);
                        self.write_expr_operand(right.as_ref(), operator, false);
                    }
                    if !outer_chain {
                        self.dedent();
                    }
                } else {
                    self.write_char(' ');
                    self.write(operator);
                    self.write_char(' ');
                    self.write_leading_comments(&right.comments);
                    // Use write_expr_inner for <| so block expressions aren't
                    // wrapped in redundant parens.
                    if operator == "<|" {
                        self.write_expr_inner(right_expr);
                    } else {
                        self.write_expr_operand(right.as_ref(), operator, false);
                    }
                }
            }
            Expr::IfElse {
                branches,
                else_branch,
            } => {
                self.write_if_expr(branches, else_branch);
            }
            Expr::CaseOf {
                expr: subject,
                branches,
            } => {
                self.write_case_expr(&subject.value, branches);
            }
            Expr::LetIn {
                declarations,
                body,
                trailing_comments,
            } => {
                self.write_let_expr(declarations, body, trailing_comments);
            }
            Expr::Lambda { args, body } => {
                self.write_lambda(args, body);
            }
            Expr::BinOps {
                operands_and_operators,
                final_operand,
            } => {
                if self.is_pretty() {
                    // elm-format rule: if any operand is multiline (output
                    // contains newlines), all operators break to vertical.
                    let any_ml = operands_and_operators
                        .iter()
                        .any(|(op, _)| self.is_multiline(&op.value))
                        || self.is_multiline(&final_operand.value);
                    if any_ml {
                        self.write_expr_app(&operands_and_operators[0].0.value);
                        self.indent();
                        for (i, (_operand, op)) in operands_and_operators.iter().enumerate() {
                            self.newline_indent();
                            self.write(&op.value);
                            self.write_char(' ');
                            if i + 1 < operands_and_operators.len() {
                                self.write_expr_app(&operands_and_operators[i + 1].0.value);
                            } else {
                                self.write_expr_app(&final_operand.value);
                            }
                        }
                        self.dedent();
                    } else {
                        for (operand, op) in operands_and_operators {
                            self.write_expr_app(&operand.value);
                            self.write_char(' ');
                            self.write(&op.value);
                            self.write_char(' ');
                        }
                        self.write_expr_app(&final_operand.value);
                    }
                } else {
                    for (operand, op) in operands_and_operators {
                        self.write_expr_app(&operand.value);
                        self.write_char(' ');
                        self.write(&op.value);
                        self.write_char(' ');
                    }
                    self.write_expr_app(&final_operand.value);
                }
            }
            _ => self.write_expr_app(expr),
        }
    }

    /// Write an operator operand, adding parens for precedence.
    fn write_expr_operand(&mut self, spanned: &Spanned<Expr>, parent_op: &str, is_left: bool) {
        self.expr_span_stack.push(spanned.span);
        let expr = if self.is_pretty() {
            unwrap_parens_non_block(&spanned.value)
        } else {
            &spanned.value
        };
        match expr {
            Expr::OperatorApplication { operator, .. } => {
                let parent_prec = op_precedence(parent_op);
                let child_prec = op_precedence(operator);
                let needs_parens = child_prec < parent_prec
                    || (child_prec == parent_prec
                        && ((is_left && is_right_assoc(parent_op))
                            || (!is_left && !is_right_assoc(parent_op))));
                if needs_parens {
                    let saved = std::mem::replace(&mut self.in_vertical_chain, false);
                    self.write_char('(');
                    self.write_expr_inner(expr);
                    self.write_char(')');
                    self.in_vertical_chain = saved;
                } else {
                    self.write_expr_inner(expr);
                }
            }
            _ => self.write_expr_app(expr),
        }
        self.expr_span_stack.pop();
    }

    /// Write a function application or negation.
    fn write_expr_app(&mut self, expr: &Expr) {
        // Crossing into an application/negation/atomic body means we're no
        // longer a direct operand of the outer chain. Reset the flag so
        // nested binop chains inside args establish their own indent.
        let saved = std::mem::replace(&mut self.in_vertical_chain, false);
        let result = self.write_expr_app_inner(expr);
        self.in_vertical_chain = saved;
        result
    }

    fn write_expr_app_inner(&mut self, expr: &Expr) {
        match expr {
            Expr::Application(args) => {
                // Two reasons to go vertical: (1) any individual argument is
                // itself multi-line, or (2) the source had consecutive args
                // split across different lines (the author chose vertical
                // layout even though each arg is individually simple).
                let pretty = self.is_pretty();
                // A trailing multi-line string argument is not enough to
                // force vertical layout: elm-format keeps `f a "\"\"\"..."\"\"\""`
                // on one line with the closing `"""` on its own line.
                let last_is_multiline_string = args
                    .last()
                    .map(|a| matches!(
                        unwrap_parens_non_block(&a.value),
                        Expr::Literal(Literal::MultilineString(s)) if s.contains('\n')
                    ))
                    .unwrap_or(false);
                let skip_last_for_ml = last_is_multiline_string && pretty;
                let any_arg_ml = if skip_last_for_ml {
                    args.len() > 2
                        && args
                            .iter()
                            .skip(1)
                            .take(args.len().saturating_sub(2))
                            .any(|a| self.is_multiline(&a.value))
                } else {
                    args.len() > 1 && args.iter().skip(1).any(|a| self.is_multiline(&a.value))
                };
                let source_vertical = pretty
                    && args.windows(2).any(|w| {
                        let end = w[0].span.end.line;
                        let start = w[1].span.start.line;
                        end != 0 && start != 0 && start > end
                    });
                if any_arg_ml || source_vertical {
                    self.write_application_vertical(args);
                } else {
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.write_char(' ');
                            self.write_app_arg_spanned(arg);
                        } else {
                            self.expr_span_stack.push(arg.span);
                            self.write_expr_atomic(&arg.value);
                            self.expr_span_stack.pop();
                        }
                    }
                }
            }
            Expr::Negation(inner) => {
                self.write_char('-');
                self.write_expr_atomic(&inner.value);
            }
            _ => self.write_expr_atomic(expr),
        }
    }

    /// Write an application argument in non-first position. Negation is
    /// emitted as `-inner` (no parens) to match elm-format's `f -x` style,
    /// which relies on the space-before / no-space-after rule to parse as
    /// unary negation rather than binary subtraction.
    fn write_app_arg(&mut self, expr: &Expr) {
        if matches!(expr, Expr::Negation(_)) {
            self.write_expr_app(expr);
        } else {
            self.write_expr_atomic(expr);
        }
    }

    fn write_app_arg_spanned(&mut self, spanned: &Spanned<Expr>) {
        // Emit inline block comments (captured by the parser as leading
        // comments on application args) before the arg itself. These are
        // things like `f 0x30 {- 0 -} x`. Line/multi-line block comments
        // aren't attached here so we only need to handle single-line blocks.
        for c in &spanned.comments {
            if let Comment::Block(_) = c.value {
                self.write_comment(&c.value);
                self.write_char(' ');
            }
        }
        self.expr_span_stack.push(spanned.span);
        self.write_app_arg(&spanned.value);
        self.expr_span_stack.pop();
    }

    fn write_application_vertical(&mut self, args: &[Spanned<Expr>]) {
        // elm-format's vertical layout: the first argument stays inline
        // with the function head only when the source already had an
        // argument on that line AND the argument is itself single-line.
        // All remaining arguments break onto their own indented line.
        // When the source has no argument on the function's line (the
        // author broke immediately after the function name), we respect
        // that by breaking every argument too.
        //
        // Subsequent arguments align `function_column + indent_width`.
        // When the function is at the ambient indent column this is the
        // same as ambient_indent + 1 level. When the function sits past
        // the ambient indent (e.g. inside `[ fn arg1 arg2 ...`), aligning
        // to the function's column matches elm-format.
        let fn_col = if self.is_pretty() {
            Some(self.current_column())
        } else {
            None
        };
        self.write_expr_atomic(&args[0].value);
        let saved_indent = self.indent;
        let saved_extra = self.indent_extra;
        let saved_stack = self.indent_extra_stack.clone();
        if let Some(col) = fn_col {
            let w = self.config.indent_width;
            self.indent = col / w;
            self.indent_extra = (col % w) as u32;
            self.indent_extra_stack.clear();
        }
        self.indent();
        if args.len() >= 2 {
            let first = &args[1];
            let func_end_line = args[0].span.end.line;
            let first_start_line = first.span.start.line;
            let dummy = func_end_line == 0 || first_start_line == 0;
            let first_inline_in_source = !dummy && first_start_line == func_end_line;
            if first_inline_in_source && !self.is_multiline(&first.value) {
                self.write_char(' ');
            } else {
                self.newline_indent();
            }
            self.write_app_arg_spanned(first);
            for arg in &args[2..] {
                self.newline_indent();
                self.write_app_arg_spanned(arg);
            }
        }
        self.dedent();
        self.indent = saved_indent;
        self.indent_extra = saved_extra;
        self.indent_extra_stack = saved_stack;
    }

    /// Write an expression in atomic (highest-precedence) position.
    /// Complex and block expressions get parenthesized.
    fn write_expr_atomic(&mut self, expr: &Expr) {
        // Atomic position is always a boundary: we are either a naturally
        // atomic value or about to introduce our own parens/block indent,
        // so any outer chain's indent inheritance stops here.
        let saved = std::mem::replace(&mut self.in_vertical_chain, false);
        self.write_expr_atomic_inner(expr);
        self.in_vertical_chain = saved;
    }

    fn write_expr_atomic_inner(&mut self, expr: &Expr) {
        match expr {
            Expr::Unit => self.write("()"),
            Expr::Literal(lit) => self.write_literal(lit),

            Expr::FunctionOrValue { module_name, name } => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
            }

            Expr::PrefixOperator(op) => {
                self.write_char('(');
                self.write(op);
                self.write_char(')');
            }

            Expr::Parenthesized(inner) => {
                // elm-format strips redundant Parenthesized wrappers.
                // In atomic position, strip parens when the inner expression
                // is itself atomic or is Negation/Application (which are
                // handled directly by write_expr_app).
                if self.is_pretty() && is_naturally_atomic(&inner.value) {
                    self.write_expr_atomic(&inner.value);
                } else if self.is_pretty() && matches!(inner.value, Expr::Negation(_)) {
                    self.write_expr_app(&inner.value);
                } else if self.is_pretty() && self.is_multiline(&inner.value) {
                    let is_block = matches!(
                        inner.value,
                        Expr::IfElse { .. }
                            | Expr::CaseOf { .. }
                            | Expr::LetIn { .. }
                            | Expr::Lambda { .. }
                    );
                    if is_block {
                        // For block expressions inside parens, set the indent
                        // system to match the column of `(`, so that the block's
                        // internal indent/dedent produces correct alignment.
                        let saved_indent = self.indent;
                        let saved_extra = self.indent_extra;
                        let saved_stack = self.indent_extra_stack.clone();

                        self.write_char('(');
                        let col = self.current_column();
                        let w = self.config.indent_width;
                        self.indent = col / w;
                        self.indent_extra = (col % w) as u32;

                        let start_len = self.buf.len();
                        self.write_spanned_expr(&inner);
                        let wrote_newline = self.buf[start_len..].contains('\n');

                        // Restore indent state and write `)` at `(` column.
                        self.indent = saved_indent;
                        self.indent_extra = saved_extra;
                        self.indent_extra_stack = saved_stack;
                        if wrote_newline {
                            self.newline();
                            // `(` was at col - 1, write spaces to align `)` there.
                            for _ in 0..(col - 1) {
                                self.buf.push(' ');
                            }
                        }
                        self.write_char(')');
                    } else {
                        // Multi-line non-block expr: align the continuation
                        // indent to `(` column + 1, and align `)` with `(`.
                        let saved_indent = self.indent;
                        let saved_extra = self.indent_extra;
                        let saved_stack = self.indent_extra_stack.clone();

                        self.write_char('(');
                        let col = self.current_column();
                        let w = self.config.indent_width;
                        self.indent = col / w;
                        self.indent_extra = (col % w) as u32;

                        self.write_spanned_expr(&inner);

                        self.indent = saved_indent;
                        self.indent_extra = saved_extra;
                        self.indent_extra_stack = saved_stack;
                        self.newline();
                        for _ in 0..(col - 1) {
                            self.buf.push(' ');
                        }
                        self.write_char(')');
                    }
                } else {
                    self.write_char('(');
                    self.write_spanned_expr(&inner);
                    self.write_char(')');
                }
            }

            Expr::Tuple(elems) => {
                self.write_comma_sep("( ", " )", elems);
            }

            Expr::List(elems) => {
                if elems.is_empty() {
                    self.write("[]");
                } else {
                    self.write_comma_sep_force_multi("[ ", " ]", elems);
                }
            }

            Expr::Record(fields) => {
                if fields.is_empty() {
                    self.write("{}");
                } else {
                    let any_ml = fields
                        .iter()
                        .any(|f| self.is_multiline(&f.value.value.value))
                        || (self.is_pretty()
                            && fields.iter().any(|f| f.value.trailing_comment.is_some()))
                        || (self.is_pretty() && Self::spans_multi_lines(fields));
                    if any_ml {
                        // Align commas and closing brace with the column of `{`.
                        // elm-format uses `open_col`-based alignment so that
                        // records laid out after a binary operator or on a
                        // non-indent column (e.g. `x == { a = 1\n   , b = 2 }`)
                        // keep commas visually flush with the opening brace.
                        let open_col = if self.is_pretty() {
                            Some(self.current_column())
                        } else {
                            None
                        };
                        self.write("{ ");
                        self.write_record_setter(&fields[0].value);
                        for field in &fields[1..] {
                            if let Some(col) = open_col {
                                self.newline();
                                for _ in 0..col {
                                    self.buf.push(' ');
                                }
                            } else {
                                self.newline_indent();
                            }
                            self.write(", ");
                            self.write_record_setter(&field.value);
                        }
                        if let Some(col) = open_col {
                            self.newline();
                            for _ in 0..col {
                                self.buf.push(' ');
                            }
                        } else {
                            self.newline_indent();
                        }
                        self.write("}");
                    } else {
                        self.write("{ ");
                        for (i, field) in fields.iter().enumerate() {
                            if i > 0 {
                                self.write(", ");
                            }
                            self.write_record_setter(&field.value);
                        }
                        self.write(" }");
                    }
                }
            }

            Expr::RecordUpdate { base, updates } => {
                let any_ml = updates
                    .iter()
                    .any(|f| self.is_multiline(&f.value.value.value))
                    || (self.is_pretty()
                        && updates.iter().any(|f| f.value.trailing_comment.is_some()))
                    || (self.is_pretty() && Self::spans_multi_lines(updates));
                if any_ml {
                    self.write("{ ");
                    self.write(&base.value);
                    self.indent();
                    for (i, field) in updates.iter().enumerate() {
                        self.newline_indent();
                        if i == 0 {
                            self.write("| ");
                        } else {
                            self.write(", ");
                        }
                        self.write_record_setter(&field.value);
                    }
                    self.dedent();
                    self.newline_indent();
                    self.write("}");
                } else {
                    self.write("{ ");
                    self.write(&base.value);
                    self.write(" | ");
                    for (i, field) in updates.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write_record_setter(&field.value);
                    }
                    self.write(" }");
                }
            }

            Expr::RecordAccess { record, field } => {
                self.write_expr_atomic(&record.value);
                self.write_char('.');
                self.write(&field.value);
            }

            Expr::RecordAccessFunction(name) => {
                self.write_char('.');
                self.write(name);
            }

            Expr::GLSLExpression(src) => {
                self.write("[glsl|");
                self.write(src);
                self.write("|]");
            }

            // Non-block complex expressions: simple inline parens.
            Expr::OperatorApplication { .. }
            | Expr::Application(_)
            | Expr::Negation(_)
            | Expr::BinOps { .. } => {
                self.write_char('(');
                self.write_expr_inner(expr);
                self.write_char(')');
            }

            // Block expressions in atomic position: parenthesized.
            Expr::IfElse { .. }
            | Expr::CaseOf { .. }
            | Expr::LetIn { .. }
            | Expr::Lambda { .. } => {
                if self.is_pretty() {
                    let saved_indent = self.indent;
                    let saved_extra = self.indent_extra;
                    let saved_stack = self.indent_extra_stack.clone();

                    self.write_char('(');
                    let col = self.current_column();
                    let w = self.config.indent_width;
                    self.indent = col / w;
                    self.indent_extra = (col % w) as u32;

                    let start_len = self.buf.len();
                    self.write_expr_inner(expr);
                    let wrote_newline = self.buf[start_len..].contains('\n');

                    self.indent = saved_indent;
                    self.indent_extra = saved_extra;
                    self.indent_extra_stack = saved_stack;
                    if wrote_newline {
                        self.newline();
                        for _ in 0..(col - 1) {
                            self.buf.push(' ');
                        }
                    }
                    self.write_char(')');
                } else {
                    self.write_char('(');
                    self.write_expr_inner(expr);
                    self.write_char(')');
                }
            }
        }
    }

    /// Write a comma-separated list of expressions with adaptive layout.
    /// Uses single-line when all elements are single-line, multi-line otherwise.
    fn write_comma_sep(&mut self, open: &str, close: &str, elems: &[Spanned<Expr>]) {
        self.write_comma_sep_inner(open, close, elems, false);
    }

    /// Like `write_comma_sep` but force multi-line when the enclosing span
    /// crosses source lines (used for lists where elm-format preserves
    /// user-chosen multi-line layout even with single-element lists).
    fn write_comma_sep_force_multi(
        &mut self,
        open: &str,
        close: &str,
        elems: &[Spanned<Expr>],
    ) {
        self.write_comma_sep_inner(open, close, elems, true);
    }

    fn write_comma_sep_inner(
        &mut self,
        open: &str,
        close: &str,
        elems: &[Spanned<Expr>],
        respect_outer_multi_line: bool,
    ) {
        let open_col = self.current_column();
        let standard_indent =
            self.indent * self.config.indent_width + self.indent_extra as usize;
        let at_standard_indent = (open_col as usize) == standard_indent;
        let outer_multi_line = respect_outer_multi_line
            && self.is_pretty()
            && at_standard_indent
            && self
                .current_expr_span()
                .map(|s| s.end.line > s.start.line && s.start.line != 0)
                .unwrap_or(false);
        let any_multiline = elems.iter().any(|e| self.is_multiline(&e.value))
            || (self.is_pretty() && Self::spans_multi_lines(elems))
            || outer_multi_line;
        if any_multiline && self.is_pretty() {
            // elm-format style: first element on same line as open bracket,
            // subsequent elements aligned with ", " prefix at same indent.
            // Set indent_extra = 2 so block expressions (if-else, let-in)
            // inside elements align else/in with the element content after
            // the "[ " or ", " prefix.
            //
            // Capture the column of the opening bracket so commas and the
            // closing bracket align with it, even when the list is written
            // as the RHS of an operator (e.g. `xs ++ [ ...`) where the `[`
            // is not at the current indent column.
            let open_col = self.current_column();
            let standard_indent =
                self.indent * self.config.indent_width + self.indent_extra as usize;
            // When the list is written inline on a chain line (open_col >
            // standard_indent), elm-format bumps the block indent by one
            // so that block expressions (if/else, case-of) inside an element
            // are indented two levels past the chain line, not just one.
            let bump_indent = open_col > standard_indent;
            let saved_extra = self.indent_extra;
            self.write(open);
            if bump_indent {
                self.indent();
            }
            self.indent_extra = saved_extra + 2;
            self.write_spanned_expr(&elems[0]);
            self.indent_extra = saved_extra;
            if bump_indent {
                self.dedent();
            }
            for elem in &elems[1..] {
                self.newline();
                for _ in 0..open_col {
                    self.buf.push(' ');
                }
                self.write(", ");
                if bump_indent {
                    self.indent();
                }
                self.indent_extra = saved_extra + 2;
                self.write_spanned_expr(&elem);
                self.indent_extra = saved_extra;
                if bump_indent {
                    self.dedent();
                }
            }
            self.newline();
            for _ in 0..open_col {
                self.buf.push(' ');
            }
            self.write(close.trim_start());
        } else if any_multiline {
            // Compact mode: one element per indented line.
            self.write(open.trim_end());
            self.indent();
            for (i, elem) in elems.iter().enumerate() {
                self.newline_indent();
                if i == 0 {
                    self.write("  ");
                } else {
                    self.write(", ");
                }
                self.write_spanned_expr(&elem);
            }
            self.newline_indent();
            self.write(close.trim_start());
            self.dedent();
        } else {
            // Single-line: all on one line.
            self.write(open);
            for (i, elem) in elems.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write_spanned_expr(&elem);
            }
            self.write(close);
        }
    }

    fn write_record_setter(&mut self, setter: &RecordSetter) {
        self.write(&setter.field.value);
        // In pretty mode, preserve source layout: if the value started on a
        // line after the field name, keep the break. elm-format respects this.
        let source_was_multiline = self.is_pretty()
            && setter.field.span.end.line < setter.value.span.start.line;
        if self.is_multiline(&setter.value.value) || source_was_multiline {
            self.write(" =");
            self.indent();
            self.newline_indent();
            self.write_spanned_expr(&setter.value);
            self.dedent();
        } else {
            self.write(" = ");
            self.write_spanned_expr(&setter.value);
        }
        if let Some(trailing) = &setter.trailing_comment
            && self.is_pretty()
        {
            self.write_char(' ');
            self.write_comment(&trailing.value);
        }
    }

    /// Write `<keyword> <cond> then`, breaking to a multi-line layout when
    /// the condition itself spans multiple lines. elm-format's convention
    /// when the cond is multi-line:
    ///     if
    ///         <cond>
    ///     then
    fn write_if_condition(&mut self, keyword: &str, cond: &Spanned<Expr>) {
        let multiline_cond = self.is_pretty() && self.is_multiline(&cond.value);
        if multiline_cond {
            self.write(keyword);
            self.indent();
            self.newline_indent();
            self.write_spanned_expr(&cond);
            self.dedent();
            self.newline_indent();
            self.write("then");
        } else {
            self.write(keyword);
            self.write_char(' ');
            self.write_spanned_expr(&cond);
            self.write(" then");
        }
    }

    fn write_if_expr(
        &mut self,
        branches: &[(Spanned<Expr>, Spanned<Expr>)],
        else_branch: &Spanned<Expr>,
    ) {
        // Single-line when all branches are simple non-block expressions.
        // elm-format always uses multiline, so skip single-line in pretty mode.
        let all_simple = !self.is_pretty()
            && branches.len() == 1
            && branches
                .iter()
                .all(|(c, b)| !self.is_multiline(&c.value) && !self.is_multiline(&b.value))
            && !self.is_multiline(&else_branch.value)
            && branches.iter().all(|(_, b)| b.comments.is_empty())
            && else_branch.comments.is_empty();

        if all_simple {
            let (cond, body) = &branches[0];
            self.write("if ");
            self.write_spanned_expr(&cond);
            self.write(" then ");
            self.write_spanned_expr(&body);
            self.write(" else ");
            self.write_spanned_expr(&else_branch);
        } else if self.is_pretty() {
            // In pretty mode, use column-based indentation so that
            // branches are always indented relative to the `if` keyword,
            // regardless of the current indent_extra state.
            let if_col = self.current_column();
            let saved_indent = self.indent;
            let saved_extra = self.indent_extra;
            let saved_stack = self.indent_extra_stack.clone();
            let w = self.config.indent_width;
            // Set indent to match the `if` keyword column.
            self.indent = if_col / w;
            self.indent_extra = (if_col % w) as u32;

            for (i, (cond, body)) in branches.iter().enumerate() {
                let keyword = if i == 0 { "if" } else { "else if" };
                self.write_if_condition(keyword, cond);
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&body.comments);
                self.write_spanned_expr(&body);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            // Flatten nested if-else into else-if chains.
            if let Expr::IfElse {
                branches: nested_branches,
                else_branch: nested_else,
            } = &else_branch.value
            {
                for (cond, body) in nested_branches {
                    self.write_if_condition("else if", cond);
                    self.indent();
                    self.newline_indent();
                    self.write_leading_comments(&body.comments);
                    self.write_spanned_expr(&body);
                    self.dedent();
                    self.newline();
                    self.newline_indent();
                }
                self.write_if_else_tail(nested_else);
            } else {
                self.write("else");
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&else_branch.comments);
                self.write_spanned_expr(&else_branch);
                self.dedent();
            }

            // Restore indent state.
            self.indent = saved_indent;
            self.indent_extra = saved_extra;
            self.indent_extra_stack = saved_stack;
        } else {
            for (i, (cond, body)) in branches.iter().enumerate() {
                if i == 0 {
                    self.write("if ");
                } else {
                    self.write("else if ");
                }
                self.write_spanned_expr(&cond);
                self.write(" then");
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&body.comments);
                self.write_spanned_expr(&body);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            self.write("else");
            self.indent();
            self.newline_indent();
            self.write_leading_comments(&else_branch.comments);
            self.write_spanned_expr(&else_branch);
            self.dedent();
        }
    }

    /// Helper for flattening nested if-else in ElmFormat mode.
    fn write_if_else_tail(&mut self, else_branch: &Spanned<Expr>) {
        if let Expr::IfElse {
            branches: nested_branches,
            else_branch: nested_else,
        } = &else_branch.value
        {
            for (cond, body) in nested_branches {
                self.write("else if ");
                self.write_spanned_expr(&cond);
                self.write(" then");
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&body.comments);
                self.write_spanned_expr(&body);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            self.write_if_else_tail(nested_else);
        } else {
            self.write("else");
            self.indent();
            self.newline_indent();
            self.write_leading_comments(&else_branch.comments);
            self.write_spanned_expr(&else_branch);
            self.dedent();
        }
    }

    fn write_case_expr(&mut self, subject: &Expr, branches: &[CaseBranch]) {
        if self.is_pretty() {
            let case_col = self.current_column();
            let saved_indent = self.indent;
            let saved_extra = self.indent_extra;
            let saved_stack = self.indent_extra_stack.clone();
            let w = self.config.indent_width;
            self.indent = case_col / w;
            self.indent_extra = (case_col % w) as u32;
            // When the scrutinee forces multi-line output, elm-format uses
            // the "hanging" form: bare "case", indented subject, and bare
            // "of". This covers control-flow compounds (if/let/case) and
            // any other expression whose rendered form spans multiple
            // lines (e.g. a multi-arg application with nested args).
            let hanging = matches!(
                subject,
                Expr::IfElse { .. } | Expr::LetIn { .. } | Expr::CaseOf { .. }
            ) || self.is_multiline(subject);
            if hanging {
                self.write("case");
                self.indent();
                self.newline_indent();
                self.write_expr(subject);
                self.dedent();
                self.newline_indent();
                self.write("of");
            } else {
                self.write("case ");
                self.write_expr(subject);
                self.write(" of");
            }
            self.indent();
            for (i, branch) in branches.iter().enumerate() {
                if i > 0 {
                    self.newline();
                }
                self.newline_indent();
                self.write_leading_comments(&branch.pattern.comments);
                self.write_pattern(&branch.pattern.value);
                self.write(" ->");
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&branch.body.comments);
                self.write_spanned_expr(&branch.body);
                self.dedent();
            }
            self.dedent();
            self.indent = saved_indent;
            self.indent_extra = saved_extra;
            self.indent_extra_stack = saved_stack;
            return;
        }
        self.write("case ");
        self.write_expr(subject);
        self.write(" of");
        self.indent();
        for (i, branch) in branches.iter().enumerate() {
            // elm-format puts a blank line between case branches.
            if self.is_pretty() && i > 0 {
                self.newline();
            }
            self.newline_indent();
            // Emit leading comments on the branch pattern.
            self.write_leading_comments(&branch.pattern.comments);
            self.write_pattern(&branch.pattern.value);
            self.write(" ->");
            self.indent();
            self.newline_indent();
            // Emit leading comments on the branch body.
            self.write_leading_comments(&branch.body.comments);
            self.write_spanned_expr(&branch.body);
            self.dedent();
        }
        self.dedent();
    }

    fn write_let_expr(
        &mut self,
        declarations: &[Spanned<LetDeclaration>],
        body: &Spanned<Expr>,
        trailing_comments: &[Spanned<Comment>],
    ) {
        if self.is_pretty() {
            let let_col = self.current_column();
            let saved_indent = self.indent;
            let saved_extra = self.indent_extra;
            let saved_stack = self.indent_extra_stack.clone();
            let w = self.config.indent_width;
            self.indent = let_col / w;
            self.indent_extra = (let_col % w) as u32;

            self.write("let");
            self.indent();
            for (i, decl) in declarations.iter().enumerate() {
                if i > 0 {
                    self.newline();
                }
                self.newline_indent();
                self.write_leading_comments(&decl.comments);
                self.write_let_declaration(&decl.value);
            }
            if !trailing_comments.is_empty() {
                self.newline();
                for c in trailing_comments {
                    self.newline_indent();
                    self.write_comment(&c.value);
                }
            }
            self.dedent();
            self.newline_indent();
            self.write("in");
            self.newline_indent();
            self.write_leading_comments(&body.comments);
            self.write_spanned_expr(&body);

            self.indent = saved_indent;
            self.indent_extra = saved_extra;
            self.indent_extra_stack = saved_stack;
            return;
        }
        self.write("let");
        self.indent();
        for decl in declarations {
            self.newline_indent();
            self.write_leading_comments(&decl.comments);
            self.write_let_declaration(&decl.value);
        }
        if !trailing_comments.is_empty() {
            for c in trailing_comments {
                self.newline_indent();
                self.write_comment(&c.value);
            }
        }
        self.dedent();
        self.newline_indent();
        self.write("in");
        self.newline_indent();
        self.write_leading_comments(&body.comments);
        self.write_spanned_expr(&body);
    }

    fn write_let_declaration(&mut self, decl: &LetDeclaration) {
        match decl {
            LetDeclaration::Function(func) => {
                if let Some(sig) = &func.signature {
                    self.write_signature(&sig.value);
                    self.newline_indent();
                }
                self.write_function_impl(&func.declaration.value);
            }
            LetDeclaration::Destructuring { pattern, body } => {
                self.write_pattern(&pattern.value);
                self.write(" =");
                self.indent();
                self.newline_indent();
                self.write_leading_comments(&body.comments);
                self.write_spanned_expr(&body);
                self.dedent();
            }
        }
    }

    fn write_lambda(&mut self, args: &[Spanned<Pattern>], body: &Spanned<Expr>) {
        self.write("\\");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.write_char(' ');
            }
            self.write_pattern_atomic(&arg.value);
        }
        let body_broken_in_source = self.is_pretty()
            && match args.last() {
                Some(last) => {
                    last.span.end.line != 0
                        && body.span.start.line != 0
                        && body.span.start.line > last.span.end.line
                }
                None => false,
            };
        if self.is_multiline(&body.value) || body_broken_in_source {
            self.write(" ->");
            self.indent();
            self.newline_indent();
            self.write_spanned_expr(&body);
            self.dedent();
        } else {
            self.write(" -> ");
            self.write_spanned_expr(&body);
        }
    }

    // ── Literals ─────────────────────────────────────────────────────

    fn write_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Char(c) => {
                self.write_char('\'');
                // In single-quoted char literals, double quotes don't need escaping.
                match c {
                    '"' => self.write_char('"'),
                    _ => self.write_escaped_char(*c),
                }
                self.write_char('\'');
            }
            Literal::String(s) => {
                self.write_char('"');
                self.write_escaped_string(s);
                self.write_char('"');
            }
            Literal::MultilineString(s) => {
                self.write("\"\"\"");
                self.write(s);
                self.write("\"\"\"");
            }
            Literal::Int(n) => self.write(&n.to_string()),
            Literal::Hex(n) => {
                if self.is_pretty() {
                    // Match elm-format's hex width normalization:
                    // pad to 2, 4, 8, or 16 digits based on magnitude.
                    let abs = n.unsigned_abs();
                    let prefix = if *n < 0 { "-0x" } else { "0x" };
                    if abs <= 0xFF {
                        self.write(&format!("{prefix}{abs:02X}"));
                    } else if abs <= 0xFFFF {
                        self.write(&format!("{prefix}{abs:04X}"));
                    } else if abs <= 0xFFFF_FFFF {
                        self.write(&format!("{prefix}{abs:08X}"));
                    } else {
                        self.write(&format!("{prefix}{abs:016X}"));
                    }
                } else {
                    self.write("0x");
                    self.write(&format!("{n:02X}"));
                }
            }
            Literal::Float(f, lexeme) => {
                // In ElmFormat mode, prefer the original source lexeme when it
                // carried scientific notation. Rust's `f64::to_string` always
                // normalizes to shortest round-trip decimal, losing forms like
                // `3.969683028665376e1`. elm-format preserves the source form,
                // so we do too. Compact mode ignores the lexeme and uses the
                // normalized numeric form.
                if self.is_pretty()
                    && let Some(lex) = lexeme
                    && lex.contains(['e', 'E'])
                {
                    let lower = lex.replace('E', "e");
                    let e_pos = lower.find('e').expect("just checked for 'e'/'E'");
                    let mantissa = &lower[..e_pos];
                    let exp = &lower[e_pos..];
                    if mantissa.contains('.') {
                        self.write(&lower);
                    } else {
                        self.write(mantissa);
                        self.write(".0");
                        self.write(exp);
                    }
                    return;
                }
                let decimal = f.to_string();
                // Rust's Display impl picks between decimal and scientific.
                // For very small / very large magnitudes it can produce a long
                // stream of zeros (`0.000...01`, `6022...000`); elm-format
                // keeps the literal in scientific form in those cases. Fall
                // back to `{:e}` when the decimal form has a long run of
                // zeros.
                let has_long_zero_run = {
                    let bytes = decimal.as_bytes();
                    let mut run = 0usize;
                    let mut max = 0usize;
                    for &b in bytes {
                        if b == b'0' {
                            run += 1;
                            if run > max {
                                max = run;
                            }
                        } else {
                            run = 0;
                        }
                    }
                    max >= 18
                };
                let s = if has_long_zero_run {
                    format!("{:e}", f)
                } else {
                    decimal
                };
                if s.contains('.') {
                    self.write(&s);
                } else if let Some(e_pos) = s.find(['e', 'E']) {
                    // Scientific form without a dot (e.g. `1e-42`): elm-format
                    // inserts `.0` before the exponent to make it `1.0e-42`.
                    self.write(&s[..e_pos]);
                    self.write(".0");
                    self.write(&s[e_pos..]);
                } else {
                    self.write(&s);
                    self.write(".0");
                }
            }
        }
    }

    fn write_escaped_char(&mut self, c: char) {
        match c {
            '\n' => self.write("\\n"),
            '\t' => self.write("\\t"),
            '\\' => self.write("\\\\"),
            '\'' => self.write("\\'"),
            '"' => self.write("\\\""),
            c if should_unicode_escape(c) => {
                // \r and other control chars use \u{XXXX} form (Elm has no \r escape)
                self.write(&format!("\\u{{{:04X}}}", c as u32));
            }
            c => self.write_char(c),
        }
    }

    fn write_escaped_string(&mut self, s: &str) {
        for c in s.chars() {
            // In double-quoted strings, single quotes don't need escaping.
            match c {
                '\'' => self.write_char('\''),
                _ => self.write_escaped_char(c),
            }
        }
    }
}

/// Whether a char should be emitted as a `\u{XXXX}` escape in string/char
/// literals. Matches elm-format: escape control chars, non-ASCII whitespace
/// (NBSP, en quad, etc.), invisible format chars, BOM, and unassigned
/// codepoints in ranges Haskell's `Char.isPrint` rejects.
pub(super) fn should_unicode_escape(c: char) -> bool {
    if c.is_control() {
        return true;
    }
    let cp = c as u32;
    matches!(
        cp,
        0x00A0      // NBSP
        | 0x1680    // OGHAM SPACE MARK
        | 0x2000..=0x200F   // various spaces + zero-width + directional
        | 0x2028..=0x202F   // line/paragraph sep, bidi, narrow nbsp
        | 0x205F..=0x206F   // medium math space, word joiner, invisible format
        | 0x2E5E..=0x2E7F   // unassigned tail of Supplemental Punctuation block
        | 0x3000    // IDEOGRAPHIC SPACE
        | 0xFEFF    // BOM / zero-width non-breaking space
    )
}

// reindent_block_comment moved to src/print/block_comment.rs

// ── Multiline detection ──────────────────────────────────────────────
//
// Inspired by elm-format's `allSingles` / `Box` model: eagerly determine
// whether an expression would produce multi-line output. Block expressions
// (case/if/let/lambda) are always multi-line. Containers are multi-line
// if any child is multi-line.

// ── Standalone helpers ───────────────────────────────────────────────

fn op_precedence(op: &str) -> u8 {
    match op {
        "<|" | "|>" => 0,
        "||" => 2,
        "&&" => 3,
        "==" | "/=" | "<" | ">" | "<=" | ">=" => 4,
        "::" | "++" => 5,
        "+" | "-" => 6,
        "*" | "/" | "//" => 7,
        "^" => 8,
        "<<" | ">>" => 9,
        _ => 9,
    }
}

fn is_right_assoc(op: &str) -> bool {
    matches!(op, "<|" | "||" | "&&" | "::" | "++" | "^" | ">>")
}

// Doc-comment / markdown helpers moved to src/print/doc_markdown.rs

/// Get the name of an exposed item for matching against `@docs` directives.
fn exposed_item_name(item: &ExposedItem) -> String {
    match item {
        ExposedItem::Function(name) | ExposedItem::TypeOrAlias(name) => name.clone(),
        ExposedItem::TypeExpose { name, .. } => name.clone(),
        ExposedItem::Infix(op) => format!("({})", op),
    }
}

/// Get a sort key for an exposed item, matching elm-format's alphabetical sort.
/// elm-format sorts by the string representation of each item.
fn exposed_item_sort_key(item: &ExposedItem) -> String {
    match item {
        ExposedItem::Function(name) => name.clone(),
        ExposedItem::TypeOrAlias(name) => name.clone(),
        ExposedItem::TypeExpose { name, .. } => name.clone(),
        ExposedItem::Infix(op) => format!("({})", op),
    }
}

fn exposed_item_to_string(item: &ExposedItem) -> String {
    match item {
        ExposedItem::Function(name) => name.clone(),
        ExposedItem::TypeOrAlias(name) => name.clone(),
        ExposedItem::TypeExpose { name, open } => {
            if open.is_some() {
                format!("{name}(..)")
            } else {
                name.clone()
            }
        }
        ExposedItem::Infix(op) => format!("({op})"),
    }
}

/// Check if an expression is naturally atomic (doesn't need parens in any position).
fn is_naturally_atomic(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Unit
            | Expr::Literal(_)
            | Expr::FunctionOrValue { .. }
            | Expr::PrefixOperator(_)
            | Expr::Parenthesized(_)
            | Expr::Tuple(_)
            | Expr::List(_)
            | Expr::Record(_)
            | Expr::RecordUpdate { .. }
            | Expr::RecordAccess { .. }
            | Expr::RecordAccessFunction(_)
            | Expr::GLSLExpression(_)
    )
}

/// Unwrap one layer of `Parenthesized` from an expression.
/// Returns the inner expression if it is parenthesized, or the original expression otherwise.
fn unwrap_parens(expr: &Expr) -> &Expr {
    match expr {
        Expr::Parenthesized(inner) => &inner.value,
        other => other,
    }
}

/// Unwrap `Parenthesized` when the inner expression doesn't need parens
/// at the operator-operand or expression level. elm-format strips redundant
/// parens around non-block, non-operator expressions in these positions.
fn unwrap_parens_non_block(expr: &Expr) -> &Expr {
    match expr {
        Expr::Parenthesized(inner)
            if !matches!(
                inner.value,
                Expr::OperatorApplication { .. }
                    | Expr::BinOps { .. }
                    | Expr::IfElse { .. }
                    | Expr::CaseOf { .. }
                    | Expr::LetIn { .. }
                    | Expr::Lambda { .. }
            ) =>
        {
            &inner.value
        }
        other => other,
    }
}

// Operator-chain flatteners moved to src/print/pipeline_layout.rs.

/// Convenience function: print an `ElmModule` to a string with default config.
///
/// Uses `PrintStyle::Compact` for round-trip-safe output.
pub fn print(module: &ElmModule) -> String {
    Printer::new(PrintConfig::default()).print_module(module)
}

/// Pretty-print an `ElmModule` using elm-format-style line breaking.
///
/// Pipelines (`|>`), records, and lists with multiple entries are always
/// multiline. Ideal for code generation where readability matters.
pub fn pretty_print(module: &ElmModule) -> String {
    Printer::new(PrintConfig {
        style: PrintStyle::ElmFormat,
        ..PrintConfig::default()
    })
    .print_module(module)
}
