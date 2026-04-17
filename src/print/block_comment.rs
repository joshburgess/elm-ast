//! Block comment continuation re-indentation.
//!
//! elm-format re-aligns the continuation lines of a multi-line block comment
//! (`{- ... -}`) so that their minimum indent sits at `{- column + 3` — the
//! column where content after `{- ` begins. The closing `-}` is normalized to
//! sit at the `{- column`.

/// Reindent a multiline block comment's content. `brace_col` is the column
/// where `{-` is being emitted in the new output.
pub(super) fn reindent_block_comment(text: &str, brace_col: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() <= 1 {
        return text.to_string();
    }

    // Compute the current minimum indent among non-blank content lines,
    // EXCLUDING the final `-}` line (which is special).
    let last_idx = lines.len() - 1;
    let mut min_indent: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if i == 0 || i == last_idx {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let ind = line.chars().take_while(|c| *c == ' ').count();
        min_indent = Some(match min_indent {
            None => ind,
            Some(m) => m.min(ind),
        });
    }

    let target_min = brace_col + 3;
    let delta: isize = match min_indent {
        Some(m) => target_min as isize - m as isize,
        None => 0,
    };

    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            out.push_str(line);
            continue;
        }
        out.push('\n');
        let is_last = i == last_idx;
        if is_last {
            let stripped = line.trim_start_matches(' ');
            for _ in 0..brace_col {
                out.push(' ');
            }
            out.push_str(stripped);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let ind = line.chars().take_while(|c| *c == ' ').count();
        let rest = &line[ind..];
        let new_ind = ((ind as isize) + delta).max(0) as usize;
        for _ in 0..new_ind {
            out.push(' ');
        }
        out.push_str(rest);
    }
    out
}
