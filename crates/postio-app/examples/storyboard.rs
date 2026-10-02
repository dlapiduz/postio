//! Classic's storyboard runner, as a command (specs/008-storyboards,
//! contracts/runner.md).
//!
//! ```sh
//! cargo run -p postio-app --example storyboard --features demo -- list
//! cargo run -p postio-app --example storyboard --features demo -- \
//!     run storyboards/list/archive-walks-down.toml --out /tmp/runs
//! ```
//!
//! `scripts/storyboards.sh run` is how people and skills drive it; it builds
//! this once and puts it on the private headless compositor.
//!
//! # Why it re-executes itself
//!
//! A storyboard's frames are compared byte for byte -- run against run, and
//! branch against base -- so everything that decides a pixel has to be the
//! same every time (research R5). Most of that has to be settled before GTK
//! starts, which is why the process starts itself again with:
//!
//! * `TZ=UTC` and a C UTF-8 locale, so dates and collation do not follow the
//!   machine;
//! * a fontconfig that knows only the faces Postio embeds, so a system font
//!   update cannot move a glyph;
//! * `GSK_RENDERER=cairo`, the renderer that measured byte-identical across
//!   processes (docs/notes/2026-10-01-what-a-storyboard-capture-costs.md);
//! * throwaway `XDG_*` directories, so no user setting or state leaks in and
//!   nothing is left behind;
//! * `GTK_A11Y=test`, so no accessibility bus is dialled.
//!
//! The clock and animations are held still inside the runner itself.
//!
//! Exit status: 0 every run passed (or was not applicable or not covered);
//! 1 a run failed; 2 a storyboard did not load or a runner errored.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use gtk::gdk;
use postio_app::demo::storyboard::{Options, run, runner_info};
use postio_gtk::{app, fonts, style};
use postio_storyboard::format::load;
use postio_storyboard::run::{Delivery, Status};
use postio_ui::reader::document::FACES;

const HERMETIC: &str = "POSTIO_STORYBOARD_HERMETIC";

const USAGE: &str = "\
usage:
  storyboard list
  storyboard run <storyboard.toml>... --out <dir> [--no-frames] [--delivery chain|direct]
                 [--variants | --variant <axis>=<value>...] [--tree-key <key>] [--commit <sha>]
  storyboard every-command --out <dir> [--gaps <storyboards/gaps/classic.toml>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os(HERMETIC).is_none() {
        return reexec(&args);
    }
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("run") => play(&args[1..]),
        Some("every-command") => every_command(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Starts this binary again inside the environment research R5 fixes.
fn reexec(args: &[String]) -> ExitCode {
    let Ok(scratch) = tempfile::tempdir() else {
        eprintln!("storyboard: no temporary directory for the hermetic run");
        return ExitCode::from(2);
    };
    let root = scratch.path();
    let fonts = root.join("fonts");
    for dir in ["fonts", "fontcache", "config", "state", "cache", "data"] {
        if std::fs::create_dir_all(root.join(dir)).is_err() {
            eprintln!("storyboard: cannot prepare {}", root.join(dir).display());
            return ExitCode::from(2);
        }
    }
    for face in FACES {
        if std::fs::write(fonts.join(face.name), face.bytes).is_err() {
            eprintln!("storyboard: cannot unpack the embedded font {}", face.name);
            return ExitCode::from(2);
        }
    }
    let conf = root.join("fonts.conf");
    let written = std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
             <fontconfig>\n  <dir>{}</dir>\n  <cachedir>{}</cachedir>\n</fontconfig>\n",
            fonts.display(),
            root.join("fontcache").display()
        ),
    );
    if written.is_err() {
        eprintln!("storyboard: cannot write the fontconfig for the hermetic run");
        return ExitCode::from(2);
    }
    let Ok(me) = std::env::current_exe() else {
        eprintln!("storyboard: cannot find this binary to start it again");
        return ExitCode::from(2);
    };
    let renderer = std::env::var("POSTIO_STORYBOARD_RENDERER").unwrap_or_else(|_| "cairo".into());
    let status = Command::new(me)
        .args(args)
        .env(HERMETIC, "1")
        .env("TZ", "UTC")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("FONTCONFIG_FILE", &conf)
        .env("GSK_RENDERER", renderer)
        .env("GTK_A11Y", "test")
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_DATA_HOME", root.join("data"))
        .status();
    match status {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(2)),
            None => {
                eprintln!("storyboard: the runner was killed by a signal");
                ExitCode::from(2)
            }
        },
        Err(error) => {
            eprintln!("storyboard: cannot start the hermetic run: {error}");
            ExitCode::from(2)
        }
    }
}

