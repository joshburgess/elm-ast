#![allow(clippy::collapsible_if)]

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use rayon::prelude::*;

use elm_lint::collect::collect_module_info;
use elm_lint::config::Config;
use elm_lint::elm_json;
use elm_lint::fix::apply_fixes;
use elm_lint::output;
use elm_lint::rule::{self, LintContext, ProjectContext};
use elm_lint::rules;
use elm_lint::watch;

/// Fast Elm linter with built-in rules.
#[derive(Parser)]
#[command(name = "elm-lint", version, about)]
struct Cli {
    /// Source directory to lint.
    #[arg(default_value = "src")]
    dir: String,

    /// List all available rules.
    #[arg(long)]
    list: bool,

    /// Only run specified rules (comma-separated).
    #[arg(long, value_delimiter = ',')]
    rules: Option<Vec<String>>,

    /// Disable specified rules (comma-separated).
    #[arg(long, value_delimiter = ',')]
    disable: Option<Vec<String>>,

    /// Apply auto-fixes interactively.
    #[arg(long, conflicts_with_all = ["fix_all", "watch"])]
    fix: bool,

    /// Apply all auto-fixes without prompting.
    #[arg(long, conflicts_with = "watch")]
    fix_all: bool,

    /// Output findings as JSON for editor integration.
    #[arg(long)]
    json: bool,

    /// Force colored output.
    #[arg(long, conflicts_with = "no_color")]
    color: bool,

    /// Disable colored output.
    #[arg(long)]
    no_color: bool,

    /// Path to config file (default: auto-discover elm-assist.toml).
    #[arg(long)]
    config: Option<String>,

    /// Re-run on file changes.
    #[arg(long)]
    watch: bool,
}

