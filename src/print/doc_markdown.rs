//! Markdown/doc-comment normalization for elm-format byte-match.
//!
//! Pure string transformations applied to doc comments (`{-| ... -}`) to
//! mirror Cheapskate markdown rendering: fenced code blocks, list rules,
//! emphasis, paragraph splitting, code-block reformat detection, and the
//! assertion-shape transforms used for Elm example code inside docs.

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
pub(super) fn looks_like_code_block_decl(line: &str) -> bool {
    if line.starts_with("type ")
        || line.starts_with("type alias ")
        || line.starts_with("port ")
        || line.starts_with("infix ")
    {
        return true;
    }
    // `name : ...` — first token is a lowercase identifier and the rest
    // of the line starts with ` : `.
    let mut chars = line.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    let mut idx = first.len_utf8();
    while idx < line.len() {
        let c = line.as_bytes()[idx] as char;
        if c.is_ascii_alphanumeric() || c == '_' || c == '\'' {
            idx += 1;
        } else {
            break;
        }
    }
    let rest = &line[idx..];
    // `name :` — type annotation.
    if rest.starts_with(" : ") || rest == " :" {
        return true;
    }
    // `name ... = ...` — value binding, but only if the `=` sits at the
    // top level (not inside parens, brackets, braces, or a record literal).
    let bytes = rest.as_bytes();
    let mut depth_round: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut depth_curly: i32 = 0;
    let mut in_string = false;
    let mut in_char = false;
    let mut j = 0;
    while j < bytes.len() {
        let b = bytes[j];
        if in_string {
            if b == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            j += 1;
            continue;
        }
        if in_char {
            if b == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if b == b'\'' {
                in_char = false;
            }
            j += 1;
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'\'' => in_char = true,
            b'(' => depth_round += 1,
            b')' => depth_round -= 1,
            b'[' => depth_square += 1,
            b']' => depth_square -= 1,
            b'{' => depth_curly += 1,
            b'}' => depth_curly -= 1,
            b'=' if depth_round == 0 && depth_square == 0 && depth_curly == 0 => {
                let prev = if j > 0 { bytes[j - 1] as char } else { ' ' };
                let next = if j + 1 < bytes.len() { bytes[j + 1] as char } else { ' ' };
                // Exclude `==`, `/=`, `>=`, `<=`.
                if prev != '=' && prev != '/' && prev != '>' && prev != '<'
                    && next != '='
                {
                    return true;
                }
            }
            _ => {}
        }
        j += 1;
    }
    false
}

/// Insert an extra blank line before an indented code block when that block
/// ends with a `-- line comment` whose only trailing lines are blanks.
///
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

