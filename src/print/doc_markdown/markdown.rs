//! Markdown list & fenced-code-block normalization.
//!
//! Shape the lines of a doc comment to mirror Cheapskate's renderer:
//! list items get a 2-space indent, fenced code blocks are converted to
//! 4-space-indented blocks (except inside list continuation contexts),
//! and misaligned code-block indentation is normalized.

use super::*;

/// Normalize markdown list indentation in doc comments.
///
/// elm-format's Cheapskate markdown parser indents unordered list items
/// by 2 spaces: `- item` becomes `  - item`. This only applies to lines
/// that are NOT inside code blocks (4+ space indentation).
pub(in crate::print) fn normalize_markdown_lists(text: &str) -> String {
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
            if starts_list_after_prose(&lines, i, list_indent) {
                result.push('\n');
            }
            result.push_str("  ");
            result.push_str(&escape_bullet_leading_underscore(line, 2));
            // "  - " = 4 chars of prefix before content
            list_indent = Some(4);
        } else if line.starts_with("  - ") {
            // Already-indented unordered list item (common inside doc
            // comments where the body is rendered with no extra indent
            // but authors still visually indent bullets by 2 spaces).
            // Preserve the indent; continuation aligns 2 spaces past the
            // `- ` marker.
            if starts_list_after_prose(&lines, i, list_indent) {
                result.push('\n');
            }
            result.push_str(&escape_bullet_leading_underscore(line, 4));
            list_indent = Some(4);
        } else if let Some(rest) = strip_ordered_list_prefix(line) {
            // Ordered list item: strip leading spaces, double-space after period.
            // `  1. text` or `1. text` -> `1.  text`
            if starts_list_after_prose(&lines, i, list_indent) {
                result.push('\n');
            }
            let trimmed = line.trim_start();
            // Extract the number and period part
            let prefix_len = trimmed.len() - rest.len();
            let number_part = &trimmed[..prefix_len]; // e.g. "1. "
            let number_dot = number_part.trim_end(); // e.g. "1."
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

/// Escape word-boundary underscores in a bullet item's content.
/// Cheapskate (elm-format's markdown renderer) escapes `_word` → `\_word`
/// and `word_` → `word\_` because `_text_` is italic markdown.
/// Mid-word underscores (e.g. `foo_bar`) aren't flanking and are left alone.
/// Underscores inside `[link text]` are left as-is, since cheapskate
/// preserves emphasis inside link labels.
///
/// `marker_len` is the number of characters preceding the content in the
/// already-extended prefix form: e.g. for `- _blank`, marker_len is 2; for
/// `  - _blank`, marker_len is 4.
pub(in crate::print) fn escape_bullet_leading_underscore(line: &str, marker_len: usize) -> String {
    if line.len() <= marker_len {
        return line.to_string();
    }
    let (prefix, content) = line.split_at(marker_len);
    let bytes = content.as_bytes();
    // Pre-scan: if flanking underscores in the bullet content pair up as a
    // balanced italic span (even count, at least one pair), cheapskate treats
    // them as italic and emits them literally. Only unmatched flanking
    // underscores need to be escaped.
    if has_balanced_flanking_underscores(bytes) {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 2);
    out.push_str(prefix);
    let mut in_link_text = false;
    let mut in_backticks = false;
    let mut prev_raw: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // Non-ASCII: copy the whole UTF-8 sequence and advance by its length.
        if b >= 0x80 {
            let seq_len = utf8_seq_len(b);
            out.push_str(std::str::from_utf8(&bytes[i..i + seq_len]).unwrap_or(""));
            prev_raw = Some(b);
            i += seq_len;
            continue;
        }
        match b {
            b'[' if !in_link_text && !in_backticks => in_link_text = true,
            b']' if in_link_text => in_link_text = false,
            b'`' => in_backticks = !in_backticks,
            _ => {}
        }
        if b == b'_' && !in_link_text && !in_backticks {
            // Skip if already escaped (prev char is an unescaped backslash).
            let already_escaped = prev_raw == Some(b'\\');
            if !already_escaped {
                let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
                let next = if i + 1 < bytes.len() {
                    Some(bytes[i + 1])
                } else {
                    None
                };
                // Flanking check: either side is a word char (letter/digit),
                // and the other side is not a word char (boundary-ish).
                let left_is_letter = prev.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
                let right_is_letter = next.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
                if left_is_letter != right_is_letter {
                    out.push('\\');
                } else if !left_is_letter && !right_is_letter {
                    // `)_ ` or `)_` at end: cheapskate still treats these as
                    // potential delimiters if preceded by closing punctuation
                    // (non-whitespace) and followed by whitespace/EOL.
                    let prev_is_nonspace = prev.map(|c| !c.is_ascii_whitespace()).unwrap_or(false);
                    let next_is_space_or_none =
                        next.map(|c| c.is_ascii_whitespace()).unwrap_or(true);
                    let prev_is_space_or_none =
                        prev.map(|c| c.is_ascii_whitespace()).unwrap_or(true);
                    let next_is_nonspace = next.map(|c| !c.is_ascii_whitespace()).unwrap_or(false);
                    if (prev_is_nonspace && next_is_space_or_none)
                        || (prev_is_space_or_none && next_is_nonspace)
                    {
                        out.push('\\');
                    }
                }
            }
        }
        out.push(b as char);
        prev_raw = Some(b);
        i += 1;
    }
    out
}

