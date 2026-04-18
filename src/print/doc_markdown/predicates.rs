//! Pure predicates over Elm code lines and doc-comment blocks.
//!
//! Small read-only inspectors used by the reformat/assertion logic to
//! decide whether a line looks like a declaration, an assertion, a
//! redundant-paren expression, etc.

pub(in crate::print) fn looks_like_code_block_decl(line: &str) -> bool {
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
pub(in crate::print) fn looks_like_value_decl_start(line: &str) -> bool {
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

pub(in crate::print) fn looks_like_type_annotation(line: &str) -> bool {
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

pub(in crate::print) fn is_assertion_only_paragraph(para: &[String]) -> bool {
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

pub(in crate::print) fn looks_like_assertion(trimmed: &str) -> bool {
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

pub(in crate::print) fn looks_like_simple_expr_line(trimmed: &str) -> bool {
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
pub(in crate::print) fn block_has_single_line_if(block_lines: &[&str]) -> bool {
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
pub(in crate::print) fn line_has_single_line_if_then_else(line: &str) -> bool {
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
pub(in crate::print) fn block_has_unseparated_assertions(block_lines: &[&str]) -> bool {
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
pub(in crate::print) fn is_redundant_paren_expr(trimmed: &str) -> bool {
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
pub(in crate::print) fn line_has_unpadded_hex(line: &str) -> bool {
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
pub(in crate::print) fn has_compact_tuple(line: &str) -> bool {
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
pub(in crate::print) fn line_has_sci_float_without_dot(line: &str) -> bool {
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
pub(in crate::print) fn has_tight_binary_op(line: &str) -> bool {
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
pub(in crate::print) fn import_has_unsorted_exposing(line: &str) -> bool {
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
pub(in crate::print) fn is_single_line_value_decl(trimmed: &str) -> bool {
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
