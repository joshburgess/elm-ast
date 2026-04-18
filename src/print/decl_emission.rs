//! Top-level declaration emission for `Printer::write_module`.
//!
//! Walks the declaration list, emitting each with the right leading-comment
//! treatment: "section header" line comments get 3 blank lines before and 2
//! after, attached markers (`{--}` / `{-- -}`) sit directly on the following
//! decl, block comments preserve source blank-line counts, and a trailing
//! line comment on the same source line as the previous decl is emitted
//! inline. Also handles the run of consecutive `infix` decls (no blank
//! lines between) and the trailing-orphan-comment block after the last
//! declaration.

use crate::comment::Comment;
use crate::declaration::Declaration;
use crate::file::ElmModule;

use super::comment_slots::CommentSlots;
use super::Printer;

impl Printer {
    pub(super) fn write_declarations_with_comments(
        &mut self,
        module: &ElmModule,
        slots: &CommentSlots,
        num_imports: usize,
    ) {
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
                && !slots.slots[slot].is_empty()
                && matches!(&slots.slots[slot][0].value, Comment::Line(_))
                && {
                    let c0 = &slots.slots[slot][0];
                    let prev_end_line = module.declarations[i - 1].span.end.line;
                    c0.span.start.line == prev_end_line
                };
            if inline_trailing {
                self.write_char(' ');
                self.write_comment(&slots.slots[slot][0].value);
            }
            let skip_first = if inline_trailing { 1 } else { 0 };
            let remaining: Vec<_> = slots.slots[slot].iter().skip(skip_first).collect();

            if !remaining.is_empty() {
                // elm-format treats a leading line comment as a "section
                // header" with 3 blank lines before (between decls / after
                // imports) and 2 blank lines after. Block comments preserve
                // the number of blank lines from the source.
                let is_section = matches!(&remaining[0].value, Comment::Line(_));
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
                        let first_comment_line = remaining[0].span.start.line;
                        let prev_end_line: u32 = if i > 0 {
                            module.declarations[i - 1].span.end.line
                        } else if num_imports > 0 {
                            module.imports[num_imports - 1].span.end.line
                        } else if let Some(doc) = &module.module_documentation {
                            doc.span.end.line
                        } else {
                            module.header.span.end.line
                        };
                        let source_blanks =
                            first_comment_line.saturating_sub(prev_end_line + 1);
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
    }

    pub(super) fn write_trailing_orphan_comments(
        &mut self,
        module: &ElmModule,
        slots: &CommentSlots,
        total_anchors: usize,
    ) {
        if slots.slots[total_anchors].is_empty() {
            return;
        }
        // If the first trailing comment is a line comment on the same source
        // line as the last declaration's end, emit it inline.
        let inline_trailing_orphan = self.is_pretty()
            && !module.declarations.is_empty()
            && matches!(
                &slots.slots[total_anchors][0].value,
                Comment::Line(_)
            )
            && {
                let c0 = &slots.slots[total_anchors][0];
                let last_decl = module.declarations.last().unwrap();
                c0.span.start.line == last_decl.span.end.line
            };
        let skip_first = if inline_trailing_orphan { 1 } else { 0 };
        if inline_trailing_orphan {
            self.write_char(' ');
            self.write_comment(&slots.slots[total_anchors][0].value);
        }
        let trailing: Vec<_> = slots.slots[total_anchors]
            .iter()
            .skip(skip_first)
            .collect();
        if trailing.is_empty() {
            return;
        }
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
