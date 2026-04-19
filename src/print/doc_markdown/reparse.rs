//! Re-parse/re-format helpers for doc-comment code blocks.
//!
//! elm-format runs Elm code found inside doc comments through the full
//! parse→print pipeline and uses the result only when the round-trip is
//! idempotent (pretty-printing twice yields the same output). These helpers
//! wrap a snippet in a dummy module header so the parser accepts it, run the
//! check, and return the re-indented body.

use super::pretty_print;

pub(in crate::print) fn try_parse_and_format_module(wrapped: &str) -> Option<String> {
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
        let next_is_import =
            i + 2 < collapsed.len() && collapsed[i + 2].trim_start().starts_with("import ");
        if (trim.starts_with("--") || ends_doc_comment)
            && i + 2 < collapsed.len()
            && collapsed[i + 1].is_empty()
            && !collapsed[i + 2].is_empty()
            && !collapsed[i + 2].trim_start().starts_with("--")
            && !next_is_import
        {
            i += 2;
        } else if crate::print::converged_mode::is_on()
            && trim.starts_with("--")
            && next_is_import
            && i + 2 < collapsed.len()
            && collapsed[i + 1].is_empty()
        {
            // In `ElmFormatConverged` mode, pre-apply elm-format's
            // second-pass mutation: `-- comment\n<blank>\nimport` becomes
            // `-- comment\n<blank>\n<blank>\nimport` (2 blank lines).
            // elm-format is not idempotent on this pattern; emitting the
            // converged form here makes our output survive an elm-format
            // round-trip unchanged (`pp == elm-format(pp)`).
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

/// Parse `raw` (which already starts with a `module ... exposing (...)`
/// header) as a full Elm module, pretty-print, and return the full output
/// (including the header) on idempotency success.
pub(in crate::print) fn try_parse_and_format_full_module(raw: &str) -> Option<String> {
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

/// Parse wrapped source as a module and extract raw declaration text
/// (no re-indenting).
pub(in crate::print) fn try_parse_and_format_module_raw(wrapped: &str) -> Option<String> {
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
pub(in crate::print) fn try_parse_and_format_expr(wrapped: &str) -> Option<String> {
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
        } else if let Some(stripped) = line.strip_prefix("    ") {
            result_lines.push(stripped.to_string());
        } else {
            // Unexpected indentation — bail out.
            return None;
        }
    }

    Some(result_lines.join("\n"))
}
