#![allow(clippy::collapsible_if)]

use elm_lint::rule;
use elm_lint::rules;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rule::LintContext;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut dir = "src";
    let mut list_rules = false;
    let mut enabled_rules: Option<Vec<String>> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--list" => list_rules = true,
            "--rules" => {
                i += 1;
                if i < args.len() {
                    enabled_rules =
                        Some(args[i].split(',').map(|s| s.trim().to_string()).collect());
                }
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            s if !s.starts_with('-') => dir = &args[i],
            _ => {
                eprintln!("Unknown flag: {}", args[i]);
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let all_rules = rules::all_rules();

    if list_rules {
        println!("Available rules ({}):\n", all_rules.len());
        for rule in &all_rules {
            println!("  {:30} {}", rule.name(), rule.description());
        }
        return;
    }

    let active_rules: Vec<&dyn rule::Rule> = match &enabled_rules {
        Some(names) => all_rules
            .iter()
            .filter(|r| names.iter().any(|n| n == r.name()))
            .map(|r| r.as_ref())
            .collect(),
        None => all_rules.iter().map(|r| r.as_ref()).collect(),
    };

    if !Path::new(dir).exists() {
        eprintln!("Error: directory '{dir}' not found.");
        print_help();
        std::process::exit(1);
    }

    let start = Instant::now();

    let files = find_elm_files(dir);
    if files.is_empty() {
        eprintln!("No .elm files found in '{dir}'.");
        std::process::exit(1);
    }

    // Collect all module names for cross-module context.
    let mut project_modules = Vec::new();
    let mut parsed: Vec<(String, elm_ast_rs::file::ElmModule, String)> = Vec::new();
    let mut parse_errors = 0;

    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match elm_ast_rs::parse(&source) {
            Ok(module) => {
                let mod_name = match &module.header.value {
                    elm_ast_rs::module_header::ModuleHeader::Normal { name, .. }
                    | elm_ast_rs::module_header::ModuleHeader::Port { name, .. }
                    | elm_ast_rs::module_header::ModuleHeader::Effect { name, .. } => {
                        name.value.join(".")
                    }
                };
                project_modules.push(mod_name.clone());
                parsed.push((file.display().to_string(), module, source));
            }
            Err(errors) => {
                eprintln!("  warning: {}: {}", file.display(), errors[0]);
                parse_errors += 1;
            }
        }
    }

    let elapsed = start.elapsed();

    // Run all rules on all modules.
    let mut total_errors = 0;
    let mut file_errors: HashMap<String, Vec<rule::LintError>> = HashMap::new();

    for (file_path, module, source) in &parsed {
        let ctx = LintContext {
            module,
            source,
            file_path,
            project_modules: &project_modules,
        };

        for rule in &active_rules {
            let errors = rule.check(&ctx);
            if !errors.is_empty() {
                total_errors += errors.len();
                file_errors
                    .entry(file_path.clone())
                    .or_default()
                    .extend(errors);
            }
        }
    }

    eprintln!(
        "Linted {} files with {} rules in {:.1}ms",
        parsed.len(),
        active_rules.len(),
        elapsed.as_secs_f64() * 1000.0,
    );
    if parse_errors > 0 {
        eprintln!("  ({parse_errors} files had parse errors and were skipped)");
    }
    eprintln!();

    if total_errors == 0 {
        println!("No lint errors found.");
        return;
    }

    // Sort and display.
    let mut file_paths: Vec<&String> = file_errors.keys().collect();
    file_paths.sort();

    for path in file_paths {
        let errors = &file_errors[path];
        let mut sorted = errors.clone();
        sorted.sort_by_key(|e| (e.span.start.line, e.span.start.column));

        for err in &sorted {
            println!(
                "{}:{}:{}: [{}] {}",
                path, err.span.start.line, err.span.start.column, err.rule, err.message
            );
        }
    }

    println!();

    // Summary by rule.
    let mut by_rule: HashMap<&str, usize> = HashMap::new();
    for errors in file_errors.values() {
        for err in errors {
            *by_rule.entry(err.rule).or_default() += 1;
        }
    }
    let mut rule_counts: Vec<_> = by_rule.into_iter().collect();
    rule_counts.sort_by_key(|(_, v)| std::cmp::Reverse(*v));

    println!("{total_errors} errors in {} files", file_errors.len());
    for (rule, count) in &rule_counts {
        println!("  {count:>4} {rule}");
    }
}

fn print_help() {
    eprintln!("Usage: elm-lint [options] [src-directory]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --list           List all available rules");
    eprintln!("  --rules R1,R2    Only run specified rules");
    eprintln!("  --help           Show this help");
}

fn find_elm_files(dir: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_elm_files(&PathBuf::from(dir), &mut files);
    files.sort();
    files
}

fn collect_elm_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_elm_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "elm") {
                files.push(path);
            }
        }
    }
}
