//! The pure half's command line, which `scripts/storyboards.sh` drives.
//! Subcommands are in `specs/008-storyboards/contracts/runner.md`.
//!
//! Exit codes follow the script's: 0 clean, 1 something failed, 2 the
//! command could not run (a bad argument, an unreadable file).

use std::path::PathBuf;
use std::process::ExitCode;

use postio_storyboard::{bundle, key, lint, page, prompt};

const USAGE: &str = "\
usage:
  postio-storyboard lint <storyboards-dir>
  postio-storyboard key --tree <path=id>...
  postio-storyboard bundle --runs <dir> [--base <dir> [--base-sha <sha>]] --acceptance <file>
                           --catalogue <storyboards-dir> --design-dir <dir>... --out <bundle-dir>
  postio-storyboard prompt <bundle> (--list | --batch <n>)
  postio-storyboard page --runs <dir> --out <index.html> [--prefix <path>] [--title <t>] [--key <k>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("lint") => lint_command(&args[1..]),
        Some("page") => page_command(&args[1..]),
        Some("key") => key_command(&args[1..]),
        Some("bundle") => bundle_command(&args[1..]),
        Some("prompt") => prompt_command(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The value after `--name`, if given.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Every value after `--name`, for a flag that may repeat.
fn flags(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(i, arg)| *i > 0 && args[i - 1] == name && !arg.starts_with("--"))
        .map(|(_, arg)| arg.clone())
        .collect()
}

fn bundle_command(args: &[String]) -> ExitCode {
    let (Some(runs), Some(acceptance), Some(catalogue), Some(out)) = (
        flag(args, "--runs"),
        flag(args, "--acceptance"),
        flag(args, "--catalogue"),
        flag(args, "--out"),
    ) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let base = flag(args, "--base").map(PathBuf::from);
    let design_dirs: Vec<PathBuf> = flags(args, "--design-dir")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let inputs = bundle::Inputs {
        runs: &PathBuf::from(&runs),
        base: base.as_deref(),
        base_sha: flag(args, "--base-sha"),
        acceptance: &PathBuf::from(&acceptance),
        catalogue: &PathBuf::from(&catalogue),
        design_dirs: &design_dirs,
        out: &PathBuf::from(&out),
    };
    match bundle::build(&inputs, &bundle::all_new) {
        Ok(manifest) => {
            let steps: usize = manifest.batches.iter().map(|b| b.steps.len()).sum();
            println!(
                "{out}: {} batch(es), {steps} step(s) to review, {} unchanged run(s)",
                manifest.batches.len(),
                manifest.unchanged
            );
            for name in &manifest.design_missing {
                eprintln!(
                    "postio-storyboard bundle: no design screen `{name}` in any --design-dir"
                );
            }
            ExitCode::SUCCESS
        }
        Err(error @ bundle::BundleError::ForbiddenDesign(_)) => {
            eprintln!("postio-storyboard bundle: {error}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("postio-storyboard bundle: {error}");
            ExitCode::from(2)
        }
    }
}

fn prompt_command(args: &[String]) -> ExitCode {
    let Some(bundle_dir) = args.first().map(PathBuf::from) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let manifest = match prompt::load_manifest(&bundle_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("postio-storyboard prompt: {error}");
            return ExitCode::from(2);
        }
    };
    if args.iter().any(|arg| arg == "--list") {
        println!("{}", prompt::count(&manifest));
        return ExitCode::SUCCESS;
    }
    // One batch needs no number; several must say which.
    let n = match flag(args, "--batch") {
        Some(n) => n.parse::<usize>().ok(),
        None if prompt::count(&manifest) == 1 => Some(1),
        None => None,
    };
    let Some(text) = n.and_then(|n| prompt::render(&manifest, n, &bundle_dir)) else {
        eprintln!(
            "postio-storyboard prompt: choose --batch 1..={} (--list counts them)",
            prompt::count(&manifest)
        );
        return ExitCode::from(2);
    };
    eprintln!("template blake3 {}", prompt::template_hash());
    print!("{text}");
    ExitCode::SUCCESS
}

fn lint_command(args: &[String]) -> ExitCode {
    let Some(root) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let problems = lint::lint_catalogue(&PathBuf::from(root));
    if problems.is_empty() {
        println!("storyboards: the catalogue lints clean");
        ExitCode::SUCCESS
    } else {
        for (file, message) in &problems {
            eprintln!("{}: {message}", file.display());
        }
        eprintln!("storyboards: {} problem(s)", problems.len());
        ExitCode::FAILURE
    }
}

fn page_command(args: &[String]) -> ExitCode {
    let (Some(runs), Some(out)) = (flag(args, "--runs"), flag(args, "--out")) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let prefix = flag(args, "--prefix").unwrap_or_else(|| "runs".to_owned());
    let strips = match page::collect(&PathBuf::from(&runs), &prefix) {
        Ok(strips) => strips,
        Err(error) => {
            eprintln!("postio-storyboard page: {runs}: {error}");
            return ExitCode::from(2);
        }
    };
    let header = page::Header {
        title: flag(args, "--title").unwrap_or_default(),
        tree_key: flag(args, "--key"),
    };
    let html = page::render(&header, &strips);
    if let Err(error) = std::fs::write(&out, html) {
        eprintln!("postio-storyboard page: {out}: {error}");
        return ExitCode::from(2);
    }
    println!("{out} ({} runs)", strips.len());
    ExitCode::SUCCESS
}

fn key_command(args: &[String]) -> ExitCode {
    let trees: Vec<&str> = args
        .iter()
        .enumerate()
        .filter(|(i, arg)| *i > 0 && args[i - 1] == "--tree" && !arg.starts_with("--"))
        .map(|(_, arg)| arg.as_str())
        .collect();
    if trees.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    println!("{}", key::key(&trees));
    ExitCode::SUCCESS
}