/// Normalize markdown list indentation in doc comments.
///
/// elm-format's Cheapskate markdown parser indents unordered list items
/// by 2 spaces: `- item` becomes `  - item`. This only applies to lines
/// that are NOT inside code blocks (4+ space indentation).
pub(super) fn normalize_markdown_lists(text: &str) -> String {
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
pub(super) fn escape_bullet_leading_underscore(line: &str, marker_len: usize) -> String {
    if line.len() <= marker_len {
        return line.to_string();
    }
    let (prefix, content) = line.split_at(marker_len);
    let bytes = content.as_bytes();
    let mut out = String::with_capacity(line.len() + 2);
    out.push_str(prefix);
    let mut in_link_text = false;
    let mut prev_raw: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' if !in_link_text => in_link_text = true,
            b']' if in_link_text => in_link_text = false,
            _ => {}
        }
        if b == b'_' && !in_link_text {
            // Skip if already escaped (prev char is an unescaped backslash).
            let already_escaped = prev_raw == Some(b'\\');
            if !already_escaped {
                let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
                let next = if i + 1 < bytes.len() { Some(bytes[i + 1]) } else { None };
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
                    let next_is_nonspace =
                        next.map(|c| !c.is_ascii_whitespace()).unwrap_or(false);
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
    }
    out
}

/// Convert fenced code blocks (triple-backtick) to indented code blocks.
///
/// elm-format's Cheapskate markdown parser converts fenced code blocks to
/// 4-space indented code blocks. We do the same to match elm-format output.
pub(super) fn normalize_fenced_code_blocks(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        // Detect opening fence: plain ``` or ```<language-tag>.
        // elm-format's Cheapskate renderer converts all fenced blocks to
        // 4-space indented blocks, stripping the fences and language tag.
        let is_fence_open = trimmed == "```"
            || (trimmed.starts_with("```")
                && trimmed.len() > 3
                && !trimmed[3..].contains('`')
                && trimmed[3..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
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
pub(super) fn fence_is_in_list_context(lines: &[&str], fence_idx: usize) -> bool {
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
pub(super) fn starts_list_after_prose(lines: &[&str], i: usize, list_indent: Option<usize>) -> bool {
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
pub(super) fn strip_ordered_list_prefix(line: &str) -> Option<&str> {
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
pub(super) fn normalize_code_block_indent(text: &str) -> String {
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
            let block = &lines[block_start..=block_end];
            let transformed = transform_assertion_paragraphs(block);
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

/// Scan a doc code block for "assertion paragraphs" (runs of adjacent
/// non-blank lines whose trimmed form contains ` == `) and rewrite each such
/// paragraph so every assertion line becomes its own paragraph, with
/// multi-space runs collapsed outside of string literals. Other lines are
/// emitted unchanged. elm-format re-parses these blocks as expressions and
/// renders each on its own "top level", which produces this output.
pub(super) fn block_has_comment_paragraph(block_lines: &[&str]) -> bool {
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
    let last_is_all_comment = last
        .iter()
        .all(|l| l.trim().starts_with("--"));
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
pub(super) fn looks_like_value_decl_start(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first == b'_') {
        return false;
    }
    // Walk identifier chars.
    let mut i = 0;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'\'')
    {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return false;
    }
    if bytes[i] != b' ' {
        return false;
    }
    // Scan for an `=` (surrounded by spaces) at outer level.
    let mut depth = 0i32;
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    while i < bytes.len() {
        let b = bytes[i];
        if esc { esc = false; i += 1; continue; }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            i += 1; continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            i += 1; continue;
        }
        match b {
            b'"' => in_str = true,
            b'\'' => in_char = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                // Ensure it's not `==`, `>=`, `<=`, `/=`, `=>`, `::=`.
                let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                let next = if i + 1 < bytes.len() { bytes[i + 1] } else { b' ' };
                if prev != b' ' { i += 1; continue; }
                if next == b'=' { i += 1; continue; }
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

pub(super) fn looks_like_type_annotation(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if escape {
            escape = false;
        } else if in_string {
            if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == b':' && i + 1 < bytes.len() && bytes[i + 1] == b' '
            && i > 0 && bytes[i - 1] == b' '
        {
            return true;
        }
        i += 1;
    }
    false
}

pub(super) fn block_is_all_comments(block_lines: &[&str]) -> bool {
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
pub(super) fn block_has_non_assertion_content(block_lines: &[&str]) -> bool {
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
            let next_blank = i + 1 >= block_lines.len()
                || block_lines[i + 1].trim().is_empty();
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
pub(super) fn insert_loose_paragraph_breaks(joined: &str) -> String {
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
        para.iter().all(|&i| lines[i].trim_start().starts_with("import "))
    };
    let is_all_comments = |para: &Vec<usize>| -> bool {
        para.iter().all(|&i| lines[i].trim_start().starts_with("--"))
    };

    // Indices (into `lines`) where an extra blank should be inserted BEFORE.
    let mut extra_before: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for pair in paragraphs.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        let cur_start = cur[0];
        let prev_imports_cur_comments = is_all_imports(prev) && is_all_comments(cur);
        let prev_comments_cur_imports = is_all_comments(prev) && is_all_imports(cur);
        if prev_imports_cur_comments || prev_comments_cur_imports {
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

pub(super) fn transform_assertion_paragraphs(block_lines: &[&str]) -> String {
    // Pre-process: merge standalone `...` lines into the previous assertion
    // as a trailing ` ...`. elm-format treats
    //     expr1 == val1
    //     ...
    //     expr2 == val2
    // identically to
    //     expr1 == val1 ...
    //     expr2 == val2
    // The existing chain logic handles the trailing-dots form.
    let merged_owned: Vec<String>;
    let block_lines: Vec<&str> = {
        let mut out: Vec<String> = Vec::with_capacity(block_lines.len());
        let mut i = 0;
        let orig = block_lines;
        while i < orig.len() {
            let line = orig[i];
            let trimmed = line.trim();
            if trimmed == "..." && !out.is_empty() {
                // Find the last non-blank line in `out` and append ` ...`.
                let mut last_idx = out.len();
                while last_idx > 0 && out[last_idx - 1].trim().is_empty() {
                    last_idx -= 1;
                }
                if last_idx > 0 {
                    let last = &out[last_idx - 1];
                    let last_trimmed = last.trim();
                    if !last_trimmed.is_empty() && !last_trimmed.starts_with("--")
                        && !last_trimmed.ends_with(" ...")
                    {
                        out[last_idx - 1] = format!("{} ...", last.trim_end());
                        // Drop any blank lines between assertion and `...`, and
                        // drop any blank lines between `...` and the next line,
                        // so the three lines become one. Skip trailing blanks
                        // after the `...` line.
                        while out.len() > last_idx && out.last().unwrap().trim().is_empty() {
                            out.pop();
                        }
                        let mut j = i + 1;
                        while j < orig.len() && orig[j].trim().is_empty() {
                            j += 1;
                        }
                        i = j;
                        continue;
                    }
                }
            }
            out.push(line.to_string());
            i += 1;
        }
        merged_owned = out;
        merged_owned.iter().map(|s| s.as_str()).collect()
    };
    let block_lines: &[&str] = &block_lines;

    // If the block contains anything other than pure assertion lines,
    // elm-format does not split adjacent assertions in this block. Emit
    // the block unchanged in that case.
    if block_has_non_assertion_content(block_lines) {
        return block_lines.join("\n");
    }
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

        // Collect a run of adjacent non-blank lines. Extend across blank lines
        // when the current last line is an assertion ending with ` ...` and the
        // next non-blank line is also an assertion — elm-format treats this as
        // a single multi-line operator chain.
        let run_start = i;
        let mut run_end = i;
        loop {
            let next = run_end + 1;
            if next >= block_lines.len() {
                break;
            }
            if !block_lines[next].trim().is_empty() {
                run_end = next;
                continue;
            }
            let last_trimmed = block_lines[run_end].trim();
            if !last_trimmed.ends_with(" ...") {
                break;
            }
            let mut j = next;
            while j < block_lines.len() && block_lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= block_lines.len() {
                break;
            }
            let next_trimmed = block_lines[j].trim();
            if next_trimmed.starts_with("--") || !looks_like_assertion(next_trimmed) {
                break;
            }
            run_end = j;
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
            if l.trim().is_empty() {
                continue;
            }
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
        let is_assertion_run = all_valid && assertion_count >= 1 && last_is_assertion;

        // If the run's last non-blank line ends with ` ...`, the chain would
        // be incomplete. elm-format preserves such blocks without chain
        // reformatting.
        let run_last_ends_with_dots = {
            let mut idx = run_end;
            while idx > run_start && block_lines[idx].trim().is_empty() {
                idx -= 1;
            }
            block_lines[idx].trim().ends_with(" ...")
        };
        if is_assertion_run && run_last_ends_with_dots {
            for (k, idx) in (run_start..=run_end).enumerate() {
                if i > 0 || k > 0 {
                    out.push('\n');
                }
                out.push_str(block_lines[idx]);
            }
            i = run_end + 1;
            continue;
        }
        if is_assertion_run {
            // Group lines into chains. A chain contains an optional run of
            // comment lines, one assertion, plus any continuation assertions
            // triggered by a trailing ` ...` on the prior assertion. Chains
            // are separated by blank lines; within a chain, a single-line
            // assertion is emitted normally, while a multi-line chain is
            // joined and split at ` == ` / ` ... ` operators into the
            // elm-format multi-line form.
            let mut chains: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
            let mut cur_comments: Vec<usize> = Vec::new();
            let mut cur_assertions: Vec<usize> = Vec::new();
            for k in run_start..=run_end {
                let trimmed = block_lines[k].trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with("--") {
                    cur_comments.push(k);
                } else {
                    cur_assertions.push(k);
                    if !trimmed.ends_with(" ...") {
                        chains.push((
                            std::mem::take(&mut cur_comments),
                            std::mem::take(&mut cur_assertions),
                        ));
                    }
                }
            }
            if !cur_comments.is_empty() || !cur_assertions.is_empty() {
                chains.push((cur_comments, cur_assertions));
            }

            for (chain_idx, (comments, assertions)) in chains.iter().enumerate() {
                if chain_idx == 0 && i > 0 {
                    out.push('\n');
                } else if chain_idx > 0 {
                    out.push_str("\n\n");
                }
                for &ci in comments {
                    out.push_str(block_lines[ci]);
                    out.push('\n');
                }
                if assertions.len() == 1 {
                    let l = block_lines[assertions[0]];
                    let indent_str = &l[..first_indent];
                    let content = &l[first_indent..];
                    let normalized = collapse_spaces_outside_strings(content);
                    let normalized = space_tight_binary_ops(&normalized);
                    let normalized = space_tight_tuples_lists(&normalized);
                    out.push_str(indent_str);
                    out.push_str(&normalized);
                } else if !assertions.is_empty() {
                    let joined = assertions
                        .iter()
                        .map(|&idx| block_lines[idx].trim())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let joined = collapse_spaces_outside_strings(&joined);
                    let joined = space_tight_binary_ops(&joined);
                    let joined = space_tight_tuples_lists(&joined);
                    let segments = split_at_chain_operators(&joined);
                    let indent_str = &block_lines[assertions[0]][..first_indent];
                    let cont_indent: String = std::iter::repeat(' ')
                        .take(first_indent + 4)
                        .collect();
                    out.push_str(indent_str);
                    if let Some(first) = segments.first() {
                        out.push_str(first);
                    }
                    for seg in segments.iter().skip(1) {
                        out.push('\n');
                        out.push_str(&cont_indent);
                        out.push_str(seg);
                    }
                }
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

pub(super) fn is_assertion_only_paragraph(para: &[String]) -> bool {
    let non_empty: Vec<&String> = para.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.len() < 2 {
        return false;
    }
    let mut assertion_count = 0usize;
    for line in &non_empty {
        // Must start at column 0 (no leading whitespace beyond what was stripped).
        if line.starts_with(' ') || line.starts_with('\t') {
            return false;
        }
        let trimmed = line.trim();
        // Allow `--` line comments mixed in, as long as at least one
        // line is a real assertion. elm-format treats a `-- comment` line
        // as attached to the following assertion.
        if trimmed.starts_with("--") {
            continue;
        }
        if !looks_like_assertion(trimmed) {
            return false;
        }
        assertion_count += 1;
    }
    assertion_count >= 1
}

pub(super) fn looks_like_assertion(trimmed: &str) -> bool {
    // Three accepted shapes for "example lines" inside doc code blocks:
    //   1. `expr == value` (optionally with trailing ` -- comment`)
    //   2. `expr -- comment` (expression followed by a line comment)
    //   3. Simple standalone expression (starts with identifier or constructor,
    //      balanced delimiters, doesn't end with an operator).
    // Lines beginning with `--` are standalone comments, not assertions.
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
    // Shape 3: a simple standalone expression line.
    looks_like_simple_expr_line(trimmed)
}

pub(super) fn looks_like_simple_expr_line(trimmed: &str) -> bool {
    // Must begin with identifier (lower/upper) or opening delimiter.
    let first = match trimmed.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_alphabetic()
        || first.is_ascii_digit()
        || first == '_'
        || first == '('
        || first == '['
        || first == '\''
        || first == '"'
        || first == '-')
    {
        return false;
    }
    // `-` only allowed as a leading negation when followed by a digit or paren.
    if first == '-' {
        let second = trimmed.chars().nth(1);
        match second {
            Some(c) if c.is_ascii_digit() || c == '(' => {}
            _ => return false,
        }
    }
    // Reject keyword-led lines (they are parts of a larger expression).
    let first_word_end = trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
        .unwrap_or(trimmed.len());
    let first_word = &trimmed[..first_word_end];
    match first_word {
        "type" | "port" | "module" | "import" | "let" | "in" | "if" | "then"
        | "else" | "case" | "of" | "where" | "alias" | "exposing" | "as"
        | "effect" | "infix" => return false,
        _ => {}
    }
    // Must have balanced parens/brackets, counting string/char literals.
    let mut paren = 0i32;
    let mut bracket = 0i32;
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    for c in trimmed.chars() {
        if esc {
            esc = false;
            continue;
        }
        if in_str {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if in_char {
            if c == '\\' {
                esc = true;
            } else if c == '\'' {
                in_char = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '\'' => in_char = true,
            '(' => paren += 1,
            ')' => {
                paren -= 1;
                if paren < 0 {
                    return false;
                }
            }
            '[' => bracket += 1,
            ']' => {
                bracket -= 1;
                if bracket < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    if paren != 0 || bracket != 0 || in_str || in_char {
        return false;
    }
    // Must not end with an operator character (continuation to next line).
    let last_non_ws = trimmed.trim_end();
    if let Some(lc) = last_non_ws.chars().last() {
        if "+-*/|&<>=,:".contains(lc) {
            return false;
        }
    }
    true
}

/// Add spaces around tight binary operators (`1/2` → `1 / 2`, `2^3` → `2 ^ 3`)
/// outside of string and char literals. Does NOT modify text inside `-- comments`.
/// Conservative: only applies when the operator is flanked by identifier/digit
/// characters on both sides.
pub(super) fn space_tight_binary_ops(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut in_line_comment = false;
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_line_comment {
            out.push(c);
            i += 1;
            continue;
        }
        if esc {
            out.push(c);
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if in_char {
            if c == '\\' {
                esc = true;
            } else if c == '\'' {
                in_char = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '\'' {
            in_char = true;
            out.push(c);
            i += 1;
            continue;
        }
        // Detect start of a line comment (`--`).
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            in_line_comment = true;
            out.push_str("--");
            i += 2;
            continue;
        }
        // `//` integer division.
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/'
            && i > 0 && i + 2 < chars.len()
            && is_ident(chars[i - 1]) && is_ident(chars[i + 2])
        {
            out.push(' ');
            out.push_str("//");
            out.push(' ');
            i += 2;
            continue;
        }
        // Single `/` or `^`.
        if matches!(c, '/' | '^')
            && i > 0 && i + 1 < chars.len()
            && is_ident(chars[i - 1]) && is_ident(chars[i + 1])
        {
            // Guard against `//` which was handled above.
            if c == '/' && chars[i + 1] == '/' {
                out.push(c);
                i += 1;
                continue;
            }
            out.push(' ');
            out.push(c);
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub(super) fn split_at_chain_operators(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut positions: Vec<usize> = Vec::new();
    let mut in_str = false;
    let mut in_ch = false;
    let mut esc = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if in_ch {
            if c == '\\' {
                esc = true;
            } else if c == '\'' {
                in_ch = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            i += 1;
            continue;
        }
        if c == '\'' {
            in_ch = true;
            i += 1;
            continue;
        }
        if c == ' '
            && i + 3 < chars.len()
            && chars[i + 1] == '='
            && chars[i + 2] == '='
            && chars[i + 3] == ' '
        {
            positions.push(i);
            i += 4;
            continue;
        }
        if c == ' '
            && i + 4 < chars.len()
            && chars[i + 1] == '.'
            && chars[i + 2] == '.'
            && chars[i + 3] == '.'
            && chars[i + 4] == ' '
        {
            positions.push(i);
            i += 5;
            continue;
        }
        i += 1;
    }

    let mut segments = Vec::new();
    let mut last = 0;
    for &pos in &positions {
        let seg: String = chars[last..pos].iter().collect();
        segments.push(seg.trim().to_string());
        last = pos + 1;
    }
    let tail: String = chars[last..].iter().collect();
    segments.push(tail.trim().to_string());
    segments
}

pub(super) fn space_tight_tuples_lists(s: &str) -> String {
    struct Frame {
        out_pos: usize,
        kind: char,
        tight: bool,
        has_content: bool,
        has_comma: bool,
        has_non_comma_content: bool,
    }
    let input: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(input.len() + 8);
    let mut frames: Vec<Frame> = Vec::new();
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut in_line_comment = false;

    let mut i = 0;
    while i < input.len() {
        let c = input[i];

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if esc {
            out.push(c);
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if in_char {
            if c == '\\' {
                esc = true;
            } else if c == '\'' {
                in_char = false;
            }
            out.push(c);
            i += 1;
            continue;
        }

        if c == '-' && i + 1 < input.len() && input[i + 1] == '-' {
            in_line_comment = true;
            out.push('-');
            out.push('-');
            i += 2;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '\'' {
            in_char = true;
            out.push(c);
            i += 1;
            continue;
        }

        if c == '(' || c == '[' {
            let next = input.get(i + 1).copied();
            let tight = !matches!(next, Some(' ') | Some('\n') | Some('\t'));
            frames.push(Frame {
                out_pos: out.len(),
                kind: c,
                tight,
                has_content: false,
                has_comma: false,
                has_non_comma_content: false,
            });
            out.push(c);
            i += 1;
            continue;
        }

        if c == ')' || c == ']' {
            if let Some(frame) = frames.pop() {
                let expected = if c == ')' { '(' } else { '[' };
                if frame.kind == expected {
                    let should_expand = if c == ']' {
                        frame.tight && frame.has_content
                    } else {
                        frame.tight && frame.has_comma && frame.has_non_comma_content
                    };
                    let should_tighten = c == ')'
                        && !frame.tight
                        && !frame.has_comma
                        && frame.has_content;
                    if should_expand {
                        out.insert(frame.out_pos + 1, ' ');
                        out.push(' ');
                    } else if should_tighten {
                        if out.get(frame.out_pos + 1).copied() == Some(' ') {
                            out.remove(frame.out_pos + 1);
                        }
                        while out.last().copied() == Some(' ') {
                            out.pop();
                        }
                    }
                }
            }
            out.push(c);
            i += 1;
            continue;
        }

        if c == ',' {
            if let Some(top) = frames.last_mut() {
                top.has_content = true;
                top.has_comma = true;
            }
            out.push(c);
            let next = input.get(i + 1).copied();
            if let Some(n) = next {
                if n.is_ascii_alphanumeric()
                    || n == '_'
                    || n == '('
                    || n == '['
                    || n == '{'
                    || n == '\''
                    || n == '"'
                    || n == '-'
                {
                    out.push(' ');
                }
            }
            i += 1;
            continue;
        }

        if !c.is_whitespace() {
            if let Some(top) = frames.last_mut() {
                top.has_content = true;
                top.has_non_comma_content = true;
            }
        }
        out.push(c);
        i += 1;
    }

    out.iter().collect()
}

pub(super) fn collapse_spaces_outside_strings(s: &str) -> String {
    // Track delimiter "style" — whether the opener was followed by a space.
    // `(x  )` collapses to `(x)`, but `[ 1, 2 ]` preserves the inner space.
    #[derive(Clone, Copy)]
    enum Style { Tight, Spaced }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escape = false;
    let mut prev_space = false;
    let mut in_line_comment = false;
    let mut style_stack: Vec<Style> = Vec::new();
    for (idx, &c) in chars.iter().enumerate() {
        if in_line_comment {
            out.push(c);
            continue;
        }
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
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            prev_space = false;
            continue;
        }
        if c == '-' && chars.get(idx + 1).copied() == Some('-') {
            in_line_comment = true;
            out.push(c);
            prev_space = false;
            continue;
        }
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
            continue;
        }
        if matches!(c, '(' | '[' | '{') {
            // Peek at next char to classify opener style.
            let next = chars.get(idx + 1).copied();
            let style = match next {
                Some(' ') => Style::Spaced,
                _ => Style::Tight,
            };
            style_stack.push(style);
            out.push(c);
            prev_space = false;
            continue;
        }
        if matches!(c, ')' | ']' | '}') {
            let style = style_stack.pop().unwrap_or(Style::Spaced);
            if prev_space && matches!(style, Style::Tight) {
                out.pop();
            }
            out.push(c);
            prev_space = false;
            continue;
        }
        out.push(c);
        prev_space = false;
    }
    out
}

/// Check whether a code block needs reformatting.
///
/// Returns true if the block contains:
/// - lines with non-4-aligned indentation (2-space indent), OR
/// - compact list/tuple syntax that elm-format would space out
///   (e.g., `[1,2]` -> `[ 1, 2 ]`, `(0,"a")` -> `( 0, "a" )`)
pub(super) fn code_block_needs_reformat(block_lines: &[&str]) -> bool {
    let mut count_non_4_aligned = 0usize;
    let mut has_compact_syntax = false;
    let mut has_single_line_decl = false;
    let mut has_unsorted_import = false;
    for &line in block_lines {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.len() - line.trim_start().len();
        if leading > 4 && (leading - 4) % 4 != 0 {
            count_non_4_aligned += 1;
        }
        // Imports with an out-of-order `exposing` list get re-sorted by
        // elm-format. Flag the block for reformat so the import re-parses
        // through the module parser which normalizes exposing order.
        if leading == 4 && import_has_unsorted_exposing(line.trim()) {
            has_unsorted_import = true;
        }
        // Check for compact list syntax [x,y] or [x] or tuple syntax (x,y)
        // that elm-format would normalize to [ x, y ] / [ x ] / ( x, y ).
        let trimmed = line.trim();
        if trimmed.contains('[') && trimmed.contains(']') {
            // Look for `[` immediately followed by a literal or identifier
            // start — the distinguishing marker of a compact list/tuple.
            if trimmed.contains("[\"") || trimmed.contains("[(") || trimmed.contains("['")
                || trimmed.contains("[0") || trimmed.contains("[1")
                || trimmed.contains("[2") || trimmed.contains("[3")
                || trimmed.contains("[4") || trimmed.contains("[5")
                || trimmed.contains("[6") || trimmed.contains("[7")
                || trimmed.contains("[8") || trimmed.contains("[9")
            {
                has_compact_syntax = true;
            }
        }
        if trimmed.contains('(') && trimmed.contains(',') && trimmed.contains(')') {
            // Match `(X` where X is a literal/identifier start — the
            // distinguishing marker of a compact tuple. A space after `(`
            // means the tuple is already normalized; skip in that case.
            if has_compact_tuple(trimmed) {
                has_compact_syntax = true;
            }
        }
        // Single-line value declaration at the block base-indent (4 spaces):
        // `name = expr` fits on one line. elm-format always expands these
        // to two lines (`name =\n    expr`), so flag for reformat.
        if leading == 4 && is_single_line_value_decl(trimmed) {
            has_single_line_decl = true;
        }
        // Single-line type / type-alias declaration at base indent. elm-format
        // always expands these to multi-line form.
        if leading == 4
            && (trimmed.starts_with("type alias ") || trimmed.starts_with("type "))
            && trimmed.contains(" = ")
        {
            has_single_line_decl = true;
        }
        // Single-line doc-comment `{-| ... -}` on its own line inside a code
        // block. elm-format splits these into multi-line form.
        if leading == 4
            && trimmed.starts_with("{-|")
            && trimmed.ends_with("-}")
            && trimmed.len() > 5
        {
            has_single_line_decl = true;
        }
        // Tight operator (no space around) like `3^2`. elm-format always
        // inserts spaces around infix operators.
        if has_tight_binary_op(trimmed) {
            has_compact_syntax = true;
        }
        // A line that is a single parenthesized operator expression with no
        // commas, like `(true || false)`. elm-format strips the redundant
        // outer parens on reformat.
        if leading == 4 && is_redundant_paren_expr(trimmed) {
            has_compact_syntax = true;
        }
        // A hex literal whose width doesn't match elm-format's padding (2, 4,
        // 8, or 16 digits). Flag for reformat so the literal gets normalized.
        if line_has_unpadded_hex(trimmed) {
            has_compact_syntax = true;
        }
        // A float literal in scientific form with no decimal point (e.g.
        // `1e-42`). elm-format normalizes to `1.0e-42`.
        if line_has_sci_float_without_dot(trimmed) {
            has_compact_syntax = true;
        }
    }
    let has_indent_issues = count_non_4_aligned > 0;
    let has_unseparated_assertions = block_has_unseparated_assertions(block_lines);
    let has_single_line_if = block_has_single_line_if(block_lines);
    has_indent_issues
        || has_compact_syntax
        || has_single_line_decl
        || has_unsorted_import
        || has_unseparated_assertions
        || has_single_line_if
}

/// Detect a code block containing a line with a single-line `if ... then ... else ...`
/// expression. elm-format always breaks `if-then-else` across multiple lines, so
/// such blocks need reformat.
pub(super) fn block_has_single_line_if(block_lines: &[&str]) -> bool {
    for &line in block_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let leading = line.len() - line.trim_start().len();
        if leading < 4 {
            continue;
        }
        if line_has_single_line_if_then_else(trimmed) {
            return true;
        }
    }
    false
}

/// True when the trimmed line contains both ` then ` and ` else ` outside
/// string/char literals and comments — markers of an inline if-then-else that
/// elm-format breaks across multiple lines.
pub(super) fn line_has_single_line_if_then_else(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut in_char = false;
    let mut in_triple = false;
    let mut esc = false;
    let mut i = 0;
    let mut saw_then = false;
    let mut saw_else = false;
    while i < bytes.len() {
        let b = bytes[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if in_triple {
            if i + 2 < bytes.len() && &bytes[i..i + 3] == b"\"\"\"" {
                in_triple = false;
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            i += 1;
            continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            i += 1;
            continue;
        }
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            // line comment — stop scanning.
            break;
        }
        if i + 2 < bytes.len() && &bytes[i..i + 3] == b"\"\"\"" {
            in_triple = true;
            i += 3;
            continue;
        }
        if b == b'"' { in_str = true; i += 1; continue; }
        if b == b'\'' { in_char = true; i += 1; continue; }
        // Match " then " and " else " as whole keywords.
        if i + 6 <= bytes.len() && &bytes[i..i + 6] == b" then " {
            saw_then = true;
        }
        if i + 6 <= bytes.len() && &bytes[i..i + 6] == b" else " {
            saw_else = true;
        }
        i += 1;
    }
    saw_then && saw_else
}

/// Detect a code block containing multiple assertion-shaped lines with no
/// blank-line separation between them. elm-format renders each assertion as
/// its own paragraph separated by blank lines, so such blocks need reformat.
/// Only considers runs of 2+ consecutive assertion lines (possibly interleaved
/// with `--` comments that attach to the following assertion).
pub(super) fn block_has_unseparated_assertions(block_lines: &[&str]) -> bool {
    let mut run_assert_count = 0usize;
    for &line in block_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if run_assert_count >= 2 {
                return true;
            }
            run_assert_count = 0;
            continue;
        }
        // Only consider lines at the 4-space base indent (code-block content).
        let leading = line.len() - line.trim_start().len();
        if leading != 4 {
            if run_assert_count >= 2 {
                return true;
            }
            run_assert_count = 0;
            continue;
        }
        if trimmed.starts_with("--") {
            // `-- comment` attaches to the following assertion; skip without
            // resetting the run.
            continue;
        }
        if looks_like_assertion(trimmed) {
            run_assert_count += 1;
        } else {
            if run_assert_count >= 2 {
                return true;
            }
            run_assert_count = 0;
        }
    }
    run_assert_count >= 2
}

/// Detect a line that is a single parenthesized operator expression like
/// `(true || false)` or `(a + b)` — where the outer parens are redundant
/// at top level. Conservative: requires a `(` at the very start of the
/// trimmed line, a matching `)` at the end, no commas at the outer level
/// (so tuples are excluded), and at least one binary-operator character
/// at the outer level between the parens.
pub(super) fn is_redundant_paren_expr(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    if bytes.len() < 4 || bytes[0] != b'(' || *bytes.last().unwrap() != b')' {
        return false;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut saw_outer_op = false;
    for (i, &b) in bytes.iter().enumerate() {
        if esc { esc = false; continue; }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'\'' => in_char = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 && i != bytes.len() - 1 {
                    // parens closed before end — not a fully-wrapped expression
                    return false;
                }
            }
            b',' if depth == 1 => return false,
            b'|' | b'&' | b'+' | b'*' | b'/' | b'<' | b'>' | b'=' if depth == 1 => {
                saw_outer_op = true;
            }
            b'-' if depth == 1 && i > 1 => {
                let prev = bytes[i - 1];
                if prev == b' ' { saw_outer_op = true; }
            }
            _ => {}
        }
    }
    depth == 0 && saw_outer_op
}

/// Detect a hex literal whose digit count is not one of elm-format's
/// canonical widths (2, 4, 8, or 16). Scans for `0x[0-9A-Fa-f]+` tokens
/// outside strings and char literals.
pub(super) fn line_has_unpadded_hex(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            i += 1;
            continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            i += 1;
            continue;
        }
        if b == b'"' { in_str = true; i += 1; continue; }
        if b == b'\'' { in_char = true; i += 1; continue; }
        // Look for `0x` not preceded by an identifier character.
        if b == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            let prev_ok = if i == 0 {
                true
            } else {
                let p = bytes[i - 1];
                !(p.is_ascii_alphanumeric() || p == b'_')
            };
            if prev_ok {
                let start = i + 2;
                let mut j = start;
                while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                    j += 1;
                }
                let width = j - start;
                if width > 0 && width != 2 && width != 4 && width != 8 && width != 16 {
                    return true;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Detect a compact tuple like `(Float, Float)` or `(1,2)` where `(` is
/// immediately followed by a literal / identifier character (no space) and
/// at least one comma at outer depth closes into a `)`. elm-format
/// normalizes to `( Float, Float )`.
pub(super) fn has_compact_tuple(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            i += 1;
            continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            i += 1;
            continue;
        }
        if b == b'"' { in_str = true; i += 1; continue; }
        if b == b'\'' { in_char = true; i += 1; continue; }
        if b == b'(' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            // A space inside `( ` means already normalized; not compact.
            if next == b' ' || next == b')' {
                i += 1;
                continue;
            }
            // Scan for a matching `)` at the same depth, tracking commas.
            let mut depth = 1i32;
            let mut j = i + 1;
            let mut inner_in_str = false;
            let mut inner_in_char = false;
            let mut inner_esc = false;
            let mut found_comma = false;
            while j < bytes.len() && depth > 0 {
                let c = bytes[j];
                if inner_esc {
                    inner_esc = false;
                    j += 1;
                    continue;
                }
                if inner_in_str {
                    if c == b'\\' { inner_esc = true; }
                    else if c == b'"' { inner_in_str = false; }
                    j += 1;
                    continue;
                }
                if inner_in_char {
                    if c == b'\\' { inner_esc = true; }
                    else if c == b'\'' { inner_in_char = false; }
                    j += 1;
                    continue;
                }
                match c {
                    b'"' => inner_in_str = true,
                    b'\'' => inner_in_char = true,
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth -= 1,
                    b',' if depth == 1 => found_comma = true,
                    _ => {}
                }
                j += 1;
            }
            if found_comma && j > 0 {
                // Check that closing `)` isn't preceded by a space (`... )`).
                // If there's a space before `)` the tuple is already normalized.
                if bytes[j - 1] == b')' {
                    let before_close = if j >= 2 { bytes[j - 2] } else { b' ' };
                    if before_close != b' ' {
                        return true;
                    }
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    false
}

/// Detect a float literal in scientific form without a decimal point, e.g.
/// `1e-42` or `6e23`. elm-format normalizes these to `1.0e-42` / `6.0e23`.
pub(super) fn line_has_sci_float_without_dot(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut in_char = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' { esc = true; }
            else if b == b'"' { in_str = false; }
            i += 1;
            continue;
        }
        if in_char {
            if b == b'\\' { esc = true; }
            else if b == b'\'' { in_char = false; }
            i += 1;
            continue;
        }
        if b == b'"' { in_str = true; i += 1; continue; }
        if b == b'\'' { in_char = true; i += 1; continue; }
        // Look for a digit that starts a numeric literal.
        if b.is_ascii_digit() {
            let prev_ok = if i == 0 {
                true
            } else {
                let p = bytes[i - 1];
                !(p.is_ascii_alphanumeric() || p == b'_' || p == b'.')
            };
            if prev_ok {
                let start = i;
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let has_dot = j < bytes.len() && bytes[j] == b'.';
                if has_dot {
                    // Skip `.digits...`
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                let has_exp = j < bytes.len() && (bytes[j] == b'e' || bytes[j] == b'E');
                if has_exp && !has_dot {
                    // Check that a digit follows (possibly after +/-).
                    let mut k = j + 1;
                    if k < bytes.len() && (bytes[k] == b'+' || bytes[k] == b'-') {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k].is_ascii_digit() {
                        // Don't flag hex literals like `0x1e` (handled elsewhere).
                        // Here our start was at digit; if digits were `0` then
                        // `x` then hex — but we already separated hex via `0x` prefix.
                        let _ = start;
                        return true;
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Detect tight infix operators like `3^2` or `a^b` with no spaces.
/// Conservative: checks for `^` operator specifically, only when flanked
/// by identifier/digit characters on both sides (and not inside a string).
pub(super) fn has_tight_binary_op(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if b == b'^' && i > 0 && i + 1 < bytes.len() {
            let prev = bytes[i - 1];
            let next = bytes[i + 1];
            let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
            if is_ident(prev) && is_ident(next) {
                return true;
            }
        }
        if b == b'/' && i > 0 && i + 1 < bytes.len() {
            let prev = bytes[i - 1];
            let next = bytes[i + 1];
            // Skip over `//` (integer division) and line comments.
            // Handle both single `/` and `//` as tight operators.
            let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
            if next == b'/' {
                // `//` integer division: look at char before and char after `//`.
                if i + 2 < bytes.len() {
                    let after = bytes[i + 2];
                    if is_ident(prev) && is_ident(after) {
                        return true;
                    }
                }
                i += 2;
                continue;
            }
            if is_ident(prev) && is_ident(next) {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Returns true if `line` is an `import ... exposing (a, b, c)` line whose
/// exposing list items are not alphabetically sorted.
pub(super) fn import_has_unsorted_exposing(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with("import ") {
        return false;
    }
    let exp_idx = match t.find(" exposing (") {
        Some(i) => i,
        None => return false,
    };
    let rest = &t[exp_idx + " exposing (".len()..];
    let close_idx = match rest.rfind(')') {
        Some(i) => i,
        None => return false,
    };
    let inner = &rest[..close_idx];
    // Ignore wildcard exposing; don't try to handle nested parentheses
    // (e.g. `Type(..)`) — item key is the head before any `(`.
    if inner.trim() == ".." {
        return false;
    }
    let items: Vec<String> = inner
        .split(',')
        .map(|s| {
            let s = s.trim();
            let head = s.split('(').next().unwrap_or(s).trim();
            head.to_string()
        })
        .filter(|s| !s.is_empty())
        .collect();
    if items.len() < 2 {
        return false;
    }
    let mut sorted = items.clone();
    sorted.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    items != sorted
}

/// Detect `name = expr` on a single line, where expr is non-empty and the
/// `=` is not part of `==`, `/=`, `<=`, `>=`. This is the shape elm-format
/// always expands into two lines inside doc-comment code blocks.
pub(super) fn is_single_line_value_decl(trimmed: &str) -> bool {
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
pub(super) fn try_reformat_code_block(block_lines: &[&str]) -> Option<String> {
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

    // If the block already begins with a `module` declaration, use it
    // directly as the wrapper (don't double-wrap).
    let trimmed_raw = raw_code.trim_start();
    if trimmed_raw.starts_with("module ") || trimmed_raw.starts_with("port module ")
        || trimmed_raw.starts_with("effect module ")
    {
        if let Some(result) = try_parse_and_format_full_module(&raw_code) {
            // Re-indent every non-blank line with the 4-space doc-code prefix.
            // Inside a markdown code block, elm-format's Cheapskate renderer
            // collapses runs of blank lines to a single blank line, so skip
            // consecutive blank lines as we reindent.
            let mut out_lines: Vec<String> = Vec::new();
            let mut prev_blank = false;
            for l in result.split('\n') {
                if l.is_empty() {
                    if prev_blank {
                        continue;
                    }
                    prev_blank = true;
                    out_lines.push(String::new());
                } else {
                    prev_blank = false;
                    out_lines.push(format!("    {}", l));
                }
            }
            return Some(out_lines.join("\n"));
        }
    }

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

        // If the paragraph consists entirely of assertion-shaped lines, parse
        // each line as its own expression and render them as separate top-level
        // expressions. This must run BEFORE whole-paragraph expression parsing
        // because consecutive assertion lines like `1 == 1\n0 == 0` would
        // otherwise parse as function application (`1 == 1` applied to
        // `0 == 0`) and render as a single expression.
        //
        // A line starting with `-<digit/paren>` is a binary-subtraction
        // continuation of the previous expression, so it is *appended* to the
        // current accumulator rather than starting a new standalone expression.
        // This matches elm-format's behavior: `14 / 4 == 3.5\n-1 / 4 == -0.25`
        // parses as one expression `14 / 4 == 3.5 - 1 / 4 == -0.25`.
        let try_per_line = is_assertion_only_paragraph(para) && {
            let mut per_line_results: Vec<String> = Vec::new();
            let mut pending_comments: Vec<String> = Vec::new();
            let mut current_accum: Option<String> = None;
            let mut all_ok = true;
            let flush_accum = |accum: Option<String>,
                               results: &mut Vec<String>,
                               pending: &mut Vec<String>|
             -> bool {
                let Some(text) = accum else { return true; };
                let wrapped = format!(
                    "module DocTemp__ exposing (..)\n\n\ndocTemp__ =\n{}\n",
                    text
                );
                match try_parse_and_format_expr(&wrapped) {
                    Some(r) => {
                        let combined = if pending.is_empty() {
                            r
                        } else {
                            let mut s = pending.join("\n");
                            s.push('\n');
                            s.push_str(&r);
                            pending.clear();
                            s
                        };
                        results.push(combined);
                        true
                    }
                    None => false,
                }
            };
            for line in para {
                if line.trim().is_empty() {
                    continue;
                }
                let trimmed = line.trim();
                if trimmed.starts_with("--") {
                    pending_comments.push(trimmed.to_string());
                    continue;
                }
                let is_minus_cont = trimmed
                    .strip_prefix('-')
                    .is_some_and(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '('));
                if is_minus_cont && current_accum.is_some() {
                    let cur = current_accum.as_mut().unwrap();
                    cur.push('\n');
                    cur.push_str("    ");
                    cur.push_str(trimmed);
                } else {
                    if !flush_accum(current_accum.take(), &mut per_line_results, &mut pending_comments) {
                        all_ok = false;
                        break;
                    }
                    current_accum = Some(format!("    {}", trimmed));
                }
            }
            if all_ok {
                if !flush_accum(current_accum, &mut per_line_results, &mut pending_comments) {
                    all_ok = false;
                }
            }
            if !pending_comments.is_empty() {
                per_line_results.push(pending_comments.join("\n"));
            }
            if all_ok && !per_line_results.is_empty() {
                formatted_paragraphs.push(per_line_results.join("\n\n"));
                true
            } else {
                false
            }
        };
        if try_per_line {
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
        // separate "top-level" expressions. A leading `-- comment` line
        // attaches to the following assertion (no blank between them).
        if is_assertion_only_paragraph(para) {
            let mut per_line_results: Vec<String> = Vec::new();
            let mut pending_comments: Vec<String> = Vec::new();
            let mut all_ok = true;
            for line in para {
                if line.trim().is_empty() {
                    continue;
                }
                let trimmed = line.trim();
                if trimmed.starts_with("--") {
                    // Queue the comment to attach to the next assertion.
                    pending_comments.push(trimmed.to_string());
                    continue;
                }
                let wrapped_line = format!(
                    "module DocTemp__ exposing (..)\n\n\ndocTemp__ =\n    {}\n",
                    line
                );
                match try_parse_and_format_expr(&wrapped_line) {
                    Some(r) => {
                        let combined = if pending_comments.is_empty() {
                            r
                        } else {
                            let mut s = pending_comments.join("\n");
                            s.push('\n');
                            s.push_str(&r);
                            pending_comments.clear();
                            s
                        };
                        per_line_results.push(combined);
                    }
                    None => {
                        all_ok = false;
                        break;
                    }
                }
            }
            // Any trailing orphan comments attach as their own block.
            if !pending_comments.is_empty() {
                per_line_results.push(pending_comments.join("\n"));
            }
            if all_ok && !per_line_results.is_empty() {
                formatted_paragraphs.push(per_line_results.join("\n\n"));
                continue;
            }
        }

        // If the paragraph contains a triple-quoted string, it's probably
        // hard to parse as a single decl/expr but is still valid Elm in situ.
        // Keep it verbatim and continue, so other paragraphs can still be
        // reformatted.
        if para.iter().any(|l| l.contains("\"\"\"")) {
            formatted_paragraphs.push(para_text);
            continue;
        }

        // Can't parse this paragraph — bail out entirely.
        return None;
    }

    // Join paragraphs with blank lines. When a paragraph begins with a line
    // comment (`--`) and the previous paragraph consists of imports, insert
    // an extra blank line between them — elm-format renders this as a loose
    // separation.
    let mut joined = String::new();
    for (idx, para_text) in formatted_paragraphs.iter().enumerate() {
        if idx > 0 {
            let prev_para = &paragraphs[idx - 1];
            let cur_para = &paragraphs[idx];
            let sep = if paragraph_is_all_imports(prev_para)
                && paragraph_starts_with_line_comment(cur_para)
            {
                "\n\n\n"
            } else {
                "\n\n"
            };
            joined.push_str(sep);
        }
        joined.push_str(para_text);
    }
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

pub(super) fn paragraph_is_all_imports(para: &[String]) -> bool {
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

pub(super) fn paragraph_starts_with_line_comment(para: &[String]) -> bool {
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
pub(super) fn split_into_paragraphs(lines: &[String]) -> Vec<Vec<String>> {
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

/// Try to parse source as a module and extract formatted declarations.
/// Returns Some(re-indented string) or None.
pub(super) fn try_parse_and_format_module(wrapped: &str) -> Option<String> {
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

    // In doc-code blocks, elm-format attaches a leading line comment to the
    // following declaration with no intervening blank line. Collapse
    // `-- comment\n\n<decl>` to `-- comment\n<decl>`. Also collapse
    // `{-| doc -}\n\n<decl>` to `{-| doc -}\n<decl>`.
    // Exception: when the following line is an `import`, elm-format inserts
    // an extra blank line rather than attaching.
    let mut attached: Vec<&str> = Vec::with_capacity(collapsed.len());
    let mut i = 0;
    while i < collapsed.len() {
        attached.push(collapsed[i]);
        let trim = collapsed[i].trim_start();
        let ends_doc_comment = trim.trim_end() == "-}";
        let next_is_import = i + 2 < collapsed.len()
            && collapsed[i + 2].trim_start().starts_with("import ");
        if (trim.starts_with("--") || ends_doc_comment)
            && i + 2 < collapsed.len()
            && collapsed[i + 1].is_empty()
            && !collapsed[i + 2].is_empty()
            && !collapsed[i + 2].trim_start().starts_with("--")
            && !next_is_import
        {
            i += 2;
        } else if trim.starts_with("--")
            && next_is_import
            && i + 2 < collapsed.len()
            && collapsed[i + 1].is_empty()
        {
            // Insert an extra blank line so that `-- comment\n\nimport` becomes
            // `-- comment\n\n\nimport` (two blank lines) in the doc code block.
            attached.push("");
            i += 1;
        } else {
            i += 1;
        }
    }
    let collapsed = attached;

    // elm-format inserts an extra blank line between an import-only block
    // and a following line-comment paragraph inside a doc code block.
    let mut loosened: Vec<String> = Vec::with_capacity(collapsed.len() + 4);
    let mut j = 0;
    while j < collapsed.len() {
        loosened.push(collapsed[j].to_string());
        if collapsed[j].trim_start().starts_with("import ")
            && j + 2 < collapsed.len()
            && collapsed[j + 1].is_empty()
            && !collapsed[j + 2].is_empty()
            && collapsed[j + 2].trim_start().starts_with("--")
        {
            let mut k = j + 1;
            while k < collapsed.len()
                && !collapsed[k].trim_start().starts_with("import ")
                && !collapsed[k].is_empty()
            {
                k += 1;
            }
            let _ = k;
            loosened.push(String::new());
        }
        j += 1;
    }
    let collapsed_strings = loosened;
    let collapsed: Vec<&str> = collapsed_strings.iter().map(|s| s.as_str()).collect();

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
/// Parse `raw` (which already starts with a `module ... exposing (...)`
/// header) as a full Elm module, pretty-print, and return the full output
/// (including the header) on idempotency success.
pub(super) fn try_parse_and_format_full_module(raw: &str) -> Option<String> {
    let owned = raw.to_string();
    let result = std::panic::catch_unwind(|| {
        let module = crate::parse::parse(&owned).ok()?;
        let first = pretty_print(&module);
        let module2 = crate::parse::parse(&first).ok()?;
        let second = pretty_print(&module2);
        if first == second { Some(first) } else { None }
    });
    let formatted = match result {
        Ok(Some(f)) => f,
        _ => return None,
    };
    Some(formatted.trim_end_matches('\n').to_string())
}

pub(super) fn try_parse_and_format_module_raw(wrapped: &str) -> Option<String> {
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
pub(super) fn try_parse_and_format_expr(wrapped: &str) -> Option<String> {
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
pub(super) fn parse_docs_groups(doc: &str) -> Vec<Vec<String>> {
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
