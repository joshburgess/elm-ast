//! Space-tight / tight-operator normalization for doc code blocks.
//!
//! elm-format expands compact tuple/list syntax (`[1,2]` → `[ 1, 2 ]`),
//! pads tight binary operators (`1/2` → `1 / 2`), and rewrites chain
//! operators in a few specific ways. These helpers perform those
//! transformations on a single line or expression string.

pub(in crate::print) fn space_tight_binary_ops(s: &str) -> String {
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
        if c == '/'
            && i + 1 < chars.len()
            && chars[i + 1] == '/'
            && i > 0
            && i + 2 < chars.len()
            && is_ident(chars[i - 1])
            && is_ident(chars[i + 2])
        {
            out.push(' ');
            out.push_str("//");
            out.push(' ');
            i += 2;
            continue;
        }
        // Single `/` or `^`.
        if matches!(c, '/' | '^')
            && i > 0
            && i + 1 < chars.len()
            && is_ident(chars[i - 1])
            && is_ident(chars[i + 1])
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

pub(in crate::print) fn split_at_chain_operators(s: &str) -> Vec<String> {
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

pub(in crate::print) fn space_tight_tuples_lists(s: &str) -> String {
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
                    let should_tighten =
                        c == ')' && !frame.tight && !frame.has_comma && frame.has_content;
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

pub(in crate::print) fn collapse_spaces_outside_strings(s: &str) -> String {
    // Track delimiter "style" — whether the opener was followed by a space.
    // `(x  )` collapses to `(x)`, but `[ 1, 2 ]` preserves the inner space.
    #[derive(Clone, Copy)]
    enum Style {
        Tight,
        Spaced,
    }
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
