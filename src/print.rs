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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintStyle {
    /// Round-trip-safe minimal line breaking.
    Compact,
    /// elm-format-style pretty printing.
    ElmFormat,
}

impl Default for PrintStyle {
    fn default() -> Self {
        Self::Compact
    }
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
            Expr::Lambda { body, .. } => self.is_multiline(&body.value),
            Expr::Application(args) => args.iter().any(|a| self.is_multiline(&a.value)),
            Expr::List(elems) => elems.iter().any(|e| self.is_multiline(&e.value)),
            Expr::Tuple(elems) => elems.iter().any(|e| self.is_multiline(&e.value)),
            Expr::Record(fields) => {
                fields
                    .iter()
                    .any(|f| self.is_multiline(&f.value.value.value))
            }
            Expr::RecordUpdate { updates, .. } => {
                updates
                    .iter()
                    .any(|f| self.is_multiline(&f.value.value.value))
            }
            Expr::OperatorApplication { left, right, .. } => {
                self.is_multiline(&left.value) || self.is_multiline(&right.value)
            }
            Expr::Parenthesized(inner) => self.is_multiline(&inner.value),
            Expr::Negation(inner) => self.is_multiline(&inner.value),
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
        if self.is_pretty() {
            if let Some(doc) = &module.module_documentation {
                self.doc_groups = parse_docs_groups(&doc.value);
            }
        }

        // Sort comments by source position.
        let mut comments: Vec<&Spanned<Comment>> = module.comments.iter().collect();
        comments.sort_by_key(|c| c.span.start.offset);

        // Build an ordered list of "anchors" — items with start offsets that
        // comments can be assigned relative to. Each anchor is either an import
        // or a declaration. A comment belongs before the first anchor whose
        // start offset is strictly greater than the comment's offset.
        let num_imports = module.imports.len();
        let num_decls = module.declarations.len();
        let total_anchors = num_imports + num_decls;

        // anchor_offsets[i] = start offset of anchor i
        let mut anchor_offsets: Vec<usize> = Vec::with_capacity(total_anchors);
        for imp in &module.imports {
            anchor_offsets.push(imp.span.start.offset);
        }
        for decl in &module.declarations {
            anchor_offsets.push(decl.span.start.offset);
        }

        // Assign comments to slots: one per anchor + one trailing slot.
        let mut anchor_comments: Vec<Vec<&Spanned<Comment>>> = vec![vec![]; total_anchors + 1];
        for c in &comments {
            let offset = c.span.start.offset;
            let slot = anchor_offsets
                .iter()
                .position(|&a| a > offset)
                .unwrap_or(total_anchors);
            anchor_comments[slot].push(c);
        }

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
                // ElmFormat mode: sort imports alphabetically by module name,
                // then merge duplicates (same module name).
                let mut sorted_indices: Vec<usize> =
                    (0..module.imports.len()).collect();
                sorted_indices.sort_by(|&a, &b| {
                    module.imports[a]
                        .value
                        .module_name
                        .value
                        .cmp(&module.imports[b].value.module_name.value)
                });