/// Returns true when the bullet content has an even, nonzero number of
/// word-boundary flanking underscores — i.e. they pair up as markdown italic
/// spans. In that case cheapskate renders them verbatim and no escape is
/// needed. A single unmatched flanking underscore (e.g. `_blank foo`) must
/// still be escaped.
fn has_balanced_flanking_underscores(bytes: &[u8]) -> bool {
    let mut count = 0usize;
    let mut in_link_text = false;
    let mut in_backticks = false;
    let mut prev_raw: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b >= 0x80 {
            let seq_len = utf8_seq_len(b);
            prev_raw = Some(b);
            i += seq_len;
            continue;
        }
        match b {
            b'[' if !in_link_text && !in_backticks => in_link_text = true,
            b']' if in_link_text => in_link_text = false,
            b'`' => in_backticks = !in_backticks,
            _ => {}
        }
        if b == b'_' && !in_link_text && !in_backticks {
            let already_escaped = prev_raw == Some(b'\\');
            if !already_escaped {
                let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
                let next = if i + 1 < bytes.len() {
                    Some(bytes[i + 1])
                } else {
                    None
                };
                let left_is_letter = prev.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
                let right_is_letter = next.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
                let flanking = if left_is_letter != right_is_letter {
                    true
                } else if !left_is_letter && !right_is_letter {
                    let prev_is_nonspace =
                        prev.map(|c| !c.is_ascii_whitespace()).unwrap_or(false);
                    let next_is_space_or_none =
                        next.map(|c| c.is_ascii_whitespace()).unwrap_or(true);
                    let prev_is_space_or_none =
                        prev.map(|c| c.is_ascii_whitespace()).unwrap_or(true);
                    let next_is_nonspace =
                        next.map(|c| !c.is_ascii_whitespace()).unwrap_or(false);
                    (prev_is_nonspace && next_is_space_or_none)
                        || (prev_is_space_or_none && next_is_nonspace)
                } else {
                    false
                };
                if flanking {
                    count += 1;
                }
            }
        }
        prev_raw = Some(b);
        i += 1;
    }
    count >= 2 && count % 2 == 0
}

