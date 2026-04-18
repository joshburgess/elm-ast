//! Block and paragraph inspectors for doc-comment code content.
//!
//! Walk a list of block lines (or a paragraph of `String`s) and report
//! shape: is this all comments, does it have a non-assertion line,
//! is it all imports, etc. Also hosts `parse_docs_groups`, the
//! `@docs` directive parser exposed to the parent printer.

use super::*;

/// Scan a doc code block for "assertion paragraphs" (runs of adjacent
/// non-blank lines whose trimmed form contains ` == `) and rewrite each such
/// paragraph so every assertion line becomes its own paragraph, with
/// multi-space runs collapsed outside of string literals. Other lines are
/// emitted unchanged. elm-format re-parses these blocks as expressions and
/// renders each on its own "top level", which produces this output.
pub(in crate::print) fn block_has_comment_paragraph(block_lines: &[&str]) -> bool {
    let mut paragraphs: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in block_lines {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    if paragraphs.len() < 2 {
        return false;
    }
    let last = paragraphs.last().unwrap();
    let last_is_all_comment = last.iter().all(|l| l.trim().starts_with("--"));
    if !last_is_all_comment {
        return false;
    }
    let first = &paragraphs[0];
    let first_line = first[0].trim();
    if first_line.starts_with("import ")
        || first_line.starts_with("--")
        || first_line.starts_with("module ")
    {
        return false;
    }
    first_line.starts_with("type ")
        || first_line.starts_with("port ")
        || looks_like_type_annotation(first_line)
        || looks_like_value_decl_start(first_line)
}

/// Detect a line that starts a value declaration: `name =` or `name args... =`.
/// Conservative: requires a lowercase identifier at the start followed by an
/// `=` at the outer level (not inside parens).
pub(in crate::print) fn block_is_all_comments(block_lines: &[&str]) -> bool {
    let mut saw_content = false;
    for line in block_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with("--") {
            return false;
        }
        saw_content = true;
    }
    saw_content
}

/// Return true if the code block contains a "preserving" piece of content,
/// either an import, or a standalone line comment paragraph that appears
/// after at least one assertion. In those situations elm-format's
/// reformatter leaves the block unchanged, so we should too.
pub(in crate::print) fn block_has_non_assertion_content(block_lines: &[&str]) -> bool {
    let mut seen_assertion = false;
    for (i, line) in block_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("import ") || trimmed.starts_with("module ") {
            return true;
        }
        if trimmed.starts_with("--") {
            // Standalone line-comment paragraph that comes after an
            // assertion: elm-format preserves the block unchanged.
            let prev_blank = i == 0 || block_lines[i - 1].trim().is_empty();
            let next_blank = i + 1 >= block_lines.len() || block_lines[i + 1].trim().is_empty();
            if seen_assertion && prev_blank && next_blank {
                return true;
            }
            continue;
        }
        if looks_like_assertion(trimmed) {
            seen_assertion = true;
            continue;
        }
        // Any other line shape bails out (prose, decl, etc.).
        return true;
    }
    false
}

/// Post-process a (pre-joined) code block, inserting extra blank lines
/// between certain paragraph pairs that elm-format renders "loose":
///   - all-imports paragraph followed by all-comments paragraph
///   - all-comments paragraph followed by all-imports paragraph
pub(in crate::print) fn insert_loose_paragraph_breaks(joined: &str) -> String {
    let lines: Vec<&str> = joined.split('\n').collect();

    // Split into paragraphs with their start indices.
    let mut paragraphs: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(idx);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    if paragraphs.len() < 2 {
        return joined.to_string();
    }

    let is_all_imports = |para: &Vec<usize>| -> bool {
        para.iter()
            .all(|&i| lines[i].trim_start().starts_with("import "))
    };
    let is_all_comments = |para: &Vec<usize>| -> bool {
        para.iter()
            .all(|&i| lines[i].trim_start().starts_with("--"))
    };

    // Indices (into `lines`) where an extra blank should be inserted BEFORE.
    // elm-format's Cheapskate inserts a double blank between an
    // imports-paragraph and a following comments-paragraph. A
    // comments-paragraph followed by imports gets only a single blank,
    // so don't insert in that direction (see Task.elm where a split-out
    // inline comment leads the paragraph).
    let mut extra_before: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for pair in paragraphs.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        let cur_start = cur[0];
        if is_all_imports(prev) && is_all_comments(cur) {
            extra_before.insert(cur_start);
        }
    }
    if extra_before.is_empty() {
        return joined.to_string();
    }

    let mut out = String::with_capacity(joined.len() + extra_before.len());
    for (idx, line) in lines.iter().enumerate() {
        if extra_before.contains(&idx) {
            out.push('\n');
        }
        out.push_str(line);
        if idx + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

pub(in crate::print) fn paragraph_is_all_imports(para: &[String]) -> bool {
    let mut saw = false;
    for line in para {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !t.starts_with("import ") {
            return false;
        }
        saw = true;
    }
    saw
}

pub(in crate::print) fn paragraph_starts_with_line_comment(para: &[String]) -> bool {
    for line in para {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        return t.starts_with("--");
    }
    false
}

/// Split raw lines into paragraphs separated by blank lines.
pub(in crate::print) fn split_into_paragraphs(lines: &[String]) -> Vec<Vec<String>> {
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

    // Merge consecutive paragraphs where the next paragraph's first line
    // begins with `-<digit/paren>`. elm-format parses the leading `-` as a
    // binary subtraction continuation of the previous paragraph's expression,
    // so the blank line between them is effectively ignored.
    let mut merged: Vec<Vec<String>> = Vec::new();
    for para in paragraphs {
        let is_minus_continuation = para.first().is_some_and(|first| {
            let t = first.trim();
            if let Some(rest) = t.strip_prefix('-') {
                rest.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '(')
            } else {
                false
            }
        });
        if is_minus_continuation && !merged.is_empty() {
            merged.last_mut().unwrap().extend(para);
        } else {
            merged.push(para);
        }
    }
    merged
}

/// Parse `@docs` directives from a module documentation string.
/// Returns groups of names, one per `@docs` line.
///
/// Example: `" @docs Foo, bar, baz\n@docs quux"` → `[["Foo", "bar", "baz"], ["quux"]]`
pub(in crate::print) fn parse_docs_groups(doc: &str) -> Vec<Vec<String>> {
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
