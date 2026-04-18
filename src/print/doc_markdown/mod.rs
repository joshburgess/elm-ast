//! Markdown/doc-comment normalization for elm-format byte-match.
//!
//! Pure string transformations applied to doc comments (`{-| ... -}`) to
//! mirror Cheapskate markdown rendering: fenced code blocks, list rules,
//! emphasis, paragraph splitting, code-block reformat detection, and the
//! assertion-shape transforms used for Elm example code inside docs.

mod blocks;
mod markdown;
mod predicates;
mod reformat;
mod reparse;
mod spacing;

pub(super) use blocks::*;
pub(super) use markdown::*;
pub(super) use predicates::*;
pub(super) use reformat::*;
pub(super) use reparse::*;
pub(super) use spacing::*;

use super::{pretty_print, should_unicode_escape};

pub(super) fn normalize_doc_comment(text: &str) -> String {
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
        // Only un-indented lines are markdown headings. An indented `# ...`
        // (4+ leading spaces) is inside a code block, which in turn may be
        // inside a string literal where `#` is just content.
        let is_heading = (line.starts_with("# ")
            || line.starts_with("## ")
            || line.starts_with("### ")
            || line.starts_with("#### ")
            || line.starts_with("##### ")
            || line.starts_with("###### "))
            && !line.starts_with("    ");

        // Rule 4: Double blank line before any `# Heading` or `## Heading` etc.
        // If we see a markdown heading and the preceding output doesn't already
        // have a double blank line, add one.
        if is_heading {
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
            if is_heading {
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
pub(super) fn normalize_emphasis(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    let mut at_line_start = true;
    let mut line_indent = 0u32;
    let mut in_docs_line = false;
    let mut prev_line_blank = false;
    let mut current_line_has_content = false;
    let mut in_indented_code_block = false;

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
            // Track code-block enter/leave. A line at 4+ indent that starts
            // after a blank line opens a code block; the block continues on
            // subsequent 4+-indent lines regardless of blanks in between,
            // and ends at any non-blank line with less than 4-space indent.
            if line_indent >= 4 {
                if prev_line_blank || in_indented_code_block {
                    in_indented_code_block = true;
                }
            } else {
                in_indented_code_block = false;
            }
            // Detect @docs lines — skip emphasis processing on these.
            if text[i..].starts_with("@docs") {
                in_docs_line = true;
            }
            // Markdown unordered-list marker. elm-format's Cheapskate
            // renderer emits these as `- `. A leading `*` followed by a
            // space at the start of line content is a bullet, not
            // emphasis. Only apply outside of indented code blocks.
            if !in_indented_code_block
                && ch == b'*' && i + 1 < len && bytes[i + 1] == b' '
            {
                result.push('-');
                i += 1;
                continue;
            }
        }

        // On @docs lines, pass through unchanged (operators like (*) must not
        // have their `*` escaped or converted).
        if in_docs_line {
            result.push(ch as char);
            i += 1;
            continue;
        }

        // Inside an indented code block — pass through unchanged.
        if in_indented_code_block {
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
pub(super) fn normalize_empty_link_refs(text: &str) -> String {
    text.replace("][]", "]")
}

/// Collapse runs of 3+ consecutive newlines (2+ blank lines) to 2 newlines
/// (1 blank line) in doc-comment text, matching Cheapskate's paragraph
/// normalization. Headings intentionally get `\n\n\n` (two blanks) before
/// them by rule 4 of `normalize_doc_comment`, so preserve `\n\n\n` when the
/// following non-empty line is a markdown heading.
/// Normalize character-literal escapes in 4-space-indented doc code blocks.
/// elm-format lexes+reprints code blocks, which unescapes:
/// - `'\"'` -> `'"'` (double quote doesn't need escaping inside `'...'`)
/// - `'\u{XXXX}'` -> the literal character when `XXXX` is a printable
///   non-control codepoint (BMP or SMP).
pub(super) fn normalize_doc_char_literals(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for line in &lines {
        if line.starts_with("    ") {
            out.push(normalize_char_literals_in_code_line(line));
        } else {
            out.push((*line).to_string());
        }
    }
    out.join("\n")
}

pub(super) fn normalize_char_literals_in_code_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' && i + 3 < chars.len() {
            // Look for '\"' -> '"'
            if chars[i + 1] == '\\' && chars[i + 2] == '"' && chars[i + 3] == '\'' {
                out.push('\'');
                out.push('"');
                out.push('\'');
                i += 4;
                continue;
            }
            // Look for '\u{HEX}' -> actual char (when printable)
            if chars[i + 1] == '\\' && chars[i + 2] == 'u' && i + 3 < chars.len() && chars[i + 3] == '{' {
                let mut j = i + 4;
                while j < chars.len() && chars[j] != '}' {
                    j += 1;
                }
                if j < chars.len() && j + 1 < chars.len() && chars[j + 1] == '\'' {
                    let hex: String = chars[i + 4..j].iter().collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            if !ch.is_control() && !should_unicode_escape(ch) {
                                out.push('\'');
                                out.push(ch);
                                out.push('\'');
                                i = j + 2;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        // Inside a string literal, normalize \u{HEX} escapes to literal chars.
        if chars[i] == '"' {
            // Find end of string (unescaped " or end of line).
            let start = i;
            let mut j = i + 1;
            let mut buf = String::new();
            buf.push('"');
            while j < chars.len() {
                let c = chars[j];
                if c == '\\' && j + 1 < chars.len() {
                    let nx = chars[j + 1];
                    if nx == 'u' && j + 2 < chars.len() && chars[j + 2] == '{' {
                        let mut k = j + 3;
                        while k < chars.len() && chars[k] != '}' {
                            k += 1;
                        }
                        if k < chars.len() {
                            let hex: String = chars[j + 3..k].iter().collect();
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    if !ch.is_control() && !should_unicode_escape(ch) && ch != '"' && ch != '\\' {
                                        buf.push(ch);
                                        j = k + 1;
                                        continue;
                                    }
                                }
                            }
                        }
                        // Fall through — keep as-is.
                        buf.push(c);
                        buf.push(nx);
                        j += 2;
                        continue;
                    }
                    // Other escape: keep verbatim.
                    buf.push(c);
                    buf.push(nx);
                    j += 2;
                    continue;
                }
                buf.push(c);
                if c == '"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            // Only commit the buffer if we reached a closing quote.
            if buf.ends_with('"') && buf.len() > 1 {
                out.push_str(&buf);
                i = j;
                continue;
            }
            // Unterminated — fall back to raw copy.
            out.push(chars[start]);
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Collapse excess blank lines that directly precede a markdown
/// link-reference definition (`[name]: url`). Cheapskate normalizes any
/// run of 2+ blank lines before a link-reference block to a single blank
/// line. Other blank-line runs are preserved as-is (some block transitions
/// like code blocks rely on specific blank-line counts).
pub(super) fn collapse_blank_lines_in_doc(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let is_link_ref = |line: &str| -> bool {
        let t = line.trim_start();
        // Must start with '[', have ']:' somewhere after, and match the
        // pattern `[name]: rest`.
        if !t.starts_with('[') {
            return false;
        }
        if let Some(close) = t.find(']') {
            let after = &t[close + 1..];
            after.starts_with(": ") || after.starts_with(":\t")
        } else {
            false
        }
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            // Count consecutive blank lines.
            let mut j = i;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            let run = j - i;
            let next_is_link_ref = j < lines.len() && is_link_ref(lines[j]);
            let emit = if next_is_link_ref && run > 1 { 1 } else { run };
            for _ in 0..emit {
                out.push(String::new());
            }
            i = j;
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    out.join("\n")
}


/// Re-serialize `@docs` lines in doc comment text.
/// elm-format normalizes multi-line `@docs` with continuation lines
/// (after a trailing comma) into separate `@docs` directives. Each
/// continuation line becomes its own `@docs` line. The original `@docs`
/// line also has its trailing comma removed.
/// Ensure a blank line separates a prose paragraph from a following `@docs`
/// line. elm-format renders `@docs` as a block-level directive that needs a
/// blank line above when preceded by prose on the same paragraph.
pub(super) fn ensure_blank_before_docs_after_prose(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("@docs") && idx > 0 {
            let prev = lines[idx - 1];
            let prev_trimmed = prev.trim_start();
            // Skip if already separated by a blank or the prev is a heading,
            // another @docs, a list item, or a code-block line.
            let needs_blank = !prev.trim().is_empty()
                && !prev_trimmed.starts_with("@docs")
                && !prev_trimmed.starts_with('#')
                && !prev_trimmed.starts_with("- ")
                && !prev_trimmed.starts_with("* ")
                && !prev_trimmed.starts_with("\\* ")
                && !prev.starts_with("    ")
                && !prev_trimmed.starts_with("```");
            if needs_blank {
                out.push(String::new());
            }
        }
        out.push((*line).to_string());
    }
    out.join("\n")
}

pub(super) fn normalize_docs_lines(text: &str) -> String {
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
pub(super) fn strip_paragraph_leading_whitespace(text: &str) -> String {
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

/// Collapse runs of 2+ internal spaces to a single space in prose lines.
///
/// elm-format's Cheapskate markdown renderer normalizes internal whitespace
/// in prose paragraphs. This does NOT apply inside code blocks (4+ space
/// indent) or inside inline-code spans (backticks).
pub(super) fn collapse_prose_internal_spaces(text: &str) -> String {
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

        // Skip blockquote lines entirely: their internal whitespace may be
        // significant (continuation alignment, nested code blocks, etc.).
        if line.trim_start().starts_with('>') {
            result.push_str(line);
            continue;
        }

        // Skip ordered list items: elm-format uses double-space after the
        // period (`1.  text`), which our collapse would destroy.
        if strip_ordered_list_prefix(line.trim_start()).is_some() {
            result.push_str(line);
            continue;
        }

        // Preserve leading whitespace.
        let leading_len = line.len() - line.trim_start().len();
        result.push_str(&line[..leading_len]);

        // Walk the rest, collapsing 2+ spaces to 1, but preserving spaces
        // inside inline-code (`...`) spans.
        let rest = &line[leading_len..];
        let bytes = rest.as_bytes();
        let mut j = 0;
        let mut in_code_span = false;
        while j < bytes.len() {
            let b = bytes[j];
            if b == b'`' {
                in_code_span = !in_code_span;
                result.push('`');
                j += 1;
                continue;
            }
            if !in_code_span && b == b' ' {
                result.push(' ');
                j += 1;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                continue;
            }
            // Multi-byte UTF-8 safe: find next char boundary.
            if b < 128 {
                result.push(b as char);
                j += 1;
            } else {
                let ch_start = j;
                j += 1;
                while j < bytes.len() && (bytes[j] & 0b1100_0000) == 0b1000_0000 {
                    j += 1;
                }
                result.push_str(&rest[ch_start..j]);
            }
        }
    }
    result
}

/// Return true if a code-block line (already trim-started) looks like an
/// Elm top-level declaration: a type/alias declaration, an `infix` line,
/// a port, a type-annotation `name : ...`, or a value binding `name arg = ...`.
/// elm-format's Markdown renderer treats a code block that trails in a
/// `-- comment` line specially: it separates the block from the preceding
/// paragraph by two blank lines instead of one. This mirrors that spacing
/// so `pretty_print ∘ elm-format` is a no-op on such docs.
pub(super) fn ensure_blank_before_code_block_with_trailing_comment(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();

    // Scan for code blocks and mark those that both:
    //   (a) end with a trailing `-- comment` line, and
    //   (b) contain at least one declaration-like line
    //       (`type ...`, `foo : Type`, or `foo arg = ...`).
    // elm-format's Cheapskate-derived markdown renderer emits two blank
    // lines between the preceding paragraph and such blocks.
    let mut block_needs_extra_blank: Vec<bool> = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let starts_code = line.starts_with("    ")
            && !line.trim().is_empty()
            && (i == 0 || lines[i - 1].trim().is_empty());
        if !starts_code {
            i += 1;
            continue;
        }
        let block_start = i;
        let mut block_end = i;
        while block_end + 1 < lines.len() {
            let next = lines[block_end + 1];
            if next.trim().is_empty() {
                block_end += 1;
                continue;
            }
            if next.starts_with("    ") {
                block_end += 1;
                continue;
            }
            break;
        }
        // Walk back over trailing blank lines.
        let mut last_non_blank = block_end;
        while last_non_blank > block_start && lines[last_non_blank].trim().is_empty() {
            last_non_blank -= 1;
        }
        // Only count as a trailing comment if it sits at the block's base
        // indent (4 spaces), not at a deeper continuation indent like 8
        // spaces — at that depth, `--` is a comment attached to a preceding
        // expression, not a standalone trailing block line.
        let last_line = lines[last_non_blank];
        let last_leading = last_line.len() - last_line.trim_start().len();
        let ends_with_comment =
            last_leading == 4 && last_line.trim_start().starts_with("--");
        let starts_with_import =
            lines[block_start].trim_start().starts_with("import ");

        // Check that every 4-space-indented non-blank, non-comment line in
        // the block looks like a declaration (not a free-standing expression
        // call). elm-format only inserts the extra leading blank when the
        // block is structurally a sequence of declarations capped by a
        // comment — an expression at the base indent suppresses that spacing.
        let mut all_decls = true;
        let mut saw_any_decl = false;
        if ends_with_comment && !starts_with_import {
            for idx in block_start..=last_non_blank {
                let line = lines[idx];
                if line.trim().is_empty() {
                    continue;
                }
                let leading = line.len() - line.trim_start().len();
                if leading != 4 {
                    continue;
                }
                let t = line.trim_start();
                if t.starts_with("--") {
                    continue;
                }
                if looks_like_code_block_decl(t) {
                    saw_any_decl = true;
                } else {
                    all_decls = false;
                    break;
                }
            }
        }
        if ends_with_comment && saw_any_decl && all_decls && !starts_with_import {
            block_needs_extra_blank[block_start] = true;
        }
        i = block_end + 1;
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 8);
    for (i, line) in lines.iter().enumerate() {
        if block_needs_extra_blank[i] && i >= 2 {
            let prev = lines[i - 1];
            let prev2 = lines[i - 2];
            let prev_blank = prev.trim().is_empty();
            let prev2_prose = !prev2.trim().is_empty()
                && !prev2.starts_with(' ')
                && !prev2.starts_with('\t')
                && !prev2.starts_with('#')
                && !prev2.starts_with("- ")
                && !prev2.starts_with("* ")
                && !prev2.starts_with("\\* ")
                && !prev2.starts_with("@docs ")
                && !prev2.starts_with("```");
            if prev_blank && prev2_prose {
                let n = out.len();
                let already_double = n >= 2
                    && out[n - 1].trim().is_empty()
                    && out[n - 2].trim().is_empty();
                if !already_double {
                    out.push(String::new());
                }
            }
        }
        out.push((*line).to_string());
    }
    out.join("\n")
}

/// Strip trailing whitespace from each line in a doc comment.
///
/// elm-format removes trailing spaces from doc comment lines. We do the same
/// as a final normalization step. We must be careful not to strip trailing
/// whitespace from the very last part (the line ending before `-}`) since
/// that's structural.
pub(super) fn strip_trailing_whitespace_in_doc(text: &str) -> String {
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


