//! Code-block reformat and assertion-paragraph transforms.
//!
//! elm-format re-runs Elm code inside doc comments through its own parser
//! and printer. These helpers decide whether a block needs to be reformatted,
//! rewrite chains of `expr == value` assertions into the elm-format vertical
//! layout, and drive the full reparse+reindent pipeline for the block.

use super::*;

pub(in crate::print) fn transform_assertion_paragraphs(block_lines: &[&str]) -> String {
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
                    if !last_trimmed.is_empty()
                        && !last_trimmed.starts_with("--")
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
        #[allow(clippy::needless_range_loop)]
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
            #[allow(clippy::needless_range_loop)]
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
                    let cont_indent: String = std::iter::repeat_n(' ', first_indent + 4).collect();
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

/// Check whether a code block needs reformatting.
///
/// Returns true if the block contains:
/// - lines with non-4-aligned indentation (2-space indent), OR
/// - compact list/tuple syntax that elm-format would space out
///   (e.g., `[1,2]` -> `[ 1, 2 ]`, `(0,"a")` -> `( 0, "a" )`)
pub(in crate::print) fn code_block_needs_reformat(block_lines: &[&str]) -> bool {
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
            if trimmed.contains("[\"")
                || trimmed.contains("[(")
                || trimmed.contains("['")
                || trimmed.contains("[0")
                || trimmed.contains("[1")
                || trimmed.contains("[2")
                || trimmed.contains("[3")
                || trimmed.contains("[4")
                || trimmed.contains("[5")
                || trimmed.contains("[6")
                || trimmed.contains("[7")
                || trimmed.contains("[8")
                || trimmed.contains("[9")
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
pub(in crate::print) fn try_reformat_code_block(block_lines: &[&str]) -> Option<String> {
    // Strip the 4-space prefix from each line to get raw Elm code.
    let mut raw_lines: Vec<String> = Vec::new();
    for &line in block_lines {
        if line.trim().is_empty() {
            raw_lines.push(String::new());
        } else if let Some(stripped) = line.strip_prefix("    ") {
            raw_lines.push(stripped.to_string());
        } else {
            return None;
        }
    }

    let raw_code = raw_lines.join("\n");

    // If the block already begins with a `module` declaration, use it
    // directly as the wrapper (don't double-wrap).
    let trimmed_raw = raw_code.trim_start();
    if (trimmed_raw.starts_with("module ")
        || trimmed_raw.starts_with("port module ")
        || trimmed_raw.starts_with("effect module "))
        && let Some(result) = try_parse_and_format_full_module(&raw_code)
    {
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

    // First try: parse as a full module with declarations.
    let wrapped = format!("module DocTemp__ exposing (..)\n\n\n{}\n", raw_code);
    if let Some(result) = try_parse_and_format_module(&wrapped) {
        return Some(result);
    }

    // Second try: split into paragraphs (separated by blank lines) and
    // try each paragraph individually. Some may be expressions, some
    // declarations.
    let paragraphs = split_into_paragraphs(&raw_lines);

    // If the block has an `import` paragraph and the module-parse path
    // above already failed, elm-format leaves the whole block verbatim
    // (it won't reformat expression paragraphs that co-exist with
    // imports inside a single code block). Mirror that here.
    if paragraphs.iter().any(|p| paragraph_is_all_imports(p)) {
        let mut out_lines: Vec<String> = Vec::new();
        let mut prev_blank = false;
        for l in raw_lines.iter() {
            if l.trim().is_empty() {
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

    let mut formatted_paragraphs: Vec<String> = Vec::new();

    for para in &paragraphs {
        let para_text = para.join("\n");

        // Try as declaration(s) first.
        let wrapped_decl = format!("module DocTemp__ exposing (..)\n\n\n{}\n", para_text);
        if let Some(result) = try_parse_and_format_module_raw(&wrapped_decl) {
            formatted_paragraphs.push(result);
            continue;
        }

        // A single bare expression followed by `-- comment` lines is left
        // verbatim by elm-format when it can't parse the paragraph as
        // declarations. Mirror that to avoid adding unrelated spacing inside
        // example code.
        if paragraph_is_single_expr_with_line_comment(para) {
            formatted_paragraphs.push(para_text);
            continue;
        }

        // Triple-quoted strings are rarely re-printable safely (the parser
        // loses attached line comments and interior formatting). Preserve
        // the paragraph verbatim if it contains one.
        if para.iter().any(|l| l.contains("\"\"\"")) {
            formatted_paragraphs.push(para_text);
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
                let Some(text) = accum else {
                    return true;
                };
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
                let is_minus_cont = trimmed.strip_prefix('-').is_some_and(|r| {
                    r.chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit() || c == '(')
                });
                if let Some(cur) = current_accum.as_mut().filter(|_| is_minus_cont) {
                    cur.push('\n');
                    cur.push_str("    ");
                    cur.push_str(trimmed);
                } else {
                    if !flush_accum(
                        current_accum.take(),
                        &mut per_line_results,
                        &mut pending_comments,
                    ) {
                        all_ok = false;
                        break;
                    }
                    current_accum = Some(format!("    {}", trimmed));
                }
            }
            if all_ok && !flush_accum(current_accum, &mut per_line_results, &mut pending_comments) {
                all_ok = false;
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
        let indented: Vec<String> = para
            .iter()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("    {}", line)
                }
            })
            .collect();
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
