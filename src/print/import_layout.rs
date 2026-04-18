//! Import block layout plan for pretty-print mode.
//!
//! elm-format sorts top-level imports alphabetically by module name and
//! merges consecutive imports with the same module name into one. This
//! module turns the raw import list into a list of groups that the
//! module emitter can walk directly.

use crate::import::Import;
use crate::node::Spanned;

pub(super) struct ImportGroup {
    /// Source indices of imports in this group, in sorted order. Groups with
    /// a single index render as a normal import; multi-index groups are
    /// merged (same module name with different exposing lists).
    pub src_indices: Vec<usize>,
}

/// Sort imports alphabetically and group consecutive entries that share a
/// module name.
pub(super) fn build_import_plan(imports: &[Spanned<Import>]) -> Vec<ImportGroup> {
    let num_imports = imports.len();
    let mut sorted_indices: Vec<usize> = (0..num_imports).collect();
    sorted_indices.sort_by(|&a, &b| {
        imports[a]
            .value
            .module_name
            .value
            .cmp(&imports[b].value.module_name.value)
    });

    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut i = 0;
    while i < sorted_indices.len() {
        let mod_name = &imports[sorted_indices[i]].value.module_name.value;
        let mut group_end = i + 1;
        while group_end < sorted_indices.len()
            && imports[sorted_indices[group_end]].value.module_name.value == *mod_name
        {
            group_end += 1;
        }
        groups.push(ImportGroup {
            src_indices: sorted_indices[i..group_end].to_vec(),
        });
        i = group_end;
    }
    groups
}
