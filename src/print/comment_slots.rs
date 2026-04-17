//! Module-level comment slot assignment.
//!
//! At module scope, comments are rendered in the gap before each "anchor"
//! (an import or a top-level declaration), with one trailing slot for
//! comments that follow the last anchor. `CommentSlots` owns that per-slot
//! bucketing and the elm-format-specific hoist that moves import trailing
//! line comments onto the preceding import so the sorted import block stays
//! contiguous.

use crate::comment::Comment;
use crate::file::ElmModule;
use crate::import::Import;
use crate::node::Spanned;

pub(super) struct CommentSlots<'m> {
    /// `slots[i]` = comments belonging to the gap before anchor `i`.
    /// `slots[num_imports + num_decls]` is the trailing slot.
    pub slots: Vec<Vec<&'m Spanned<Comment>>>,
    pub num_imports: usize,
    pub num_decls: usize,
}

impl<'m> CommentSlots<'m> {
    /// Bucket each comment into the slot before its first following anchor.
    ///
    /// `skip_internal` controls whether comments whose offsets fall inside an
    /// anchor's span are dropped. Pretty mode drops them (they belong to the
    /// AST node itself, e.g. a block comment inside a record type); compact
    /// mode keeps them for round-trip fidelity.
    pub fn build(module: &'m ElmModule, skip_internal: bool) -> Self {
        let num_imports = module.imports.len();
        let num_decls = module.declarations.len();
        let total = num_imports + num_decls;

        let mut comments: Vec<&Spanned<Comment>> = module.comments.iter().collect();
        comments.sort_by_key(|c| c.span.start.offset);

        let mut anchor_offsets: Vec<usize> = Vec::with_capacity(total);
        let mut anchor_ends: Vec<usize> = Vec::with_capacity(total);
        for imp in &module.imports {
            anchor_offsets.push(imp.span.start.offset);
            anchor_ends.push(imp.span.end.offset);
        }
        for decl in &module.declarations {
            anchor_offsets.push(decl.span.start.offset);
            anchor_ends.push(decl.span.end.offset);
        }

        let mut slots: Vec<Vec<&'m Spanned<Comment>>> = vec![vec![]; total + 1];
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
                .unwrap_or(total);
            slots[slot].push(c);
        }

        Self {
            slots,
            num_imports,
            num_decls,
        }
    }

    pub fn trailing_slot(&self) -> usize {
        self.num_imports + self.num_decls
    }

    /// elm-format attaches a trailing line comment on an import as a leading
    /// comment on that same import after alphabetical sorting, so the import
    /// block renders contiguously. Mutate the slots in place to reflect that
    /// hoist: a comment currently in `slots[i]` whose source line equals the
    /// previous import's end line gets moved to `slots[i - 1]`, provided
    /// import `i - 1` is still followed by another import in sorted order.
    pub fn hoist_import_trailing_comments(&mut self, imports: &'m [Spanned<Import>]) {
        let num_imports = self.num_imports;
        if num_imports < 2 {
            return;
        }

        let mut sorted_for_hoist: Vec<usize> = (0..num_imports).collect();
        sorted_for_hoist.sort_by(|&a, &b| {
            imports[a]
                .value
                .module_name
                .value
                .cmp(&imports[b].value.module_name.value)
        });
        let mut sort_pos: Vec<usize> = vec![0; num_imports];
        for (pos, &src_idx) in sorted_for_hoist.iter().enumerate() {
            sort_pos[src_idx] = pos;
        }

        for i in 1..=num_imports {
            if self.slots[i].is_empty() {
                continue;
            }
            let prev_import = &imports[i - 1];
            let prev_end_line = prev_import.span.end.line;
            let (trailing, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.slots[i])
                .into_iter()
                .partition(|c| {
                    matches!(c.value, Comment::Line(_)) && c.span.start.line == prev_end_line
                });
            self.slots[i] = keep;
            if trailing.is_empty() {
                continue;
            }
            let p = sort_pos[i - 1];
            let followed_by_import = p + 1 < num_imports;
            if !followed_by_import {
                self.slots[i].extend(trailing);
                continue;
            }
            for c in trailing {
                self.slots[i - 1].push(c);
            }
            self.slots[i - 1].sort_by_key(|c| c.span.start.offset);
        }
    }
}