fn list() -> ExitCode {
    let info = runner_info();
    let described = serde_json::json!({
        "app": info.app,
        "delivery": ["chain", "direct"],
        "seeds": info.seeds,
        "presets": info.presets,
        "axes": info.axes,
        "unobserved": postio_app::demo::storyboard::UNOBSERVED,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&described).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

/// The value after `--name`, if given.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn play(args: &[String]) -> ExitCode {
    let Some(out) = flag(args, "--out") else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    // Everything that is neither a flag nor a flag's value is a storyboard.
    let takes_value = ["--out", "--delivery", "--variant", "--tree-key", "--commit"];
    let mut files = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
        } else if takes_value.contains(&arg.as_str()) {
            skip = true;
        } else if !arg.starts_with("--") {
            files.push(PathBuf::from(arg));
        }
    }
    if files.is_empty() {
        eprintln!("storyboard: no storyboard to run\n{USAGE}");
        return ExitCode::from(2);
    }
    let delivery = match flag(args, "--delivery").as_deref() {
        None | Some("chain") => Delivery::Chain,
        Some("direct") => Delivery::Direct,
        Some(other) => {
            eprintln!("storyboard: unknown delivery `{other}`");
            return ExitCode::from(2);
        }
    };
    let mut variant = BTreeMap::new();
    for (i, arg) in args.iter().enumerate() {
        if arg == "--variant"
            && let Some((axis, value)) = args.get(i + 1).and_then(|v| v.split_once('='))
        {
            variant.insert(axis.to_owned(), value.to_owned());
        }
    }
    let options = Options {
        out: Some(PathBuf::from(out)),
        frames: !args.iter().any(|arg| arg == "--no-frames"),
        delivery,
        variant,
        tree_key: flag(args, "--tree-key").unwrap_or_default(),
        commit: flag(args, "--commit").unwrap_or_default(),
        ..Options::default()
    };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("storyboard: no display; run it under scripts/test-headless.sh");
        return ExitCode::from(2);
    }
    let display = gdk::Display::default().expect("a display");
    if let Err(error) = fonts::install() {
        eprintln!("storyboard: {error}");
        return ExitCode::from(2);
    }
    style::install(&display);
    app::install_icons(&display);

    let mut worst = 0u8;
    for file in &files {
        let board = match load(file) {
            Ok(board) => board,
            Err(error) => {
                eprintln!("{}: {error}", file.display());
                worst = 2;
                continue;
            }
        };
        // Each variant the storyboard asks for and Classic supports, with
        // --variants; otherwise the one variant the flags named (or none).
        let variants = if args.iter().any(|arg| arg == "--variants") {
            postio_app::demo::storyboard::variants_for(&board)
        } else {
            (vec![options.variant.clone()], Vec::new())
        };
        for variant in variants.0 {
            let options = Options {
                variant,
                ignored_axes: variants.1.clone(),
                ..options.clone()
            };
            worst = worst.max(play_one(&board, &options));
        }
    }
    ExitCode::from(worst)
}

