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
mod doc_markdown;

use block_comment::reindent_block_comment;
use doc_markdown::*;

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

        // Build anchor end-offsets for checking whether a comment falls
        // inside a declaration/import (in which case it's an internal
        // comment that shouldn't be emitted at the module level).
        let mut anchor_ends: Vec<usize> = Vec::with_capacity(total_anchors);
        for imp in &module.imports {
            anchor_ends.push(imp.span.end.offset);
        }
        for decl in &module.declarations {
            anchor_ends.push(decl.span.end.offset);
        }

        // Assign comments to slots: one per anchor + one trailing slot.
        // In pretty mode, skip comments that fall inside any anchor's span —
        // those are internal comments that belong to the AST node (e.g. block
        // comments inside a record type) and shouldn't be emitted at module
        // scope where they'd land in the wrong place.
        // In compact mode, keep them: round-trip correctness relies on every
        // comment being emitted somewhere so the re-parser recovers the same
        // comment count.
        let skip_internal = self.is_pretty();
        let mut anchor_comments: Vec<Vec<&Spanned<Comment>>> = vec![vec![]; total_anchors + 1];
        'outer: for c in &comments {
            let offset = c.span.start.offset;
            if skip_internal {
                for (a_start, a_end) in anchor_offsets.iter().zip(anchor_ends.iter()) {
                    if *a_start <= offset && offset < *a_end {
                        continue 'outer;
                    }
                }
            }
            let slot = anchor_offsets
                .iter()
                .position(|&a| a > offset)
                .unwrap_or(total_anchors);
            anchor_comments[slot].push(c);
        }

        // Hoist trailing line comments on imports that are followed by another
        // import in the *sorted* output. elm-format attaches such trailing
        // comments as leading comments on the preceding import so the imports
        // render as a contiguous block. A slot-i comment is a trailing comment
        // on import[i-1] iff (a) 1 <= i <= num_imports, (b) its source line
        // equals import[i-1]'s end line, and (c) slot i corresponds to another
        // import (i.e. i < num_imports).
        if self.is_pretty() && num_imports >= 2 {
            let mut sorted_for_hoist: Vec<usize> = (0..num_imports).collect();
            sorted_for_hoist.sort_by(|&a, &b| {
                module.imports[a]
                    .value
                    .module_name
                    .value
                    .cmp(&module.imports[b].value.module_name.value)
            });
            // Build a map: source-order index -> position in sorted output.
            let mut sort_pos: Vec<usize> = vec![0; num_imports];
            for (pos, &src_idx) in sorted_for_hoist.iter().enumerate() {
                sort_pos[src_idx] = pos;
            }
            for i in 1..=num_imports {
                if anchor_comments[i].is_empty() {
                    continue;
                }
                let prev_import = &module.imports[i - 1];
                let prev_end_line = prev_import.span.end.line;
                // Partition: trailing line-comments on prev_import vs others.
                let (trailing, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut anchor_comments[i])
                    .into_iter()
                    .partition(|c| {
                        matches!(c.value, Comment::Line(_))
                            && c.span.start.line == prev_end_line
                    });
                anchor_comments[i] = keep;
                if trailing.is_empty() {
                    continue;
                }
                // Only hoist when import (i-1) is followed by another import in
                // the sorted output. Hoist to slot for the FIRST import of the
                // sorted contiguous group that contains import (i-1).
                let p = sort_pos[i - 1];
                let followed_by_import = p + 1 < num_imports;
                if !followed_by_import {
                    // No hoist — put comments back where they were.
                    anchor_comments[i].extend(trailing);
                    continue;
                }
                let target_src_idx = sorted_for_hoist[0]; // leading slot of the sorted import block
                let _ = target_src_idx;
                // Hoist onto the source-order slot for import (i-1) itself so
                // it precedes that import in sorted output. Since the sort is
                // stable by module name, placing the comment in slot (i-1)
                // keeps it attached to the same import whose trailing comment
                // it was.
                for c in trailing {
                    anchor_comments[i - 1].push(c);
                }
                // Re-sort anchor_comments[i-1] by original offset.
                anchor_comments[i - 1].sort_by_key(|c| c.span.start.offset);
            }
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
                    // elm-format separates leading orphan comments from the
                    // import line with a blank line.
                    let mut had_comments = false;
                    for &idx in &sorted_indices[i..group_end] {
                        if !anchor_comments[idx].is_empty() {
                            had_comments = true;
                            for c in &anchor_comments[idx] {
                                self.write_comment(&c.value);
                                self.newline();
                            }
                        }
                    }
                    if had_comments {
                        self.newline();
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

            // If the first "leading" comment is a line comment on the same
            // source line as the previous declaration's end, it's really a
            // trailing comment on that previous decl. Emit it inline now
            // (on the same line as the just-written decl) and drop it from
            // the leading list.
            let inline_trailing = self.is_pretty()
                && i > 0
                && !anchor_comments[slot].is_empty()
                && matches!(&anchor_comments[slot][0].value, Comment::Line(_))
                && {
                    let c0 = &anchor_comments[slot][0];
                    let prev_end_line = module.declarations[i - 1].span.end.line;
                    c0.span.start.line == prev_end_line
                };
            if inline_trailing {
                self.write_char(' ');
                self.write_comment(&anchor_comments[slot][0].value);
            }
            let skip_first = if inline_trailing { 1 } else { 0 };
            let remaining: Vec<_> = anchor_comments[slot].iter().skip(skip_first).collect();

            // Emit leading comments for this declaration.
            if !remaining.is_empty() {
                // elm-format treats a leading line comment as a "section
                // header" with 3 blank lines before (between decls / after
                // imports) and 2 blank lines after. Block comments preserve
                // the number of blank lines from the source.
                let is_section = matches!(
                    &remaining[0].value,
                    Comment::Line(_)
                );
                // A single empty block comment (e.g. `{--}`) is treated by
                // elm-format as an attached marker: normal 2-blank-line
                // separation before, and no blank line between it and the
                // following decl/doc-comment.
                let is_attached_marker = remaining.len() == 1
                    && matches!(
                        &remaining[0].value,
                        Comment::Block(text) if text.trim().is_empty()
                            || text.trim().chars().all(|c| c == '-')
                    );
                if self.is_pretty() {
                    if is_section {
                        if i > 0 {
                            // 4 newlines from end of prev decl = 3 blank lines.
                            self.newline();
                            self.newline();
                            self.newline();
                            self.newline();
                        } else if num_imports > 0 {
                            // Cursor already on new line after last import's
                            // trailing newline. 3 newlines = 3 blank lines.
                            self.newline();
                            self.newline();
                            self.newline();
                        } else {
                            // After module header/doc: 1 blank line.
                            self.newline();
                        }
                    } else if is_attached_marker {
                        // Empty block comment marker: 2 blank lines before
                        // (normal decl separation), matching elm-format.
                        if i > 0 {
                            self.newline();
                            self.newline();
                            self.newline();
                        } else if num_imports > 0 {
                            self.newline();
                            self.newline();
                        } else {
                            self.newline();
                        }
                    } else {
                        // Block comment: preserve source blank-line count,
                        // clamped to elm-format's minimums.
                        let first_comment_line =
                            remaining[0].span.start.line;
                        let prev_end_line: u32 = if i > 0 {
                            module.declarations[i - 1].span.end.line
                        } else if num_imports > 0 {
                            module.imports[num_imports - 1].span.end.line
                        } else if let Some(doc) = &module.module_documentation {
                            doc.span.end.line
                        } else {
                            module.header.span.end.line
                        };
                        let source_blanks = first_comment_line
                            .saturating_sub(prev_end_line + 1);
                        let min_blanks = if i == 0 && num_imports == 0 {
                            1u32
                        } else if i > 0 {
                            3u32
                        } else {
                            2u32
                        };
                        let blanks = source_blanks.max(min_blanks);
                        let newlines = if i > 0 { blanks + 1 } else { blanks };
                        for _ in 0..newlines {
                            self.newline();
                        }
                    }
                } else {
                    self.newline();
                    self.newline();
                }
                for c in &remaining {
                    self.write_comment(&c.value);
                    self.newline();
                }
                if self.is_pretty() && is_attached_marker {
                    // Marker comment attaches directly to the following decl.
                } else {
                    // elm-format puts 2 blank lines after a leading comment
                    // (same spacing as between declarations).
                    self.newline();
                    self.newline();
                }
            } else if infix_group {
                // No extra blank lines between consecutive infix declarations.
                self.newline();
            } else {
                self.newline();
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
            // If the first trailing comment is a line comment on the same
            // source line as the last declaration's end, emit it inline.
            let inline_trailing_orphan = self.is_pretty()
                && !module.declarations.is_empty()
                && matches!(
                    &anchor_comments[total_anchors][0].value,
                    Comment::Line(_)
                )
                && {
                    let c0 = &anchor_comments[total_anchors][0];
                    let last_decl = module.declarations.last().unwrap();
                    c0.span.start.line == last_decl.span.end.line
                };
            let skip_first = if inline_trailing_orphan { 1 } else { 0 };
            if inline_trailing_orphan {
                self.write_char(' ');
                self.write_comment(&anchor_comments[total_anchors][0].value);
            }
            let trailing: Vec<_> = anchor_comments[total_anchors]
                .iter()
                .skip(skip_first)
                .collect();
            if !trailing.is_empty() {
                self.newline();
                self.newline();
                // elm-format places 3 blank lines between the last declaration
                // and a trailing orphan comment (vs 2 blank lines between decls).
                if self.is_pretty() {
                    self.newline();
                    self.newline();
                }
                for (i, c) in trailing.iter().enumerate() {
                    if i > 0 {
                        self.newline();
                    }
                    self.write_comment(&c.value);
                }
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
                    // elm-format normalizes `{-- foo...` (block content
                    // starting with "- ") by dropping the space after the
                    // leading dash, keeping `{--` as a single marker.
                    let reindented = if self.is_pretty()
                        && reindented.starts_with("- ")
                    {
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
                {
                    if let Some((head, rest)) = flatten_mixed_pipe_chain(expr) {
                        let any_ml = self.is_multiline(head)
                            || rest.iter().any(|(_, op)| self.is_multiline(op));
                        if any_ml {
                            // Break ALL operators to vertical.
                            self.write_expr_operand(head, operator, true);
                            self.indent();
                            for (op, operand) in &rest {
                                self.newline_indent();
                                self.write(op);
                                self.write_char(' ');
                                self.write_expr_operand(operand, op, false);
                            }
                            self.dedent();
                            return;
                        }
                        // All operands are single-line; fall through to
                        // normal inline path (which handles recursion).
                    }
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
                {
                    if let Some((head, rest)) = flatten_mixed_cons_append_chain(expr) {
                        let any_ml = self.is_multiline(head)
                            || rest.iter().any(|(_, e)| self.is_multiline(e));
                        if any_ml {
                            self.write_expr_operand(head, operator, true);
                            self.indent();
                            for (op, operand) in &rest {
                                self.newline_indent();
                                self.write(op);
                                self.write_char(' ');
                                self.write_expr_operand(operand, op, false);
                            }
                            self.dedent();
                            return;
                        }
                    }
                }

                if self.is_pretty()
                    && matches!(operator.as_str(), ">>" | "<<")
                {
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

                // Same rule for left-associative arithmetic chains (+, -).
                // elm-format: if any operand in the chain is multiline,
                // break at every operator.
                if self.is_pretty()
                    && matches!(operator.as_str(), "+" | "-")
                {
                    let op_owned = operator.clone();
                    if let Some((head, rest)) = flatten_left_assoc_pred(
                        expr,
                        &|o: &str| o == op_owned,
                    ) {
                        let any_ml = self.is_multiline(head)
                            || rest.iter().any(|(_, op)| self.is_multiline(op));
                        if any_ml {
                            self.write_expr_operand(head, operator, true);
                            self.indent();
                            for (op, operand) in &rest {
                                self.newline_indent();
                                self.write(op);
                                self.write_char(' ');
                                self.write_expr_operand(operand, op, false);
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
            // When the scrutinee is a control-flow compound expression that
            // forces multi-line output (if/let/case), elm-format uses the
            // "hanging" form: bare "case", indented subject, and bare "of".
            let hanging = matches!(
                subject,
                Expr::IfElse { .. } | Expr::LetIn { .. } | Expr::CaseOf { .. }
            );
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
                if s.contains('.') {
                    self.write(&s);
                } else if let Some(e_pos) = s.find(|c: char| c == 'e' || c == 'E') {
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

/// Check whether a left-associative operator chain spans multiple source
/// lines — i.e. any two adjacent operands in the chain start on different
/// lines in the source. Used to force vertical pipeline layout when the
/// source already had the pipeline broken across lines.
fn left_chain_spans_multiple_lines(expr: &Expr, target_op: &str) -> bool {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if operator == target_op => {
            if left.span.end.line != right.span.start.line {
                return true;
            }
            left_chain_spans_multiple_lines(&left.value, target_op)
        }
        _ => false,
    }
}

/// Flatten a mixed pipe chain (`|>`, `|.`, `|=`) into a list of
/// `(operand, operator)` pairs plus the first operand.
/// Returns `None` if `expr` is not a pipe-chain. Returns the initial
/// operand and a list of (op, operand) pairs representing the chain.
fn flatten_mixed_pipe_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
    fn is_pipe(op: &str) -> bool {
        matches!(op, "|>" | "|." | "|=")
    }
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if is_pipe(operator) => {
            let (head, mut tail) =
                flatten_mixed_pipe_chain(&left.value).unwrap_or((&left.value, Vec::new()));
            tail.push((operator.as_str(), &right.value));
            Some((head, tail))
        }
        _ => None,
    }
}

/// Flatten a single-operator left-associative chain, carrying operator
/// text for each step (so it can be reused by other callers that want
/// heterogeneous chains). Accepts a predicate for which operators start/
/// continue the chain.
fn flatten_left_assoc_pred<'a>(
    expr: &'a Expr,
    pred: &impl Fn(&str) -> bool,
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if pred(operator) => {
            let (head, mut tail) = flatten_left_assoc_pred(&left.value, pred)
                .unwrap_or((&left.value, Vec::new()));
            tail.push((operator.as_str(), &right.value));
            Some((head, tail))
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

/// Flatten a right-associative chain where operators may mix between `::` and
/// `++` (same precedence 5, right-associative in Elm). Returns the head operand
/// and a list of (operator, operand) pairs. elm-format treats such chains as
/// one unified vertical layout.
fn flatten_mixed_cons_append_chain<'a>(
    expr: &'a Expr,
) -> Option<(&'a Expr, Vec<(&'a str, &'a Expr)>)> {
    fn is_cons_or_append(op: &str) -> bool {
        matches!(op, "::" | "++")
    }
    match expr {
        Expr::OperatorApplication {
            operator,
            left,
            right,
            ..
        } if is_cons_or_append(operator) => {
            let mut rest: Vec<(&'a str, &'a Expr)> = Vec::new();
            let (head, tail_rest) =
                match flatten_mixed_cons_append_chain(&right.value) {
                    Some((head, rest_r)) => {
                        rest.push((operator.as_str(), head));
                        for (op, e) in rest_r {
                            rest.push((op, e));
                        }
                        (&left.value, rest)
                    }
                    None => {
                        rest.push((operator.as_str(), &right.value));
                        (&left.value, rest)
                    }
                };
            Some((head, tail_rest))
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