                // Group consecutive imports with the same module name.
                let mut i = 0;
                while i < sorted_indices.len() {
                    let first_idx = sorted_indices[i];
                    let first = &module.imports[first_idx].value;
                    let mod_name = &first.module_name.value;

                    // Collect all indices for this module name.
                    let mut group_end = i + 1;
                    while group_end < sorted_indices.len()
                        && module.imports[sorted_indices[group_end]]
                            .value
                            .module_name
                            .value
                            == *mod_name
                    {
                        group_end += 1;
                    }

                    // Emit leading comments for all imports in the group.
                    for &idx in &sorted_indices[i..group_end] {
                        if !anchor_comments[idx].is_empty() {
                            for c in &anchor_comments[idx] {
                                self.write_comment(&c.value);
                                self.newline();
                            }
                        }
                    }

                    if group_end - i == 1 {
                        // Single import — write normally.
                        self.write_import(first);
                    } else {
                        // Multiple imports for the same module — merge them.
                        self.write_merged_imports(
                            &sorted_indices[i..group_end]
                                .iter()
                                .map(|&idx| &module.imports[idx].value)
                                .collect::<Vec<_>>(),
                        );
                    }
                    self.newline();
                    i = group_end;
                }
            } else {
                for (i, imp) in module.imports.iter().enumerate() {
                    // Slot i = comments before import i.
                    if !anchor_comments[i].is_empty() {
                        for c in &anchor_comments[i] {
                            self.write_comment(&c.value);
                            self.newline();
                        }
                    }
                    self.write_import(&imp.value);
                    self.newline();
                }
            }
        }

        for (i, decl) in module.declarations.iter().enumerate() {
            let slot = num_imports + i;

            // elm-format groups consecutive infix declarations with no blank
            // lines between them.
            let is_infix = matches!(decl.value, Declaration::InfixDeclaration(_));
            let prev_is_infix = i > 0
                && matches!(
                    module.declarations[i - 1].value,
                    Declaration::InfixDeclaration(_)
                );
            let infix_group = self.is_pretty() && is_infix && prev_is_infix;

            // Blank line separator before each declaration.
            self.newline();

            // Emit leading comments for this declaration.
            if !anchor_comments[slot].is_empty() {
                self.newline();
                // elm-format treats section comments as standalone items:
                // 3 blank lines before (for i > 0), 2 blank lines after.
                // For the first declaration (i == 0), 2 blank lines before.
                if self.is_pretty() {
                    if i > 0 {
                        self.newline();
                        self.newline();
                    } else {
                        self.newline();
                    }
                }
                for c in &anchor_comments[slot] {
                    self.write_comment(&c.value);
                    self.newline();
                }
                // elm-format puts 2 blank lines after section comments too
                // (same spacing as between declarations).
                self.newline();
                self.newline();
            } else if infix_group {
                // No extra blank lines between consecutive infix declarations.
            } else {
                self.newline();
                // elm-format uses two blank lines between top-level declarations.
                // The first declaration already has the right spacing from the
                // import block (or module header), so only add extra for i > 0.
                if self.is_pretty() && i > 0 {
                    self.newline();
                }
            }

            self.write_declaration(&decl.value);
        }

        // Trailing comments after the last anchor.
        if !anchor_comments[total_anchors].is_empty() {
            self.newline();
            self.newline();
            // elm-format places 3 blank lines between the last declaration
            // and a trailing orphan comment (vs 2 blank lines between decls).
            if self.is_pretty() {
                self.newline();
                self.newline();
            }
            for (i, c) in anchor_comments[total_anchors].iter().enumerate() {
                if i > 0 {
                    self.newline();
                }
                self.write_comment(&c.value);
            }
        }

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
            let normalized = normalize_emphasis(&normalized);
            let normalized = normalize_empty_link_refs(&normalized);
            let normalized = normalize_markdown_lists(&normalized);
            let normalized = normalize_fenced_code_blocks(&normalized);
            let normalized = normalize_code_block_indent(&normalized);
            let normalized = normalize_docs_lines(&normalized);
            let normalized = strip_paragraph_leading_whitespace(&normalized);
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
            let mut emitted: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            for group in &doc_groups {
                let group_items: Vec<&ExposedItem> = group
                    .iter()
                    .filter_map(|name| {
                        let item = item_map.get(name.as_str()).copied()?;
                        // elm-format's textToRef only recognizes operators with
                        // 1 or 2 symbol characters.  Operators with 3+ chars
                        // (e.g. </>, <?>, |=, |.) silently fail to match their
                        // @docs entry and end up as leftovers. Replicate that
                        // behavior so our output matches elm-format exactly.
                        if let ExposedItem::Infix(op) = item {
                            if op.len() >= 3 {
                                return None;
                            }
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
            leftovers.sort_by(|a, b| exposed_item_sort_key(a).cmp(&exposed_item_sort_key(b)));
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
            let mut sorted_items: Vec<&ExposedItem> =
                items.iter().map(|i| &i.value).collect();
            sorted_items.sort_by(|a, b| {
                exposed_item_sort_key(a).cmp(&exposed_item_sort_key(b))
            });

            let single_line: String = {
                let parts: Vec<String> = sorted_items
                    .iter()
                    .map(|item| exposed_item_to_string(item))
                    .collect();
                format!("({})", parts.join(", "))
            };

            let line_start = self.buf.rfind('\n').map_or(0, |p| p + 1);
            let current_col = self.buf.len() - line_start;
            if current_col + single_line.len() <= 200 {
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
            let is_redundant = self.is_pretty()
                && alias.value == import.module_name.value;
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
        let has_expose_all = imports.iter().any(|imp| {
            matches!(&imp.exposing, Some(e) if matches!(e.value, Exposing::All(_)))
        });
        if has_expose_all {
            self.write(" exposing (..)");
        } else {
            // Collect all exposed items from all imports.
            let mut all_items: Vec<&ExposedItem> = Vec::new();
            for imp in imports {
                if let Some(exposing) = &imp.exposing {
                    if let Exposing::Explicit(items) = &exposing.value {
                        for item in items {
                            all_items.push(&item.value);
                        }
                    }
                }
            }
            if !all_items.is_empty() {
                // Deduplicate and sort.
                all_items.sort_by(|a, b| {
                    exposed_item_sort_key(a).cmp(&exposed_item_sort_key(b))
                });
                all_items.dedup_by(|a, b| {
                    exposed_item_sort_key(a) == exposed_item_sort_key(b)
                });
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
                let mut sorted: Vec<&ExposedItem> =
                    items.iter().map(|i| &i.value).collect();
                sorted.sort_by(|a, b| {
                    exposed_item_sort_key(a).cmp(&exposed_item_sort_key(b))
                });
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
        self.write(" : ");
        self.write_type(&sig.type_annotation.value);
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
        self.write_expr(&imp.body.value);
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
        for arg in &ctor.args {
            self.write_char(' ');
            self.write_type_atomic(&arg.value);
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
                self.write_pattern_app(&head.value);
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

                // In pretty mode, flatten |>/>>/<< chains. elm-format's rule:
                // if ANY part of a binops expression contains newlines (i.e.,
                // any operand is multiline), ALL operators break to vertical.
                // We detect this via is_multiline on each chain operand.
                if self.is_pretty()
                    && matches!(operator.as_str(), "|>" | ">>" | "<<")
                {
                    if let Some(chain) = flatten_left_assoc_chain(expr, operator) {
                        let any_ml = chain.iter().any(|op| self.is_multiline(op));
                        if any_ml {
                            // Break ALL operators to vertical.
                            self.write_expr_operand(chain[0], operator, true);
                            self.indent();
                            for operand in &chain[1..] {
                                self.newline_indent();
                                self.write(operator);
                                self.write_char(' ');
                                self.write_expr_operand(operand, operator, false);
                            }
                            self.dedent();
                            return;
                        }
                        // All operands are single-line; fall through to
                        // normal inline path (which handles recursion).
                    }
                }

                // Same rule for right-associative :: and ++ chains.
                if self.is_pretty() && matches!(operator.as_str(), "::" | "++") {
                    if let Some(chain) = flatten_right_assoc_chain(expr, operator) {
                        let any_ml = chain.iter().any(|op| self.is_multiline(op));
                        if any_ml {
                            self.write_expr_operand(chain[0], operator, true);
                            self.indent();
                            for operand in &chain[1..] {
                                self.newline_indent();
                                self.write(operator);
                                self.write_char(' ');
                                self.write_expr_operand(operand, operator, false);
                            }
                            self.dedent();
                            return;
                        }
                    }
                }

                let use_vertical = if self.is_pretty() {
                    // elm-format: if either operand is multiline, break.
                    self.is_multiline(&left.value) || self.is_multiline(right_expr)
                } else {
                    self.is_multiline(right_expr)
                };
                self.write_leading_comments(&left.comments);
                self.write_expr_operand(&left.value, operator, true);
                if use_vertical && operator == "<|" {
                    // Left-pipe: operator stays on same line as left operand,
                    // right operand goes on a new indented line.
                    // This matches elm-format's behavior for `<|`.
                    self.write(" <|");
                    self.indent();
                    self.newline_indent();
                    self.write_leading_comments(&right.comments);
                    // Use write_expr_inner so block expressions (Lambda,
                    // IfElse, etc.) aren't re-wrapped in parens.
                    self.write_expr_inner(right_expr);
                    self.dedent();
                } else if use_vertical {
                    // Vertical layout: operator and right operand on a new
                    // indented line so that the right side starts at a
                    // predictable column (satisfying the parser's indent rules).
                    self.indent();
                    self.newline_indent();
                    self.write(operator);
                    self.write_char(' ');
                    self.write_leading_comments(&right.comments);
                    self.write_expr_operand(right_expr, operator, false);
                    self.dedent();
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
                        for (i, (_operand, op)) in
                            operands_and_operators.iter().enumerate()
                        {
                            self.newline_indent();
                            self.write(&op.value);
                            self.write_char(' ');
                            if i + 1 < operands_and_operators.len() {
                                self.write_expr_app(
                                    &operands_and_operators[i + 1].0.value,
                                );
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
                    self.write_char('(');
                    self.write_expr_inner(expr);
                    self.write_char(')');
                } else {
                    self.write_expr_inner(expr);
                }
            }
            _ => self.write_expr_app(expr),
        }
    }

    /// Write a function application or negation.
    fn write_expr_app(&mut self, expr: &Expr) {
        match expr {
            Expr::Application(args) => {
                // When any argument (beyond the function) is multiline,
                // use vertical layout so each arg starts on a new indented
                // line — this ensures args are always at a column greater
                // than the function name, satisfying the parser's indent rules.
                let any_arg_ml =
                    args.len() > 1 && args.iter().skip(1).any(|a| self.is_multiline(&a.value));
                if any_arg_ml {
                    self.write_expr_atomic(&args[0].value);
                    self.indent();
                    for arg in &args[1..] {
                        self.newline_indent();
                        self.write_expr_atomic(&arg.value);
                    }
                    self.dedent();
                } else {
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.write_char(' ');
                        }
                        self.write_expr_atomic(&arg.value);
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

    /// Write an expression in atomic (highest-precedence) position.
    /// Complex and block expressions get parenthesized.
    fn write_expr_atomic(&mut self, expr: &Expr) {
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
                        let saved_extra = self.indent_extra;
                        self.write_char('(');
                        self.write_expr(&inner.value);
                        self.indent_extra = saved_extra;
                        self.newline_indent();
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
                    let any_ml =
                        fields.iter().any(|f| self.is_multiline(&f.value.value.value));
                    if any_ml {
                        self.write("{ ");
                        self.write_record_setter(&fields[0].value);
                        for field in &fields[1..] {
                            self.newline_indent();
                            self.write(", ");
                            self.write_record_setter(&field.value);
                        }
                        self.newline_indent();
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
                    .any(|f| self.is_multiline(&f.value.value.value));
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
        let any_multiline = elems.iter().any(|e| self.is_multiline(&e.value));
        if any_multiline && self.is_pretty() {
            // elm-format style: first element on same line as open bracket,
            // subsequent elements aligned with ", " prefix at same indent.
            // Set indent_extra = 2 so block expressions (if-else, let-in)
            // inside elements align else/in with the element content after
            // the "[ " or ", " prefix.
            let saved_extra = self.indent_extra;
            self.write(open);
            self.indent_extra = saved_extra + 2;
            self.write_expr(&elems[0].value);
            self.indent_extra = saved_extra;
            for elem in &elems[1..] {
                self.newline_indent();
                self.write(", ");
                self.indent_extra = saved_extra + 2;
                self.write_expr(&elem.value);
                self.indent_extra = saved_extra;
            }
            self.newline_indent();
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
            // Flatten nested if-else into else-if chains.
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
            self.write("case ");
            self.write_expr(subject);
            self.write(" of");
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
        for (i, decl) in declarations.iter().enumerate() {
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
                    let abs = n.unsigned_abs() as u64;
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
                self.write(&s);
                if !s.contains('.') {
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
fn should_unicode_escape(c: char) -> bool {
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

/// Reindent a multiline block comment's content. `brace_col` is the column
/// where `{-` is being emitted in the new output. We estimate the original
/// `{-` column from the indent of the line preceding `-}` (or the min
/// non-empty indent of middle lines - 3), then shift continuation lines so
/// they land at the corresponding position relative to the new `{-`.
fn reindent_block_comment(text: &str, brace_col: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() <= 1 {
        return text.to_string();
    }

    let last = lines.last().unwrap();
    let last_is_ws_only = last.trim().is_empty();
    let original_base: usize = if last_is_ws_only {
        last.chars().take_while(|c| *c == ' ').count()
    } else {
        let mut min_indent = usize::MAX;
        for line in &lines[1..] {
            if line.trim().is_empty() {
                continue;
            }
            let ind = line.chars().take_while(|c| *c == ' ').count();
            if ind < min_indent {
                min_indent = ind;
            }
        }
        if min_indent == usize::MAX {
            return text.to_string();
        }
        min_indent.saturating_sub(3)
    };

    let delta: isize = brace_col as isize - original_base as isize;

    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            out.push_str(line);
        } else {
            out.push('\n');
            let is_last = i == lines.len() - 1;
            if line.is_empty() && !is_last {
                continue;
            }
            let ind = line.chars().take_while(|c| *c == ' ').count();
            let rest = &line[ind..];
            let new_ind = ((ind as isize) + delta).max(0) as usize;
            for _ in 0..new_ind {
                out.push(' ');
            }
            out.push_str(rest);
        }
    }
    out
}

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

/// Normalize doc comment content to match elm-format's conventions.
///
/// elm-format round-trips doc comments through a Markdown parser (Cheapskate)
/// and re-serializes them. We approximate the most impactful normalizations
/// without a full Markdown parser:
///
/// 1. `*text*` → `_text_` (emphasis normalization, but not `**bold**`)
/// 2. `[text][]` → `[text]` (empty link references)
/// 3. Ensure blank line after `# Heading` before `@docs`
/// 4. Ensure double blank line before `# Heading` (after content)
/// 5. Ensure trailing `\n\n` before `-}` for multi-paragraph docs
/// 6. Single-line docs: `{-| text -}` → `{-| text\n-}` (strip trailing space)
/// 7. Empty docs: `""` → `" "`
fn normalize_doc_comment(text: &str) -> String {
    // Rule 7: Empty or whitespace-only doc → single space
    // elm-format: `{-|-}` or `{-| -}` → `{-| -}`
    if text.trim().is_empty() {
        return " ".to_string();
    }

    let mut result = String::with_capacity(text.len() + 16);

    // Rule 6: Single-line doc — content has no newline (just ` text `)
    // elm-format: `{-| text. -}` → `{-| text.\n-}`
    if !text.contains('\n') {
        result.push_str(text.trim_end());
        result.push('\n');
        return result;
    }

    // Rule 8: If doc starts with `\n` followed by non-empty content (no
    // intervening blank line), collapse the leading newline to a space.
    // elm-format: `{-|\nText... -}` → `{-| Text...\n-}`
    // Exception: if the first content line is a 4-space-indented code block,
    // leave the newline in place (and insert a blank line before the code
    // block, matching elm-format's behavior).
    let text = if text.starts_with('\n') && !text.starts_with("\n\n") {
        let rest = &text[1..];
        if !rest.is_empty() && !rest.starts_with('\n') && !rest.trim().is_empty() {
            if rest.starts_with("    ") {
                // Keep as a code block: `{-|\n\n    code...`
                std::borrow::Cow::Owned(format!("\n\n{}", rest))
            } else {
                std::borrow::Cow::Owned(format!(" {}", rest))
            }
        } else {
            std::borrow::Cow::Borrowed(text)
        }
    } else {
        std::borrow::Cow::Borrowed(text)
    };

    // Multi-line doc: apply transformations.
    // Work line-by-line for heading spacing rules.
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Rule 4: Double blank line before any `# Heading` or `## Heading` etc.
        // If we see a markdown heading and the preceding output doesn't already
        // have a double blank line, add one.
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
            // Check if preceding content in result ends with \n\n\n (double blank)
            if !result.is_empty() && !result.ends_with("\n\n\n") {
                // We need at least one more newline. The line separator from the
                // previous line already contributes one \n. We need \n\n\n total
                // before the heading.
                if result.ends_with("\n\n") {
                    result.push('\n');
                } else if result.ends_with('\n') {
                    result.push_str("\n\n");
                }
            }
        }

        result.push_str(line);

        if i + 1 < lines.len() {
            result.push('\n');

            // Rule 3: Blank line after any heading before `@docs` or content
            if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
                // Check if next non-empty line is `@docs` or other content
                // and there isn't already a blank line
                if i + 1 < lines.len() && !lines[i + 1].trim().is_empty() {
                    result.push('\n');
                }
            }
        }

        i += 1;
    }

    // Rule 5: Ensure correct trailing newlines before `-}`.
    // - Multi-paragraph docs (content with a blank line between content): end with `\n\n`
    // - Single-paragraph multi-line docs: end with `\n`
    // Check for a blank line between content (not just trailing blank lines).
    let trimmed_result = result.trim_end_matches('\n');
    let has_multiple_paragraphs = trimmed_result.contains("\n\n");
    if has_multiple_paragraphs {
        // Ensure trailing \n\n
        if !result.ends_with("\n\n") {
            if result.ends_with('\n') {
                result.push('\n');
            } else {
                result.push_str("\n\n");
            }
        }
    } else {
        // Single paragraph: ensure trailing \n
        // Strip any extra trailing newlines first.
        while result.ends_with("\n\n") {
            result.pop();
        }
        if !result.ends_with('\n') {
            result.push('\n');
        }
    }

    result
}

/// Normalize emphasis markers in doc comment text: `*text*` → `_text_`.
/// Also escapes lone `*` as `\*` (matching elm-format's Cheapskate round-trip).
/// Does NOT convert `**bold**`. Does NOT modify `*` inside code spans or
/// indented code blocks.
fn normalize_emphasis(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    let mut at_line_start = true;
    let mut line_indent = 0u32;
    let mut in_docs_line = false;
    let mut prev_line_blank = false;
    let mut current_line_has_content = false;

    // Helper: push the UTF-8 character starting at byte position `pos` in
    // `text` into `result` and return the number of bytes consumed.
    #[inline]
    fn push_utf8_char(text: &str, pos: usize, result: &mut String) -> usize {
        let ch = text[pos..].chars().next().unwrap();
        result.push(ch);
        ch.len_utf8()
    }

    while i < len {
        let ch = bytes[i];

        // Non-ASCII byte: part of a multi-byte UTF-8 character.
        // Copy the full character to avoid double-encoding.
        if ch > 127 {
            at_line_start = false;
            i += push_utf8_char(text, i, &mut result);
            continue;
        }

        // Track line starts and indentation for code block detection.
        if ch == b'\n' {
            result.push('\n');
            i += 1;
            prev_line_blank = !current_line_has_content;
            at_line_start = true;
            line_indent = 0;
            in_docs_line = false;
            current_line_has_content = false;
            continue;
        }
        if at_line_start {
            if ch == b' ' {
                line_indent += 1;
                result.push(' ');
                i += 1;
                continue;
            }
            at_line_start = false;
            current_line_has_content = true;
            // Detect @docs lines — skip emphasis processing on these.
            if text[i..].starts_with("@docs") {
                in_docs_line = true;
            }
        }

        // On @docs lines, pass through unchanged (operators like (*) must not
        // have their `*` escaped or converted).
        if in_docs_line {
            result.push(ch as char);
            i += 1;
            continue;
        }

        // Inside a code block (4+ spaces indent after a blank line) — pass through unchanged.
        // Only treat as a code block if preceded by a blank line (proper markdown code block).
        // List continuation lines with 4+ indent should still have emphasis processed.
        if line_indent >= 4 && prev_line_blank {
            result.push(ch as char);
            i += 1;
            continue;
        }

        // Handle backtick sequences for code spans.
        // In CommonMark/Cheapskate, a code span is opened by N backticks and
        // closed by exactly N backticks. If no matching closer exists, the
        // backticks are literal and should be escaped.
        if ch == b'`' {
            // Count consecutive backticks.
            let bt_start = i;
            let mut bt_count = 0;
            while i + bt_count < len && bytes[i + bt_count] == b'`' {
                bt_count += 1;
            }

            // Fenced code blocks: 3+ backticks at the start of a line are fenced
            // code block markers. Pass through unchanged — they're handled by
            // normalize_fenced_code_blocks separately.
            if bt_count >= 3 && (bt_start == 0 || bytes[bt_start - 1] == b'\n') {
                // Copy the opening fence line.
                let mut pos = bt_start;
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                result.push_str(&text[bt_start..pos]);
                i = pos;
                // Copy everything until closing fence.
                if i < len && bytes[i] == b'\n' {
                    result.push('\n');
                    i += 1;
                }
                while i < len {
                    let line_start = i;
                    // Check for closing fence (3+ backticks at start of line).
                    let mut bc = 0;
                    while i + bc < len && bytes[i + bc] == b'`' {
                        bc += 1;
                    }
                    if bc >= bt_count {
                        // Check rest of line is whitespace.
                        let mut j = i + bc;
                        let mut rest_ws = true;
                        while j < len && bytes[j] != b'\n' {
                            if bytes[j] != b' ' && bytes[j] != b'\t' {
                                rest_ws = false;
                                break;
                            }
                            j += 1;
                        }
                        if rest_ws {
                            result.push_str(&text[line_start..j]);
                            i = j;
                            break;
                        }
                    }
                    // Not a closing fence — copy line.
                    while i < len {
                        if bytes[i] > 127 {
                            let ch = text[i..].chars().next().unwrap();
                            result.push(ch);
                            i += ch.len_utf8();
                        } else {
                            result.push(bytes[i] as char);
                            i += 1;
                        }
                        if i > 0 && bytes[i - 1] == b'\n' {
                            break;
                        }
                    }
                }
                continue;
            }

            // Inline code span: look for a matching closer on the same line
            // (for single backtick) or within the same paragraph (for multi-backtick).
            let after_open = bt_start + bt_count;
            let mut found_close = false;
            let mut close_start = after_open;
            // Determine search boundary.
            let mut search_end = after_open;
            if bt_count == 1 {
                // Don't cross newlines for single-backtick spans.
                while search_end < len && bytes[search_end] != b'\n' {
                    search_end += 1;
                }
            } else {
                // Multi-backtick spans stop at blank lines.
                while search_end < len {
                    if bytes[search_end] == b'\n' {
                        let nls = search_end + 1;
                        if nls >= len {
                            search_end = len;
                            break;
                        }
                        let mut ws = nls;
                        while ws < len && bytes[ws] == b' ' {
                            ws += 1;
                        }
                        if ws >= len || bytes[ws] == b'\n' {
                            break;
                        }
                    }
                    search_end += 1;
                }
            }

            while close_start < search_end {
                if bytes[close_start] == b'`' {
                    let mut cc = 0;
                    while close_start + cc < len && bytes[close_start + cc] == b'`' {
                        cc += 1;
                    }
                    if cc == bt_count {
                        found_close = true;
                        result.push_str(&text[bt_start..close_start + cc]);
                        i = close_start + cc;
                        break;
                    }
                    close_start += cc;
                } else {
                    close_start += 1;
                }
            }

            if !found_close {
                // No matching closer — escape each backtick.
                for _ in 0..bt_count {
                    result.push('\\');
                    result.push('`');
                }
                i = bt_start + bt_count;
            }
            continue;
        }

        if ch == b'*' {
            // Check for **bold** — leave as-is
            if i + 1 < len && bytes[i + 1] == b'*' {
                // Start of **bold** — scan for closing **
                result.push('*');
                result.push('*');
                i += 2;
                while i < len {
                    if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'*' {
                        result.push('*');
                        result.push('*');
                        i += 2;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        result.push('\n');
                        i += 1;
                        at_line_start = true;
                        line_indent = 0;
                        break;
                    }
                    if bytes[i] > 127 {
                        i += push_utf8_char(text, i, &mut result);
                    } else {
                        result.push(bytes[i] as char);
                        i += 1;
                    }
                }
                continue;
            }

            // Single *emphasis* — scan for closing *
            // But only if the next char is not a space (valid emphasis opening)
            if i + 1 < len && bytes[i + 1] != b' ' && bytes[i + 1] != b'\n' {
                // Look for closing *
                let start = i + 1;
                let mut end = start;
                let mut found = false;
                while end < len {
                    if bytes[end] == b'*' && end > start && bytes[end - 1] != b' ' {
                        found = true;
                        break;
                    }
                    if bytes[end] == b'\n' {
                        break; // Don't cross line boundaries
                    }
                    end += 1;
                }
                if found {
                    result.push('_');
                    // Copy the emphasized text properly (handle multi-byte)
                    result.push_str(&text[start..end]);
                    result.push('_');
                    i = end + 1;
                    continue;
                }
            }

            // Lone `*` that's not emphasis — escape it (unless already escaped).
            if i > 0 && bytes[i - 1] == b'\\' {
                // Already preceded by backslash — don't double-escape.
                result.push('*');
            } else {
                result.push('\\');
                result.push('*');
            }
            i += 1;
        } else {
            result.push(ch as char);
            i += 1;
        }
    }
    result
}

/// Normalize empty link references: `[text][]` → `[text]`.
/// Only removes `[]` that immediately follows `]` (i.e., the pattern `][]`).
fn normalize_empty_link_refs(text: &str) -> String {
    text.replace("][]", "]")
}


/// Re-serialize `@docs` lines in doc comment text.
/// elm-format normalizes multi-line `@docs` with continuation lines
/// (after a trailing comma) into separate `@docs` directives. Each
/// continuation line becomes its own `@docs` line. The original `@docs`
/// line also has its trailing comma removed.
fn normalize_docs_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@docs") {
            let leading_ws: &str = &line[..line.len() - line.trim_start().len()];

            // Get names from the @docs line itself.
            let base_names: Vec<String> = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let has_trailing_comma = rest.trim_end().ends_with(',');

            // Emit the base @docs line (without trailing comma).
            result.push_str(leading_ws);
            result.push_str("@docs ");
            result.push_str(&base_names.join(", "));

            // Consume and emit continuation lines as separate @docs.
            if has_trailing_comma {
                while i + 1 < lines.len() {
                    let next = lines[i + 1].trim();
                    if next.is_empty() || next.starts_with('@') || next.starts_with('#') {
                        break;
                    }
                    i += 1;
                    let cont_names: Vec<String> = next
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let cont_trailing = next.ends_with(',');
                    result.push('\n');
                    result.push_str(leading_ws);
                    result.push_str("@docs ");
                    result.push_str(&cont_names.join(", "));
                    if !cont_trailing {
                        break;
                    }
                }
            }
        } else {
            result.push_str(line);
        }
        if i + 1 < lines.len() {
            result.push('\n');
        }
        i += 1;
    }
    result
}

/// Strip leading whitespace from paragraph continuation lines in doc comments.
///
/// In Cheapskate (elm-format's markdown engine), paragraph continuation lines
/// have their leading whitespace normalized. This strips up to 1 space of
/// consistent leading indent from non-first lines within each paragraph,
/// but preserves code blocks (4+ space indent after blank line) and list items.
fn strip_paragraph_leading_whitespace(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;

    for (i, &line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }

        // Track code block state (4+ space indent after blank line).
        if line.starts_with("    ") {
            if i == 0 || lines[i - 1].trim().is_empty() {
                in_code_block = true;
            }
        } else if !line.trim().is_empty() {
            in_code_block = false;
        }

        if in_code_block || line.trim().is_empty() {
            result.push_str(line);
            continue;
        }

        // Skip the first line (it's the space after {-|).
        if i == 0 {
            result.push_str(line);
            continue;
        }

        // Skip list items (already handled by normalize_markdown_lists).
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ")
            || trimmed.starts_with("@docs")
            || trimmed.starts_with('#')
            || strip_ordered_list_prefix(trimmed).is_some()
        {
            result.push_str(line);
            continue;
        }

        // Strip a single leading space if the line starts with " X" where X
        // is a non-space character. This matches Cheapskate's paragraph
        // whitespace normalization.
        if line.starts_with(' ') && line.len() > 1 && !line.as_bytes()[1].is_ascii_whitespace() {
            result.push_str(&line[1..]);
        } else {
            result.push_str(line);
        }
    }
    result
}

/// Strip trailing whitespace from each line in a doc comment.
///
/// elm-format removes trailing spaces from doc comment lines. We do the same
/// as a final normalization step. We must be careful not to strip trailing
/// whitespace from the very last part (the line ending before `-}`) since
/// that's structural.
fn strip_trailing_whitespace_in_doc(text: &str) -> String {
    // Split on newlines, trim trailing whitespace from each line except the
    // last segment (which may be just whitespace before `-}`).
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if i == lines.len() - 1 {
            // Last segment — preserve as-is (it's the closing indent).
            result.push_str(line);
        } else {
            result.push_str(line.trim_end());
        }
    }
    result
}

/// Normalize markdown list indentation in doc comments.
///
/// elm-format's Cheapskate markdown parser indents unordered list items
/// by 2 spaces: `- item` becomes `  - item`. This only applies to lines
/// that are NOT inside code blocks (4+ space indentation).
fn normalize_markdown_lists(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;
    // Track list item continuation: if we're inside a list item, continuation
    // lines (non-blank, non-list-marker lines) get indented to align with the
    // list item content.
    let mut list_indent: Option<usize> = None; // indent width for continuation lines

    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }

        // Track code block state: lines starting with 4+ spaces after a blank
        // line enter code block mode; non-indented lines after a blank leave it.
        if line.starts_with("    ") {
            if i == 0 || lines[i - 1].trim().is_empty() {
                in_code_block = true;
            }
        } else if !line.trim().is_empty() && !line.starts_with("    ") {
            in_code_block = false;
        }

        if in_code_block {
            result.push_str(line);
        } else if line.trim().is_empty() {
            // Blank line ends list continuation context.
            list_indent = None;
            result.push_str(line);
        } else if line.starts_with("- ") || *line == "-" {
            // Unordered list item: indent by 2 spaces.
            result.push_str("  ");
            result.push_str(line);
            // "  - " = 4 chars of prefix before content
            list_indent = Some(4);
        } else if let Some(rest) = strip_ordered_list_prefix(line) {
            // Ordered list item: strip leading spaces, double-space after period.
            // `  1. text` or `1. text` -> `1.  text`
            let trimmed = line.trim_start();
            // Extract the number and period part
            let prefix_len = trimmed.len() - rest.len();
            let number_part = &trimmed[..prefix_len]; // e.g. "1. "
            let number_dot = number_part.trim_end();   // e.g. "1."
            result.push_str(number_dot);
            result.push_str("  ");
            result.push_str(rest);
            // Continuation indent = length of "N.  " prefix
            list_indent = Some(number_dot.len() + 2);
        } else if let Some(indent_width) = list_indent {
            // Continuation line of a list item: indent to align with content.
            let trimmed = line.trim_start();
            if trimmed.starts_with("@docs") || trimmed.starts_with('#') {
                // New heading or @docs ends the list context.
                list_indent = None;
                result.push_str(line);
            } else {
                for _ in 0..indent_width {
                    result.push(' ');
                }
                result.push_str(trimmed);
            }
        } else {
            result.push_str(line);
        }
    }
    result
}

/// Convert fenced code blocks (triple-backtick) to indented code blocks.
///
/// elm-format's Cheapskate markdown parser converts fenced code blocks to
/// 4-space indented code blocks. We do the same to match elm-format output.
fn normalize_fenced_code_blocks(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        // Detect opening fence: only plain ``` without a language tag.
        // Fenced blocks with language tags (e.g. ```elm) are preserved by elm-format.
        if trimmed == "```" {
            // Find the closing fence
            let mut end = i + 1;
            let mut found_close = false;
            while end < lines.len() {
                if lines[end].trim() == "```" {
                    found_close = true;
                    break;
                }
                end += 1;
            }

            if found_close {
                // Convert: skip opening fence, indent content lines by 4 spaces,
                // skip closing fence.
                for j in (i + 1)..end {
                    if !result.is_empty() || j > i + 1 {
                        result.push('\n');
                    }
                    if lines[j].is_empty() {
                        // Keep blank lines blank
                    } else {
                        result.push_str("    ");
                        result.push_str(lines[j]);
                    }
                }
                i = end + 1;
                continue;
            }
        }

        if i > 0 {
            result.push('\n');
        }
        result.push_str(lines[i]);
        i += 1;
    }
    result
}

/// Check if a line is an ordered list item: optional whitespace, digits, period, space(s).
/// Returns the text after all spaces following "N.", or None.
fn strip_ordered_list_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    // Must start with a digit
    let mut chars = trimmed.char_indices();
    let first = chars.next()?;
    if !first.1.is_ascii_digit() {
        return None;
    }
    // Consume remaining digits
    let mut after_digits = first.0 + 1;
    for (pos, ch) in chars {
        if ch.is_ascii_digit() {
            after_digits = pos + 1;
        } else {
            break;
        }
    }
    // Must be followed by "." then at least one space
    let rest = &trimmed[after_digits..];
    let after_dot = rest.strip_prefix('.')?;
    if !after_dot.starts_with(' ') {
        return None;
    }
    Some(after_dot.trim_start())
}

/// Normalize code examples in doc comments by re-parsing and re-formatting them.
///
/// elm-format re-parses indented code blocks (4+ spaces after a blank line) as
/// Elm code and reformats them. We do the same: strip the 4-space prefix, wrap
/// in a dummy module, parse, pretty-print, then re-indent with 4 spaces.
/// If parsing fails, the code block is left unchanged.
fn normalize_code_block_indent(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Check if this line starts a code block:
        // - must have 4+ leading spaces
        // - must be preceded by a blank line (or be the first line)
        let starts_code = line.starts_with("    ")
            && (i == 0
                || lines[i - 1].trim().is_empty());

        if !starts_code {
            result.push_str(line);
            if i + 1 < lines.len() {
                result.push('\n');
            }
            i += 1;
            continue;
        }

        // Collect the code block lines.
        let block_start = i;
        let mut block_end = i; // inclusive
        while block_end + 1 < lines.len() {
            let next = lines[block_end + 1];
            if next.trim().is_empty() {
                // Blank line: include if followed by another code line
                if block_end + 2 < lines.len() && lines[block_end + 2].starts_with("    ") {
                    block_end += 1;
                    continue;
                }
                break;
            } else if next.starts_with("    ") {
                block_end += 1;
            } else {
                break;
            }
        }

        // Only try to reformat if the code block appears to use non-elm-format
        // indentation (e.g. 2-space indent). Code blocks already using 4-space
        // indentation are left unchanged to avoid regressions from imperfect
        // pretty printing.
        let needs_reformat = code_block_needs_reformat(&lines[block_start..=block_end]);

        let reformatted = if needs_reformat {
            try_reformat_code_block(&lines[block_start..=block_end])
        } else {
            None
        };

        if let Some(reformatted) = reformatted {
            result.push_str(&reformatted);
            if block_end < lines.len() - 1 {
                result.push('\n');
            }
        } else {
            // Parsing failed or not needed — emit the block, but apply a
            // lightweight assertion-paragraph transform: adjacent lines that
            // look like `expr == value` get a blank line inserted between them
            // and have multi-space runs (outside strings) collapsed, matching
            // elm-format's behavior.
            let block = &lines[block_start..=block_end];
            let transformed = transform_assertion_paragraphs(block);
            let end_idx = result.len();
            result.push_str(&transformed);
            let _ = end_idx;
            if block_end < lines.len() - 1 {
                result.push('\n');
            }
        }
        i = block_end + 1;
    }

    result
}

/// Scan a doc code block for "assertion paragraphs" (runs of adjacent
/// non-blank lines whose trimmed form contains ` == `) and rewrite each such
/// paragraph so every assertion line becomes its own paragraph, with
/// multi-space runs collapsed outside of string literals. Other lines are
/// emitted unchanged. elm-format re-parses these blocks as expressions and
/// renders each on its own "top level", which produces this output.
fn transform_assertion_paragraphs(block_lines: &[&str]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < block_lines.len() {
        let line = block_lines[i];
        if line.trim().is_empty() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
            i += 1;
            continue;
        }

        // Collect a run of adjacent non-blank lines.
        let run_start = i;
        let mut run_end = i;
        while run_end + 1 < block_lines.len()
            && !block_lines[run_end + 1].trim().is_empty()
        {
            run_end += 1;
        }

        // Check if every line in this run is either a top-level assertion
        // (`expr == value` / `expr -- comment`) or a line-comment (`-- ...`),
        // all at the same leading indent, and the run ends in an assertion.
        // Comments stay attached to the following assertion; assertions are
        // separated from one another by blank lines.
        let first_indent = block_lines[run_start].len() - block_lines[run_start].trim_start().len();
        let mut all_valid = true;
        let mut assertion_count = 0usize;
        for k in run_start..=run_end {
            let l = block_lines[k];
            let indent = l.len() - l.trim_start().len();
            if indent != first_indent {
                all_valid = false;
                break;
            }
            let trimmed = l.trim();
            if trimmed.starts_with("--") {
                // comment line — ok in between assertions
            } else if looks_like_assertion(trimmed) {
                assertion_count += 1;
            } else {
                all_valid = false;
                break;
            }
        }
        let last_is_assertion = {
            let trimmed = block_lines[run_end].trim();
            !trimmed.starts_with("--") && looks_like_assertion(trimmed)
        };
        let is_assertion_run = all_valid && assertion_count >= 2 && last_is_assertion;

        if is_assertion_run {
            // Split the run into "units": a unit is zero or more consecutive
            // comment lines followed by one assertion line. Units are
            // separated by blank lines; within a unit, lines are contiguous.
            let mut unit_start = run_start;
            let mut unit_idx: usize = 0;
            let mut k = run_start;
            while k <= run_end {
                let trimmed = block_lines[k].trim();
                let is_comment = trimmed.starts_with("--");
                if !is_comment {
                    if unit_idx == 0 && i > 0 {
                        out.push('\n');
                    } else if unit_idx > 0 {
                        out.push_str("\n\n");
                    }
                    for (j, idx) in (unit_start..=k).enumerate() {
                        if j > 0 {
                            out.push('\n');
                        }
                        let l = block_lines[idx];
                        if idx == k {
                            let indent_str = &l[..first_indent];
                            let content = &l[first_indent..];
                            let normalized = collapse_spaces_outside_strings(content);
                            out.push_str(indent_str);
                            out.push_str(&normalized);
                        } else {
                            out.push_str(l);
                        }
                    }
                    unit_idx += 1;
                    unit_start = k + 1;
                }
                k += 1;
            }
        } else {
            for (k, idx) in (run_start..=run_end).enumerate() {
                if i > 0 || k > 0 {
                    out.push('\n');
                }
                out.push_str(block_lines[idx]);
            }
        }
        i = run_end + 1;
    }
    out
}