fn main() {
    let cli = Cli::parse();

    // --fix/--fix-all conflict with --json.
    if cli.json && (cli.fix || cli.fix_all) {
        eprintln!("Error: --json cannot be combined with --fix or --fix-all");
        std::process::exit(2);
    }

    // Load config.
    let config = if let Some(path) = &cli.config {
        match Config::load(Path::new(path)) {
            Ok(c) => {
                eprintln!("Using config: {path}");
                c
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
        }
    } else if let Some((path, c)) = Config::discover() {
        eprintln!("Using config: {}", path.display());
        c
    } else {
        Config::default()
    };

    let mut all_rules = rules::all_rules();

    // Apply per-rule config options.
    for rule in &mut all_rules {
        if let Some(options) = config.rule_options(rule.name()) {
            if let Err(e) = rule.configure(options) {
                eprintln!("Error configuring rule {}: {e}", rule.name());
                std::process::exit(2);
            }
        }
    }

    if cli.list {
        println!("Available rules ({}):\n", all_rules.len());
        for rule in &all_rules {
            let disabled = config.is_rule_disabled(rule.name());
            let marker = if disabled { " (disabled)" } else { "" };
            println!("  {:40} {}{}", rule.name(), rule.description(), marker);
        }
        return;
    }

    // Determine active rules: --rules overrides everything, then --disable + config.
    let active_rules: Vec<&dyn rule::Rule> = match &cli.rules {
        Some(names) => all_rules
            .iter()
            .filter(|r| names.iter().any(|n| n == r.name()))
            .map(|r| r.as_ref())
            .collect(),
        None => {
            let cli_disabled: Vec<&str> = cli
                .disable
                .as_ref()
                .map(|v| v.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();

            all_rules
                .iter()
                .filter(|r| {
                    !config.is_rule_disabled(r.name()) && !cli_disabled.contains(&r.name())
                })
                .map(|r| r.as_ref())
                .collect()
        }
    };

    // Resolve source directory (CLI overrides config).
    let dir = if cli.dir != "src" {
        &cli.dir
    } else {
        config.src.as_deref().unwrap_or("src")
    };

    if !Path::new(dir).exists() {
        eprintln!("Error: directory '{dir}' not found.");
        std::process::exit(2);
    }

    // Determine output format.
    let format = output::resolve_format(cli.json, cli.color, cli.no_color);

    if cli.watch {
        watch::run_watch_loop(dir, || {
            run_lint(dir, &active_rules, &config, &format);
        });
    }

    // One-shot mode.
    let (total_errors, file_errors, sources) = run_lint(dir, &active_rules, &config, &format);

    // Apply fixes if requested.
    if (cli.fix || cli.fix_all) && total_errors > 0 {
        println!();
        let fix_mode = if cli.fix_all {
            FixMode::All
        } else {
            FixMode::Interactive
        };
        let applied = apply_all_fixes(&file_errors, &sources, &fix_mode);
        if applied > 0 {
            println!("{applied} fixes applied.");
        } else {
            println!("No fixes applied.");
        }
    }

    // Exit code: 0 = clean, 1 = findings, 2 = error (handled above).
    if total_errors > 0 {
        std::process::exit(1);
    }
}

// ── Lint pipeline ──────────────────────────────────────────────────

/// Run the full lint pipeline: discover files, parse in parallel, collect module
/// info, build project context, run rules in parallel, report. Returns total
/// error count and the collected data (for fix application).
fn run_lint(
    dir: &str,
    active_rules: &[&dyn rule::Rule],
    config: &Config,
    format: &output::OutputFormat,
) -> (
    usize,
    HashMap<String, Vec<rule::LintError>>,
    HashMap<String, String>,
) {
    let start = Instant::now();

    let files = find_elm_files(dir);
    if files.is_empty() {
        eprintln!("No .elm files found in '{dir}'.");
        return (0, HashMap::new(), HashMap::new());
    }

    // Phase 1: Read + parse in parallel.
    let parse_results: Vec<Option<Result<_, String>>> = files
        .par_iter()
        .map(|file| {
            let source = fs::read_to_string(file).ok()?;
            let path_str = file.display().to_string();
            match elm_ast::parse(&source) {
                Ok(module) => {
                    let mod_name = extract_module_name(&module);
                    Some(Ok((path_str, mod_name, module, source)))
                }
                Err(errors) => Some(Err(format!("{}: {}", path_str, errors[0]))),
            }
        })
        .collect();

    // Split results (sequential, cheap).
    let mut parsed: Vec<(String, String, elm_ast::file::ElmModule, String)> = Vec::new();
    let mut project_modules = Vec::new();
    let mut parse_errors = 0;

    for result in parse_results.into_iter().flatten() {
        match result {
            Ok((path, mod_name, module, source)) => {
                project_modules.push(mod_name.clone());
                parsed.push((path, mod_name, module, source));
            }
            Err(msg) => {
                eprintln!("  warning: {msg}");
                parse_errors += 1;
            }
        }
    }

    // Phase 2: Collect ModuleInfo in parallel.
    let module_infos: HashMap<String, elm_lint::collect::ModuleInfo> = parsed
        .par_iter()
        .map(|(_path, mod_name, module, _source)| {
            (mod_name.clone(), collect_module_info(module))
        })
        .collect();

    // Load elm.json for dependency checking.
    let elm_json_info = elm_json::load_elm_json(Path::new(dir)).ok();

    // Phase 3: Build ProjectContext (sequential, single-shot aggregation).
    let project_context = ProjectContext::build_with_elm_json(module_infos, elm_json_info);

    // Phase 4: Run all rules on all files in parallel.
    let results: Vec<(String, String, Vec<rule::LintError>)> = parsed
        .par_iter()
        .map(|(file_path, mod_name, module, source)| {
            let ctx = LintContext {
                module,
                source,
                file_path,
                project_modules: &project_modules,
                module_info: project_context.modules.get(mod_name),
                project: Some(&project_context),
            };

            let mut file_lint_errors = Vec::new();
            for rule in active_rules {
                let mut errors = rule.check(&ctx);
                let severity = config
                    .severity_for(rule.name())
                    .unwrap_or(rule.default_severity());
                for err in &mut errors {
                    err.severity = severity;
                }
                file_lint_errors.extend(errors);
            }

            (file_path.clone(), source.clone(), file_lint_errors)
        })
        .collect();

    let elapsed = start.elapsed();

    // Assemble results.
    let mut file_errors: HashMap<String, Vec<rule::LintError>> = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for (path, source, errors) in results {
        if !errors.is_empty() {
            file_errors.insert(path.clone(), errors);
        }
        sources.insert(path, source);
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

    let total_errors: usize = file_errors.values().map(|v| v.len()).sum();

    // Report findings.
    output::report(format, &file_errors, &sources, parsed.len(), active_rules.len());
    output::report_summary(format, &file_errors);

    (total_errors, file_errors, sources)
}

fn extract_module_name(module: &elm_ast::file::ElmModule) -> String {
    match &module.header.value {
        elm_ast::module_header::ModuleHeader::Normal { name, .. }
        | elm_ast::module_header::ModuleHeader::Port { name, .. }
        | elm_ast::module_header::ModuleHeader::Effect { name, .. } => name.value.join("."),
    }
}

// ── Fix application ────────────────────────────────────────────────

enum FixMode {
    Interactive,
    All,
}

fn apply_all_fixes(
    file_errors: &HashMap<String, Vec<rule::LintError>>,
    sources: &HashMap<String, String>,
    fix_mode: &FixMode,
) -> usize {
    let mut total_applied = 0;

    let mut file_paths: Vec<&String> = file_errors.keys().collect();
    file_paths.sort();

    let stdin = io::stdin();
    let mut stdin_lines = stdin.lock().lines();

    for path in file_paths {
        let Some(source) = sources.get(path) else {
            continue;
        };

        let errors = &file_errors[path];
        let mut sorted = errors.clone();
        sorted.sort_by_key(|e| (e.span.start.line, e.span.start.column));

        let mut edits_to_apply = Vec::new();

        for err in &sorted {
            let Some(fix) = &err.fix else {
                continue;
            };

            match fix_mode {
                FixMode::All => {
                    edits_to_apply.extend(fix.edits.iter().cloned());
                }
                FixMode::Interactive => {
                    eprint!(
                        "{}:{}:{}: [{}] {} — apply fix? [y/n/q] ",
                        path, err.span.start.line, err.span.start.column, err.rule, err.message
                    );
                    io::stderr().flush().ok();

                    if let Some(Ok(line)) = stdin_lines.next() {
                        let answer = line.trim().to_lowercase();
                        if answer == "q" {
                            return total_applied;
                        }
                        if answer == "y" || answer == "yes" {
                            edits_to_apply.extend(fix.edits.iter().cloned());
                        }
                    }
                }
            }
        }

        if edits_to_apply.is_empty() {
            continue;
        }

        match apply_fixes(source, &edits_to_apply) {
            Ok(fixed) => {
                if elm_ast::parse(&fixed).is_err() {
                    eprintln!("  warning: fix for {path} produced invalid Elm, skipping");
                    continue;
                }
                match fs::write(path, &fixed) {
                    Ok(()) => {
                        let count = edits_to_apply.len();
                        total_applied += count;
                        eprintln!("  fixed {path} ({count} edits)");
                    }
                    Err(e) => {
                        eprintln!("  warning: could not write {path}: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("  warning: could not apply fixes to {path}: {e}");
            }
        }
    }

    total_applied
}

// ── File discovery ─────────────────────────────────────────────────

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
