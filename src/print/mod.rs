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
            Expr::Lambda { body, .. } => {
                self.is_multiline(&body.value)
                    || (self.is_pretty() && Self::span_crosses_lines(body.span))
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
                    let reindented = reindent_block_comment(text, brace_col);
                    // elm-format normalizes `{-- foo...` (block content
                    // starting with "- ") by dropping the space after the
                    // leading dash, keeping `{--` as a single marker.
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

            let line_start = self.buf.rfind('\n').map_or(0, |p| p + 1);
            let current_col = self.buf.len() - line_start;
            let source_was_multiline = Self::spans_multi_lines(items);
            if !source_was_multiline && current_col + single_line.len() <= 120 {
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
                self.write_expr(&body.value);
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
            self.write_type_multiline(&sig.type_annotation.value);
            self.dedent();
        } else {
            self.write(" : ");
            self.write_type(&sig.type_annotation.value);
        }
    }

    /// True when the type annotation's span crosses source lines.
    fn type_ann_spans_multi_lines(sp: &Spanned<TypeAnnotation>) -> bool {
        sp.span.start.line != 0
            && sp.span.end.line != 0
            && sp.span.end.line > sp.span.start.line
    }

    /// Write a type annotation broken across lines: function arrows on their
    /// own lines, record arguments expanded multi-line. Matches elm-format's
    /// layout when the source annotation spans multiple lines.
    fn write_type_multiline(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::FunctionType { from, to } => {
                self.write_type_arm_multiline(&from.value);
                let mut cur: &TypeAnnotation = &to.value;
                loop {
                    match cur {
                        TypeAnnotation::FunctionType { from, to } => {
                            self.write_function_arm_arrow(&from.value);
                            cur = &to.value;
                        }
                        _ => {
                            self.write_function_arm_arrow(cur);
                            break;
                        }
                    }
                }
            }
            TypeAnnotation::Record(fields) if fields.len() >= 2 => {
                self.write_record_type_fields_multiline(fields, None);
            }
            _ => self.write_type(ty),
        }
    }

    /// Emit one `-> <arm>` step of a multi-line function type. When the arm
    /// itself will expand onto multiple lines (e.g. a 2+ field record),
    /// elm-format puts the arrow alone on its own line and indents the arm
    /// below it; otherwise the arrow and arm sit on the same line.
    fn write_function_arm_arrow(&mut self, arm: &TypeAnnotation) {
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

    fn type_arm_is_multiline(ty: &TypeAnnotation) -> bool {
        matches!(ty, TypeAnnotation::Record(fields) if fields.len() >= 2)
    }

    /// Write a single arm of a multi-line function type. Records expand
    /// multi-line; other forms stay on one line (parenthesized if needed).
    fn write_type_arm_multiline(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::Record(fields) if fields.len() >= 2 => {
                self.write_record_type_fields_multiline(fields, None);
            }
            _ => self.write_type_non_arrow(ty),
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
            self.write_type_pretty_toplevel(&alias.type_annotation.value);
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
            self.newline_indent();
            if i == 0 {
                self.write("= ");
            } else {
                self.write("| ");
            }
            self.write_value_constructor(&ctor.value);
        }
        self.dedent();
    }

    fn write_value_constructor(&mut self, ctor: &ValueConstructor) {
        self.write(&ctor.name.value);
        // Only force multi-line constructor layout when an arg's printed
        // form will genuinely span multiple lines. Function types,
        // tuples, and applied types are always printed single-line via
        // `write_type_atomic`, so forcing a break after the ctor name
        // would be rewrapped by elm-format into a single line, breaking
        // round-trip stability. Records with >=2 fields are the only
        // arg kind that reliably print multi-line.
        let any_multiline = self.is_pretty()
            && ctor.args.iter().any(|a| {
                Self::type_ann_spans_multi_lines(a)
                    && matches!(&a.value, TypeAnnotation::Record(fs) if fs.len() >= 2)
            });
        if any_multiline {
            self.indent();
            for arg in &ctor.args {
                self.newline_indent();
                self.write_ctor_arg_multiline(&arg.value);
            }
            self.dedent();
        } else {
            for arg in &ctor.args {
                self.write_char(' ');
                self.write_type_atomic(&arg.value);
            }
        }
    }

    fn write_ctor_arg_multiline(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::Record(fields) if fields.len() >= 2 => {
                self.write_record_type_fields_multiline(fields, None);
            }
            _ => self.write_type_atomic(ty),
        }
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
                self.write_type(&to.value);
            }
            _ => self.write_type_non_arrow(ty),
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
                    self.newline_indent();
                    self.write(", ");
                }
                self.write(&field.value.name.value);
                self.write(" : ");
                self.write_type(&field.value.type_annotation.value);
            }
            self.dedent();
        } else {
            self.write("{ ");
            self.write(&fields[0].value.name.value);
            self.write(" : ");
            self.write_type(&fields[0].value.type_annotation.value);
            for field in &fields[1..] {
                self.newline_indent();
                self.write(", ");
                self.write(&field.value.name.value);
                self.write(" : ");
                self.write_type(&field.value.type_annotation.value);
            }
        }
        self.newline_indent();
        self.write("}");
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

    /// Emit leading comments attached to a node.
    fn write_leading_comments(&mut self, comments: &[Spanned<Comment>]) {
        for c in comments {
            self.write_comment(&c.value);
            self.newline();
            self.write_indent();
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
                // right side of `<|` since `<|` has the lowest precedence and
                // is right-associative, so parens are never required there.
                let right_expr = if self.is_pretty() && operator == "<|" {
                    unwrap_parens(&right.value)
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
                    let any_ml = self.is_multiline(head)
                        || rest.iter().any(|(_, op)| self.is_multiline(op))
                        || Self::spans_cross_lines(left.span, right.span);
                    if any_ml {
                        // When the pipe chain goes vertical and its head is
                        // itself a binop chain (e.g. `a ++ b |> f`), elm-
                        // format flattens the head's operators into the same
                        // vertical layout at the same indent column.
                        let (real_head, head_rest) = match flatten_as_chain(head) {
                            Some(x) => x,
                            None => (head, Vec::new()),
                        };
                        let first_op = head_rest
                            .first()
                            .map(|(op, _)| *op)
                            .unwrap_or(operator.as_str());
                        let outer_chain = self.in_vertical_chain;
                        self.write_expr_operand(real_head, first_op, true);
                        if !outer_chain {
                            self.indent();
                        }
                        self.in_vertical_chain = true;
                        for (op, operand) in head_rest.iter().chain(rest.iter()) {
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
                    let any_ml = self.is_multiline(head)
                        || rest.iter().any(|(_, e)| self.is_multiline(e))
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
                    let any_ml = chain.iter().any(|op| self.is_multiline(op))
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

                // Arithmetic operators `+` `-` `*` `/` `//` sit at two
                // precedences (6 and 7). Mixed chains (`a - k * p`) parse
                // with the higher-precedence op nested inside, but
                // elm-format lays the whole thing out as a single vertical
                // sequence with every operator at the same indent column.
                if self.is_pretty()
                    && matches!(operator.as_str(), "+" | "-" | "*" | "/" | "//")
                    && let Some((head, rest)) = flatten_mixed_arithmetic_chain(expr)
                {
                    let any_ml = self.is_multiline(head)
                        || rest.iter().any(|(_, op)| self.is_multiline(op))
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
                    let any_ml = self.is_multiline(head)
                        || rest.iter().any(|(_, e)| self.is_multiline(e))
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

                let use_vertical = if self.is_pretty() {
                    // elm-format: if either operand is multiline, break.
                    self.is_multiline(&left.value)
                        || self.is_multiline(right_expr)
                        || Self::spans_cross_lines(left.span, right.span)
                } else {
                    self.is_multiline(right_expr)
                };
                self.write_leading_comments(&left.comments);
                let left_multiline = self.is_pretty() && self.is_multiline(&left.value);
                self.write_expr_operand(&left.value, operator, true);
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
                    self.newline_indent();
                    self.write(operator);
                    self.write_char(' ');
                    self.write_leading_comments(&right.comments);
                    self.write_expr_operand(right_expr, operator, false);
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
                        self.write_expr_operand(right_expr, operator, false);
                    }
                }
            }
            Expr::IfElse {
                branches,
                else_branch,
            } => {
                self.write_if_expr(branches, &else_branch.value);
            }
            Expr::CaseOf {
                expr: subject,
                branches,
            } => {
                self.write_case_expr(&subject.value, branches);
            }
            Expr::LetIn { declarations, body } => {
                self.write_let_expr(declarations, &body.value);
            }
            Expr::Lambda { args, body } => {
                self.write_lambda(args, &body.value);
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
    fn write_expr_operand(&mut self, expr: &Expr, parent_op: &str, is_left: bool) {
        let expr = if self.is_pretty() {
            unwrap_parens_non_block(expr)
        } else {
            expr
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
                let any_arg_ml =
                    args.len() > 1 && args.iter().skip(1).any(|a| self.is_multiline(&a.value));
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
                            self.write_app_arg(&arg.value);
                        } else {
                            self.write_expr_atomic(&arg.value);
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
            self.write_app_arg(&first.value);
            for arg in &args[2..] {
                self.newline_indent();
                self.write_app_arg(&arg.value);
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

                        self.write_expr(&inner.value);

                        // Restore indent state and write `)` at `(` column.
                        self.indent = saved_indent;
                        self.indent_extra = saved_extra;
                        self.indent_extra_stack = saved_stack;
                        self.newline();
                        // `(` was at col - 1, write spaces to align `)` there.
                        for _ in 0..(col - 1) {
                            self.buf.push(' ');
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

                        self.write_expr(&inner.value);

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
                    self.write_expr(&inner.value);
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
                    self.write_comma_sep("[ ", " ]", elems);
                }
            }

            Expr::Record(fields) => {
                if fields.is_empty() {
                    self.write("{}");
                } else {
                    let any_ml = fields
                        .iter()
                        .any(|f| self.is_multiline(&f.value.value.value))
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

                    self.write_expr_inner(expr);

                    self.indent = saved_indent;
                    self.indent_extra = saved_extra;
                    self.indent_extra_stack = saved_stack;
                    self.newline();
                    for _ in 0..(col - 1) {
                        self.buf.push(' ');
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
        let any_multiline = elems.iter().any(|e| self.is_multiline(&e.value))
            || (self.is_pretty() && Self::spans_multi_lines(elems));
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
            self.write_expr(&elems[0].value);
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
                self.write_expr(&elem.value);
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
                self.write_expr(&elem.value);
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
                self.write_expr(&elem.value);
            }
            self.write(close);
        }
    }

    fn write_record_setter(&mut self, setter: &RecordSetter) {
        self.write(&setter.field.value);
        if self.is_multiline(&setter.value.value) {
            self.write(" =");
            self.indent();
            self.newline_indent();
            self.write_expr(&setter.value.value);
            self.dedent();
        } else {
            self.write(" = ");
            self.write_expr(&setter.value.value);
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
            self.write_expr(&cond.value);
            self.dedent();
            self.newline_indent();
            self.write("then");
        } else {
            self.write(keyword);
            self.write_char(' ');
            self.write_expr(&cond.value);
            self.write(" then");
        }
    }

    fn write_if_expr(&mut self, branches: &[(Spanned<Expr>, Spanned<Expr>)], else_branch: &Expr) {
        // Single-line when all branches are simple non-block expressions.
        // elm-format always uses multiline, so skip single-line in pretty mode.
        let all_simple = !self.is_pretty()
            && branches.len() == 1
            && branches
                .iter()
                .all(|(c, b)| !self.is_multiline(&c.value) && !self.is_multiline(&b.value))
            && !self.is_multiline(else_branch);

        if all_simple {
            let (cond, body) = &branches[0];
            self.write("if ");
            self.write_expr(&cond.value);
            self.write(" then ");
            self.write_expr(&body.value);
            self.write(" else ");
            self.write_expr(else_branch);
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
                self.write_expr(&body.value);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            // Flatten nested if-else into else-if chains.
            if let Expr::IfElse {
                branches: nested_branches,
                else_branch: nested_else,
            } = else_branch
            {
                for (cond, body) in nested_branches {
                    self.write_if_condition("else if", cond);
                    self.indent();
                    self.newline_indent();
                    self.write_expr(&body.value);
                    self.dedent();
                    self.newline();
                    self.newline_indent();
                }
                self.write_if_else_tail(&nested_else.value);
            } else {
                self.write("else");
                self.indent();
                self.newline_indent();
                self.write_expr(else_branch);
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
                self.write_expr(&cond.value);
                self.write(" then");
                self.indent();
                self.newline_indent();
                self.write_expr(&body.value);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            self.write("else");
            self.indent();
            self.newline_indent();
            self.write_expr(else_branch);
            self.dedent();
        }
    }

    /// Helper for flattening nested if-else in ElmFormat mode.
    fn write_if_else_tail(&mut self, else_branch: &Expr) {
        if let Expr::IfElse {
            branches: nested_branches,
            else_branch: nested_else,
        } = else_branch
        {
            for (cond, body) in nested_branches {
                self.write("else if ");
                self.write_expr(&cond.value);
                self.write(" then");
                self.indent();
                self.newline_indent();
                self.write_expr(&body.value);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            self.write_if_else_tail(&nested_else.value);
        } else {
            self.write("else");
            self.indent();
            self.newline_indent();
            self.write_expr(else_branch);
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
                self.write_expr(&branch.body.value);
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
            self.write_expr(&branch.body.value);
            self.dedent();
        }
        self.dedent();
    }

    fn write_let_expr(&mut self, declarations: &[Spanned<LetDeclaration>], body: &Expr) {
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
            self.dedent();
            self.newline_indent();
            self.write("in");
            self.newline_indent();
            self.write_expr(body);

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
        self.dedent();
        self.newline_indent();
        self.write("in");
        self.newline_indent();
        self.write_expr(body);
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
                self.write_expr(&body.value);
                self.dedent();
            }
        }
    }

    fn write_lambda(&mut self, args: &[Spanned<Pattern>], body: &Expr) {
        self.write("\\");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.write_char(' ');
            }
            self.write_pattern_atomic(&arg.value);
        }
        if self.is_multiline(body) {
            self.write(" ->");
            self.indent();
            self.newline_indent();
            self.write_expr(body);
            self.dedent();
        } else {
            self.write(" -> ");
            self.write_expr(body);
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
            Literal::Float(f) => {
                let s = f.to_string();
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