fn is_assertion_only_paragraph(para: &[String]) -> bool {
    let non_empty: Vec<&String> = para.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.len() < 2 {
        return false;
    }
    for line in &non_empty {
        // Must start at column 0 (no leading whitespace beyond what was stripped).
        if line.starts_with(' ') || line.starts_with('\t') {
            return false;
        }
        if !looks_like_assertion(line.trim()) {
            return false;
        }
    }
    true
}

fn looks_like_assertion(trimmed: &str) -> bool {
    // Two accepted shapes for "example lines" inside doc code blocks:
    //   1. `expr == value` (optionally with trailing ` -- comment`)
    //   2. `expr -- comment` (expression followed by a line comment)
    // Neither begins with `--` (that's a standalone comment line).
    if trimmed.starts_with("--") {
        return false;
    }
    if let Some(eq) = trimmed.find(" == ") {
        let (left, right) = (&trimmed[..eq], &trimmed[eq + 4..]);
        if left.is_empty() || right.is_empty() {
            return false;
        }
        if right.starts_with('=') {
            return false;
        }
        let last_ch = left.chars().last().unwrap();
        if "+-*/|&<>".contains(last_ch) {
            return false;
        }
        return true;
    }
    // Shape 2: `expr -- comment`. Require ` -- ` separator and non-empty left.
    if let Some(dash) = trimmed.find(" -- ") {
        let left = &trimmed[..dash];
        if left.is_empty() {
            return false;
        }
        let last_ch = left.chars().last().unwrap();
        if "+-*/|&<>=".contains(last_ch) {
            return false;
        }
        return true;
    }
    false
}