fn utf8_seq_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xC0 {
        // Continuation byte alone; treat as 1 to avoid infinite loop.
        1
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

/// Convert fenced code blocks (triple-backtick) to indented code blocks.
///
/// elm-format's Cheapskate markdown parser converts fenced code blocks to
/// 4-space indented code blocks. We do the same to match elm-format output.
pub(in crate::print) fn normalize_fenced_code_blocks(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        // Detect opening fence: plain ``` or ```<language-tag>.
        // elm-format's Cheapskate renderer converts all fenced blocks to
        // 4-space indented blocks, stripping the fences and language tag.
        // Cheapskate only converts fences with no language tag or the `elm`
        // language tag to indented code blocks. Fences tagged for other
        // languages (e.g. `javascript`) are left intact.
        let is_fence_open = trimmed == "```"
            || (trimmed.starts_with("```")
                && trimmed.len() > 3
                && !trimmed[3..].contains('`')
                && trimmed[3..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && trimmed[3..].eq_ignore_ascii_case("elm"));
        if is_fence_open {
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
                // If the fence is inside a list context, cheapskate keeps the
                // fence (does not convert to 4-space indent). Detect this by
                // scanning backward: a list item marker before any unindented
                // paragraph line means we're still in list continuation.
                let in_list_context = fence_is_in_list_context(&lines, i);

                if in_list_context {
                    // Preserve the fence as-is; fall through to default copy.
                } else {
                    // Convert: skip opening fence, indent content lines by 4
                    // spaces, skip closing fence.
                    #[allow(clippy::needless_range_loop)]
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
        }

        if i > 0 {
            result.push('\n');
        }
        result.push_str(lines[i]);
        i += 1;
    }
    result
}

/// Returns true if the fence opening at `fence_idx` is inside a markdown list
/// continuation. Scans backward through lines, skipping blank lines and
/// indented continuation text; if we encounter a list item marker before an
/// unindented paragraph-style line, the fence is in list context.
pub(in crate::print) fn fence_is_in_list_context(lines: &[&str], fence_idx: usize) -> bool {
    if fence_idx == 0 {
        return false;
    }
    let mut k = fence_idx;
    while k > 0 {
        k -= 1;
        let line = lines[k];
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        // List item marker
        if trimmed.starts_with("- ")
            || trimmed == "-"
            || strip_ordered_list_prefix(trimmed).is_some()
        {
            return true;
        }
        // Indented continuation line — keep walking back
        if indent >= 2 {
            continue;
        }
        // Unindented, non-list content ends the potential list scope
        return false;
    }
    false
}

/// Determine whether a list item line should be preceded by a blank line.
/// elm-format's Cheapskate markdown renderer separates a list from a preceding
/// paragraph with a blank line, even when the source had none.
pub(in crate::print) fn starts_list_after_prose(
    lines: &[&str],
    i: usize,
    list_indent: Option<usize>,
) -> bool {
    // Already inside a list context (previous item or continuation) — no blank.
    if list_indent.is_some() {
        return false;
    }
    if i == 0 {
        return false;
    }
    let prev = lines[i - 1];
    // Previous line blank → already separated.
    if prev.trim().is_empty() {
        return false;
    }
    let prev_trimmed = prev.trim_start();
    // Previous line is itself a list item (list_indent should have been set, but
    // be defensive).
    if prev_trimmed.starts_with("- ")
        || prev_trimmed == "-"
        || strip_ordered_list_prefix(prev_trimmed).is_some()
    {
        return false;
    }
    // Previous line is a heading or @docs — those act as block separators.
    if prev_trimmed.starts_with('#') || prev_trimmed.starts_with("@docs") {
        return false;
    }
    true
}

/// Check if a line is an ordered list item: optional whitespace, digits, period, space(s).
/// Returns the text after all spaces following "N.", or None.
pub(in crate::print) fn strip_ordered_list_prefix(line: &str) -> Option<&str> {
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
pub(in crate::print) fn normalize_code_block_indent(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());

    // Pre-pass: elm-format preserves ALL code blocks in a doc comment
    // verbatim when any one of them mixes declarations with bare expressions.
    // A later "example usage" block full of bare exprs + `-->` assertions is
    // enough to disqualify sibling decl-only blocks from reformatting.
    let doc_has_mixed_block = doc_comment_has_mixed_block(&lines);

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Check if this line starts a code block:
        // - must have 4+ leading spaces
        // - must be preceded by a blank line (or be the first line)
        let starts_code = line.starts_with("    ") && (i == 0 || lines[i - 1].trim().is_empty());

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
        let needs_reformat = !doc_has_mixed_block
            && code_block_needs_reformat(&lines[block_start..=block_end]);

        let reformatted = if needs_reformat {
            try_reformat_code_block(&lines[block_start..=block_end])
        } else {
            None
        };

        if let Some(reformatted) = reformatted {
            // When elm-format re-parses a doc code block containing both code
            // and a comment-only paragraph, it treats the block as "loose" and
            // inserts an extra blank line before the block.
            if block_has_comment_paragraph(&lines[block_start..=block_end]) {
                result.push('\n');
            }
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
            //
            // When the doc comment is an "example" (any block shows expected
            // output via `-->` or mixes decls with bare exprs), preserve every
            // block exactly as written. The assertion transform would
            // otherwise rewrite compact tuples and trim alignment spaces that
            // elm-format leaves alone.
            let block = &lines[block_start..=block_end];
            let transformed = if doc_has_mixed_block {
                block.join("\n")
            } else {
                transform_assertion_paragraphs(block)
            };
            let transformed = insert_loose_paragraph_breaks(&transformed);
            let end_idx = result.len();
            result.push_str(&transformed);
            let _ = end_idx;
            if block_end < lines.len() - 1 {
                result.push('\n');
            }
            // Code blocks containing only line comments (e.g. `-- foo`) get a
            // 3-blank-line separator before following content in elm-format's
            // Cheapskate output, not the usual 1. Force that here and skip the
            // source's own trailing blanks so they don't add extra newlines.
            if block_is_all_comments(block) {
                let mut k = block_end + 1;
                while k < lines.len() && lines[k].trim().is_empty() {
                    k += 1;
                }
                result.push('\n');
                result.push('\n');
                result.push('\n');
                i = k;
                continue;
            }
        }
        i = block_end + 1;
    }

    result
}

/// Pre-scan all 4-space-indented code blocks in a doc comment body and return
/// true if any block indicates the whole comment is an "example" that
/// elm-format preserves verbatim. Currently two signals qualify:
///
/// 1. A single block mixes declarations with bare expressions (e.g. a `foo :`
///    annotation plus a `foo 42` usage line).
/// 2. Any block contains one or more `-->` result-comment lines used to show
///    expected output of the preceding expression.
fn doc_comment_has_mixed_block(lines: &[&str]) -> bool {
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let starts_code = line.starts_with("    ") && (i == 0 || lines[i - 1].trim().is_empty());
        if !starts_code {
            i += 1;
            continue;
        }
        let block_start = i;
        let mut block_end = i;
        while block_end + 1 < lines.len() {
            let next = lines[block_end + 1];
            if next.trim().is_empty() {
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
        let block = &lines[block_start..=block_end];
        if super::reformat::block_mixes_decls_and_bare_exprs(block)
            || block_has_result_arrow_comment(block)
        {
            return true;
        }
        i = block_end + 1;
    }
    false
}

/// Returns true if any line in the block is a `-->` result-comment at base
/// (4-space) indent. elm-format treats these blocks as example output and
/// preserves them and their sibling blocks verbatim.
fn block_has_result_arrow_comment(block_lines: &[&str]) -> bool {
    for &line in block_lines {
        let trimmed = line.trim();
        if trimmed.starts_with("-->") {
            return true;
        }
    }
    false
}
