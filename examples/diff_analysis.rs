use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn main() {
    let elm_format = env::var("ELM_FORMAT").expect("Set ELM_FORMAT env var");

    let dirs = vec![
        "test-fixtures/core/src",
        "test-fixtures/html/src",
        "test-fixtures/browser/src",
        "test-fixtures/json/src",
        "test-fixtures/http/src",
        "test-fixtures/url/src",
        "test-fixtures/parser/src",
        "test-fixtures/virtual-dom/src",
        "test-fixtures/bytes/src",
        "test-fixtures/file/src",
        "test-fixtures/time/src",
        "test-fixtures/regex/src",
        "test-fixtures/random/src",
        "test-fixtures/svg/src",
        "test-fixtures/compiler/reactor/src",
        "test-fixtures/project-metadata-utils/src",
        "test-fixtures/test/src",
        "test-fixtures/markdown/src",
        "test-fixtures/linear-algebra/src",
        "test-fixtures/webgl/src",
        "test-fixtures/benchmark/src",
        "test-fixtures/list-extra/src",
        "test-fixtures/maybe-extra/src",
        "test-fixtures/string-extra/src",
        "test-fixtures/dict-extra/src",
        "test-fixtures/array-extra/src",
        "test-fixtures/result-extra/src",
        "test-fixtures/html-extra/src",
        "test-fixtures/json-extra/src",
        "test-fixtures/typed-svg/src",
        "test-fixtures/elm-json-decode-pipeline/src",
        "test-fixtures/elm-sweet-poll/src",
        "test-fixtures/elm-compare/src",
        "test-fixtures/elm-string-conversions/src",
        "test-fixtures/elm-sortable-table/src",
        "test-fixtures/elm-css/src",
        "test-fixtures/elm-hex/src",
        "test-fixtures/elm-iso8601-date-strings/src",
        "test-fixtures/elm-ui/src",
        "test-fixtures/elm-animator/src",
        "test-fixtures/elm-markdown/src",
        "test-fixtures/remotedata/src",
        "test-fixtures/murmur3/src",
        "test-fixtures/elm-round/src",
        "test-fixtures/elm-base64/src",
        "test-fixtures/elm-flate/src",
        "test-fixtures/elm-csv/src",
        "test-fixtures/elm-rosetree/src",
        "test-fixtures/assoc-list/src",
        "test-fixtures/elm-bool-extra/src",
    ];

    let mut results: Vec<(usize, String, String)> = Vec::new();

    for dir in dirs {
        let files = find_elm_files(dir);
        for path in files {
            let src = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let ast = match elm_ast::parse::parse(&src) {
                Ok(a) => a,
                Err(_) => continue,
            };

            let pretty = elm_ast::print::pretty_print(&ast);

            let mut child = match Command::new(&elm_format)
                .args(["--stdin", "--elm-version=0.19"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(pretty.as_bytes())
                .unwrap();
            let output = match child.wait_with_output() {
                Ok(o) => o,
                Err(_) => continue,
            };

            if !output.status.success() {
                continue;
            }

            let formatted = String::from_utf8_lossy(&output.stdout);
            if pretty == formatted.as_ref() {
                continue;
            }

            let p_lines: Vec<&str> = pretty.lines().collect();
            let f_lines: Vec<&str> = formatted.lines().collect();

            let lcs_len = lcs_length(&p_lines, &f_lines);
            let diff_count = (p_lines.len() - lcs_len) + (f_lines.len() - lcs_len);

            let mut first_diff = String::new();
            for (i, (p, f)) in p_lines.iter().zip(f_lines.iter()).enumerate() {
                if p != f {
                    let pt: &str = p.trim();
                    let ft: &str = f.trim();
                    first_diff = format!("L{}: pp=[{}] ef=[{}]", i + 1, pt, ft);
                    break;
                }
            }
            if first_diff.is_empty() {
                first_diff = format!("line count: pp={} ef={}", p_lines.len(), f_lines.len());
            }

            results.push((diff_count, path, first_diff));
        }
    }

    results.sort_by_key(|(count, _, _)| *count);

    for (count, path, first_diff) in &results {
        println!("{}\t{}\t{}", count, path, first_diff);
    }
}

fn lcs_length(a: &[&str], b: &[&str]) -> usize {
    let n = a.len();
    let m = b.len();
    let mut prev = vec![0usize; m + 1];
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                curr[j] = prev[j - 1] + 1;
            } else {
                curr[j] = prev[j].max(curr[j - 1]);
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        for x in curr.iter_mut() {
            *x = 0;
        }
    }
    prev[m]
}

fn find_elm_files(dir: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect_elm_files(&std::path::PathBuf::from(dir), &mut files);
    files.sort();
    files
}

fn collect_elm_files(dir: &std::path::PathBuf, files: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_elm_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "elm") {
                files.push(path.to_string_lossy().into_owned());
            }
        }
    }
}
