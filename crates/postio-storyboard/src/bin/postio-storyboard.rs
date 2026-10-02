//! The pure half's command line, which `scripts/storyboards.sh` drives.
//! Subcommands are in `specs/008-storyboards/contracts/runner.md`.
//!
//! Exit codes follow the script's: 0 clean, 1 something failed, 2 the
//! command could not run (a bad argument, an unreadable file).

use std::path::PathBuf;
use std::process::ExitCode;

use postio_storyboard::{lint, page};

const USAGE: &str = "\
usage:
  postio-storyboard lint <storyboards-dir>
  postio-storyboard page --runs <dir> --out <index.html> [--prefix <path>] [--title <t>] [--key <k>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("lint") => lint_command(&args[1..]),
        Some("page") => page_command(&args[1..]),
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