fn collapse_spaces_outside_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escape = false;
    let mut prev_space = false;
    for c in s.chars() {
        if escape {
            out.push(c);
            escape = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            out.push(c);
            prev_space = false;
        } else if c == '"' {
            in_string = true;
            out.push(c);
            prev_space = false;
        } else if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// Check whether a code block needs reformatting.
///
/// Returns true if the block contains:
/// - lines with non-4-aligned indentation (2-space indent), OR
/// - compact list/tuple syntax that elm-format would space out
///   (e.g., `[1,2]` -> `[ 1, 2 ]`, `(0,"a")` -> `( 0, "a" )`)
fn code_block_needs_reformat(block_lines: &[&str]) -> bool {
    let mut count_4_aligned = 0usize;
    let mut count_non_4_aligned = 0usize;
    let mut has_compact_syntax = false;
    let mut has_single_line_decl = false;
    for &line in block_lines {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.len() - line.trim_start().len();
        if leading > 4 {
            if (leading - 4) % 4 == 0 {
                count_4_aligned += 1;
            } else {
                count_non_4_aligned += 1;
            }
        }
        // Check for compact list syntax [x,y] or tuple syntax (x,y) that
        // elm-format would normalize to [ x, y ] or ( x, y ).
        let trimmed = line.trim();
        if trimmed.contains("[") && trimmed.contains(",") && trimmed.contains("]") {
            // Look for `[x,` or `[x]` without spaces after [ or before ]
            if trimmed.contains("[\"") || trimmed.contains("[(")
                || trimmed.contains("[0") || trimmed.contains("[1")
                || trimmed.contains("[2") || trimmed.contains("[3")
                || trimmed.contains("[4") || trimmed.contains("[5")
                || trimmed.contains("[6") || trimmed.contains("[7")
                || trimmed.contains("[8") || trimmed.contains("[9")
            {
                has_compact_syntax = true;
            }
        }
        if trimmed.contains("(") && trimmed.contains(",") && trimmed.contains(")") {
            if trimmed.contains("(0") || trimmed.contains("(1")
                || trimmed.contains("(2") || trimmed.contains("(3")
                || trimmed.contains("(\"")
            {
                has_compact_syntax = true;
            }
        }
        // Single-line value declaration at the block base-indent (4 spaces):
        // `name = expr` fits on one line. elm-format always expands these
        // to two lines (`name =\n    expr`), so flag for reformat.
        if leading == 4 && is_single_line_value_decl(trimmed) {
            has_single_line_decl = true;
        }
    }
    let has_indent_issues = count_non_4_aligned > 0 && count_non_4_aligned >= count_4_aligned;
    has_indent_issues || has_compact_syntax || has_single_line_decl
}

/// Detect `name = expr` on a single line, where expr is non-empty and the
/// `=` is not part of `==`, `/=`, `<=`, `>=`. This is the shape elm-format
/// always expands into two lines inside doc-comment code blocks.
fn is_single_line_value_decl(trimmed: &str) -> bool {
    // Must start with a lowercase identifier character.
    let first = match trimmed.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    // Reject keyword-led lines: these are handled by the parser/printer
    // directly and don't fit the `name = expr` value-decl shape.
    let first_word_end = trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(trimmed.len());
    let first_word = &trimmed[..first_word_end];
    match first_word {
        "type" | "port" | "module" | "import" | "let" | "in" | "if" | "then"
        | "else" | "case" | "of" | "where" | "alias" | "exposing" | "as"
        | "effect" | "infix" => return false,
        _ => {}
    }
    // Find ` = ` that isn't part of `== `, `/= `, etc.
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b' ' && bytes[i + 1] == b'=' && bytes[i + 2] == b' ' {
            // Reject `== `, `/= `, `<= `, `>= ` (char before the `=` is an op-char).
            if i > 0 {
                let prev = bytes[i - 1];
                if prev == b'=' || prev == b'/' || prev == b'<' || prev == b'>'
                    || prev == b'!' || prev == b':'
                {
                    i += 1;
                    continue;
                }
            }
            // Reject `= =` (next char after `= ` is `=`).
            if i + 3 < bytes.len() && bytes[i + 3] == b'=' {
                i += 1;
                continue;
            }
            // Left side must be an identifier (plus optional argument pattern).
            let left = trimmed[..i].trim();
            if left.is_empty() {
                return false;
            }
            let left_first = left.chars().next().unwrap();
            if !(left_first.is_ascii_lowercase() || left_first == '_') {
                return false;
            }
            // Right side must be non-empty.
            let right = trimmed[i + 3..].trim();
            if right.is_empty() {
                return false;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Try to reformat a code block (lines starting with 4+ spaces) as Elm code.
///
/// Splits the code block at blank lines into "paragraphs", then tries each
/// as either a declaration (in a module) or an expression (wrapped in a dummy
/// function). Returns `Some(reformatted)` with 4-space-prefixed lines if all
/// paragraphs can be parsed, or `None` if any paragraph fails.
fn try_reformat_code_block(block_lines: &[&str]) -> Option<String> {
    // Strip the 4-space prefix from each line to get raw Elm code.
    let mut raw_lines: Vec<String> = Vec::new();
    for &line in block_lines {
        if line.trim().is_empty() {
            raw_lines.push(String::new());
        } else if line.starts_with("    ") {
            raw_lines.push(line[4..].to_string());
        } else {
            return None;
        }
    }

    let raw_code = raw_lines.join("\n");

    // First try: parse as a full module with declarations.
    let wrapped = format!("module DocTemp__ exposing (..)\n\n\n{}\n", raw_code);
    if let Some(result) = try_parse_and_format_module(&wrapped) {
        return Some(result);
    }

    // Second try: split into paragraphs (separated by blank lines) and
    // try each paragraph individually. Some may be expressions, some
    // declarations.
    let paragraphs = split_into_paragraphs(&raw_lines);
    let mut formatted_paragraphs: Vec<String> = Vec::new();

    for para in &paragraphs {
        let para_text = para.join("\n");

        // Try as declaration(s) first.
        let wrapped_decl = format!("module DocTemp__ exposing (..)\n\n\n{}\n", para_text);
        if let Some(result) = try_parse_and_format_module_raw(&wrapped_decl) {
            formatted_paragraphs.push(result);
            continue;
        }

        // Try as expression by wrapping in a dummy function.
        let indented: Vec<String> = para.iter().enumerate().map(|(i, line)| {
            if line.is_empty() {
                String::new()
            } else if i == 0 {
                format!("    {}", line)
            } else {
                format!("    {}", line)
            }
        }).collect();
        let wrapped_expr = format!(
            "module DocTemp__ exposing (..)\n\n\ndocTemp__ =\n{}\n",
            indented.join("\n")
        );
        if let Some(result) = try_parse_and_format_expr(&wrapped_expr) {
            formatted_paragraphs.push(result);
            continue;
        }

        // Third try: if every non-empty line in this paragraph looks like an
        // independent assertion (`expr == value`), parse each line as its own
        // expression and join with blank lines. elm-format renders these as
        // separate "top-level" expressions.
        if is_assertion_only_paragraph(para) {
            let mut per_line_results: Vec<String> = Vec::new();
            let mut all_ok = true;
            for line in para {
                if line.trim().is_empty() {
                    continue;
                }
                let wrapped_line = format!(
                    "module DocTemp__ exposing (..)\n\n\ndocTemp__ =\n    {}\n",
                    line
                );
                match try_parse_and_format_expr(&wrapped_line) {
                    Some(r) => per_line_results.push(r),
                    None => {
                        all_ok = false;
                        break;
                    }
                }
            }
            if all_ok && !per_line_results.is_empty() {
                formatted_paragraphs.push(per_line_results.join("\n\n"));
                continue;
            }
        }

        // Can't parse this paragraph — bail out entirely.
        return None;
    }

    // Join paragraphs with blank lines and re-indent with 4 spaces.
    let joined = formatted_paragraphs.join("\n\n");
    let mut output = String::new();
    for (idx, line) in joined.split('\n').enumerate() {
        if idx > 0 {
            output.push('\n');
        }
        if line.is_empty() {
            // Keep blank lines blank.
        } else {
            output.push_str("    ");
            output.push_str(line);
        }
    }

    Some(output)
}

/// Split raw lines into paragraphs separated by blank lines.
fn split_into_paragraphs(lines: &[String]) -> Vec<Vec<String>> {
    let mut paragraphs: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current);
                current = Vec::new();
            }
        } else {
            current.push(line.clone());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

/// Try to parse source as a module and extract formatted declarations.
/// Returns Some(re-indented string) or None.
fn try_parse_and_format_module(wrapped: &str) -> Option<String> {
    let raw = try_parse_and_format_module_raw(wrapped)?;

    // Collapse runs of 2+ consecutive blank lines to 1.
    let lines: Vec<&str> = raw.split('\n').collect();
    let mut collapsed: Vec<&str> = Vec::new();
    let mut prev_blank = false;
    for line in &lines {
        if line.is_empty() {
            if prev_blank {
                continue;
            }
            prev_blank = true;
        } else {
            prev_blank = false;
        }
        collapsed.push(line);
    }

    // Re-indent with 4 spaces.
    let mut output = String::new();
    for (idx, line) in collapsed.iter().enumerate() {
        if idx > 0 {
            output.push('\n');
        }
        if line.is_empty() {
            // blank
        } else {
            output.push_str("    ");
            output.push_str(line);
        }
    }

    Some(output)
}

/// Parse wrapped source as module and extract raw declaration text (no re-indenting).
fn try_parse_and_format_module_raw(wrapped: &str) -> Option<String> {
    let wrapped_owned = wrapped.to_string();
    let result = std::panic::catch_unwind(|| {
        let module = crate::parse::parse(&wrapped_owned).ok()?;
        let first = pretty_print(&module);
        // Idempotency check
        let module2 = crate::parse::parse(&first).ok()?;
        let second = pretty_print(&module2);
        if first == second { Some(first) } else { None }
    });
    let formatted = match result {
        Ok(Some(f)) => f,
        _ => return None,
    };

    // Extract everything after the module header line + blank lines.
    let header_end = formatted.find('\n')? + 1;
    let rest = &formatted[header_end..];
    let trimmed = rest.trim_start_matches('\n');
    if trimmed.is_empty() {
        return None;
    }
    let decl_text = trimmed.trim_end_matches('\n');
    if decl_text.is_empty() {
        return None;
    }
    Some(decl_text.to_string())
}

/// Parse wrapped source as a dummy function and extract the expression body.
fn try_parse_and_format_expr(wrapped: &str) -> Option<String> {
    let wrapped_owned = wrapped.to_string();
    let result = std::panic::catch_unwind(|| {
        let module = crate::parse::parse(&wrapped_owned).ok()?;
        let first = pretty_print(&module);
        // Idempotency check
        let module2 = crate::parse::parse(&first).ok()?;
        let second = pretty_print(&module2);
        if first == second { Some(first) } else { None }
    });
    let formatted = match result {
        Ok(Some(f)) => f,
        _ => return None,
    };

    // Extract the expression body from:
    // module DocTemp__ exposing (..)
    //
    //
    // docTemp__ =
    //     <expr>
    //
    // We need just the <expr> part, un-indented by 4 spaces.
    let marker = "docTemp__ =\n";
    let idx = formatted.find(marker)?;
    let body = &formatted[idx + marker.len()..];
    let body = body.trim_end_matches('\n');
    if body.is_empty() {
        return None;
    }

    // Remove 4-space indent from each line (the function body indent).
    let mut result_lines: Vec<String> = Vec::new();
    for line in body.split('\n') {
        if line.is_empty() {
            result_lines.push(String::new());
        } else if line.starts_with("    ") {
            result_lines.push(line[4..].to_string());
        } else {
            // Unexpected indentation — bail out.
            return None;
        }
    }

    Some(result_lines.join("\n"))
}

/// Parse `@docs` directives from a module documentation string.
/// Returns groups of names, one per `@docs` line.
///
/// Example: `" @docs Foo, bar, baz\n@docs quux"` → `[["Foo", "bar", "baz"], ["quux"]]`
fn parse_docs_groups(doc: &str) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut in_continuation = false;
    for line in doc.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@docs") {
            let names: Vec<String> = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !names.is_empty() {
                groups.push(names);
            }
            // If the line ends with a trailing comma, the next non-@docs
            // line is a continuation of this group.
            in_continuation = rest.trim_end().ends_with(',');
        } else if in_continuation && !trimmed.is_empty() && !trimmed.starts_with('#') {
            // Continuation line: push as a separate group (matching
            // elm-format's behavior of treating each continuation line
            // as its own @docs directive).
            let names: Vec<String> = trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !names.is_empty() {
                groups.push(names);
            }
            // If this continuation line also ends with comma, keep going.
            in_continuation = trimmed.ends_with(',');
        } else {
            in_continuation = false;
        }
    }
    groups
}

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

/// Flatten a left-associative operator chain into a list of expressions.
/// `a |> b |> c` (parsed as `(a |> b) |> c`) becomes `[a, b, c]`.
fn flatten_left_assoc_chain<'a>(expr: &'a Expr, target_op: &str) -> Option<Vec<&'a Expr>> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if operator == target_op => {
            let mut chain = match flatten_left_assoc_chain(&left.value, target_op) {
                Some(v) => v,
                None => vec![&left.value],
            };
            chain.push(&right.value);
            Some(chain)
        }
        _ => None,
    }
}

/// Flatten a right-associative operator chain into a list of expressions.
/// `a :: b :: c` (parsed as `a :: (b :: c)`) becomes `[a, b, c]`.
fn flatten_right_assoc_chain<'a>(expr: &'a Expr, target_op: &str) -> Option<Vec<&'a Expr>> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if operator == target_op => {
            let mut chain = vec![&left.value];
            match flatten_right_assoc_chain(&right.value, target_op) {
                Some(mut rest) => chain.append(&mut rest),
                None => chain.push(&right.value),
            }
            Some(chain)
        }
        _ => None,
    }
}

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