/// Plays one storyboard in one variant, says how it went, and returns the
/// exit code it is worth.
fn play_one(board: &postio_storyboard::format::Storyboard, options: &Options) -> u8 {
    {
        let played = postio_app::demo::on_runtime(run(board, options));
        let (word, code) = match &played.status {
            Status::Passed => ("passed".to_owned(), 0),
            Status::Failed => ("FAILED".to_owned(), 1),
            Status::NotApplicable { reason } => (format!("not applicable: {reason}"), 0),
            Status::NotCovered { reason } => (format!("not covered: {reason}"), 0),
            Status::Unavailable { reason } => (format!("unavailable: {reason}"), 0),
            Status::Error { message } => (format!("ERROR: {message}"), 2),
        };
        let variant = postio_storyboard::run::variant_key(&options.variant);
        if variant == "default" {
            println!("classic {}: {word}", board.name);
        } else {
            println!("classic {} [{variant}]: {word}", board.name);
        }
        if played.status == Status::Failed {
            for step in &played.steps {
                for check in step
                    .checks
                    .iter()
                    .filter(|c| c.outcome == postio_storyboard::check::Outcome::Fail)
                {
                    println!(
                        "    step {}: {} expected {}, saw {}",
                        step.step,
                        check.path,
                        check.expected,
                        check
                            .observed
                            .as_ref()
                            .map_or_else(|| "absent".to_owned(), ToString::to_string)
                    );
                }
                if !matches!(step.outcome, postio_storyboard::run::StepOutcome::Delivered) {
                    println!("    step {}: {:?}", step.step, step.outcome);
                }
            }
        }
        code
    }
}

/// Starts GTK the way every subcommand that draws needs it.
fn start_gtk() -> Result<(), ExitCode> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("storyboard: no display; run it under scripts/test-headless.sh");
        return Err(ExitCode::from(2));
    }
    let display = gdk::Display::default().expect("a display");
    if let Err(error) = fonts::install() {
        eprintln!("storyboard: {error}");
        return Err(ExitCode::from(2));
    }
    style::install(&display);
    app::install_icons(&display);
    Ok(())
}

/// The generated pass (spec US6): every command in every context, judged.
/// Writes `coverage.json`; exits 1 on any command with no visible effect
/// that the gap list does not name, or any stale gap.
fn every_command(args: &[String]) -> ExitCode {
    let Some(out) = flag(args, "--out") else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let gaps = flag(args, "--gaps").map(PathBuf::from).unwrap_or_default();
    if let Err(code) = start_gtk() {
        return code;
    }
    let all = match postio_app::demo::on_runtime(postio_app::demo::storyboard::every_command(&gaps))
    {
        Ok(all) => all,
        Err(error) => {
            eprintln!("storyboard: {error}");
            return ExitCode::from(2);
        }
    };
    let presses: Vec<_> = all.iter().flat_map(|c| c.presses.iter().cloned()).collect();
    let counts = postio_storyboard::coverage::tally(&presses);
    for coverage in &all {
        if let Some(why) = &coverage.unreachable {
            println!("{}: unreachable -- {why}", coverage.context);
        }
        for press in &coverage.presses {
            use postio_storyboard::coverage::Effect;
            match &press.effect {
                Effect::NoEffect => {
                    println!("{} {}: NO VISIBLE EFFECT", press.context, press.command)
                }
                Effect::StaleGap { reason } => println!(
                    "{} {}: STALE GAP (now has an effect; listed because: {reason})",
                    press.context, press.command
                ),
                _ => {}
            }
        }
    }
    println!("coverage: {counts:?}");
    let json = serde_json::Value::Array(all.iter().map(|c| c.to_json()).collect());
    let dir = PathBuf::from(&out).join("classic");
    if std::fs::create_dir_all(&dir).is_err()
        || std::fs::write(dir.join("coverage.json"), json.to_string()).is_err()
    {
        eprintln!(
            "storyboard: cannot write {}",
            dir.join("coverage.json").display()
        );
        return ExitCode::from(2);
    }
    let bad = counts.get("no_effect").copied().unwrap_or(0)
        + counts.get("stale_gap").copied().unwrap_or(0);
    if bad > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
