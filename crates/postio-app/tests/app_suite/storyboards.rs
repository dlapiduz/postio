//! A storyboard, played on Classic through the real composition root.
//!
//! The runner core (`postio_app::demo::storyboard`) is what every Classic
//! filmstrip and every catalogue check goes through, so this pins what it
//! promises (specs/008-storyboards contracts/runner.md): a command is
//! *pressed*, by the binding Classic has for it in the context it is in, so a
//! broken binding fails the step; every step is observed and checked; and
//! `run.json` and the frames land where the output tree says.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound.

use std::path::Path;

use gtk::gdk;
use postio_app::demo::storyboard::{Options, run};
use postio_gtk::{app, fonts, style};
use postio_storyboard::check::Outcome;
use postio_storyboard::format::parse;
use postio_storyboard::run::{Status, StepOutcome};

const WALK: &str = r#"
source = { kind = "flow", ref = "T039 fixture" }

[[step]]
id = "down"
command = "next_message"
check = { keyboard.region = "list", cursor.index = 1 }

[[step]]
command = "archive"
check = { cursor.index = { same_as = "down" }, notice.undo = true }
"#;

// `send` is the composer's; the list has no binding for it.
const UNBOUND: &str = r#"
source = { kind = "flow", ref = "T039 fixture" }

[[step]]
command = "send"
"#;

pub fn a_storyboard_plays_on_classic_and_writes_its_run() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let walk = parse(WALK, Path::new("storyboards/list/fixture-walk.toml")).expect("it loads");
        let out = tempfile::tempdir().expect("an output directory");
        let played = run(
            &walk,
            &Options {
                out: Some(out.path().to_path_buf()),
                ..Options::default()
            },
        )
        .await;

        assert_eq!(played.status, Status::Passed, "{played:#?}");
        assert_eq!(played.steps.len(), 3, "the starting state and two steps");
        let down = &played.steps[1];
        assert_eq!(down.outcome, StepOutcome::Delivered);
        let delivered = down.input.as_ref().expect("what was delivered");
        assert_eq!(
            delivered.chord.as_deref(),
            Some("j"),
            "next_message is pressed as Classic's own binding"
        );
        assert_eq!(delivered.context.as_deref(), Some("list"));
        assert!(
            played
                .steps
                .iter()
                .flat_map(|step| &step.checks)
                .all(|check| check.outcome == Outcome::Pass),
            "{played:#?}"
        );

        let dir = out.path().join("classic/fixture-walk/default");
        assert!(dir.join("run.json").is_file(), "run.json is written");
        for step in 0..3 {
            assert!(dir.join(format!("{step:02}.png")).is_file(), "frame {step}");
            assert!(
                dir.join(format!("{step:02}.outlined.png")).is_file(),
                "outlined frame {step}"
            );
        }

        // `--no-frames`: the same checks, nothing drawn to disk.
        let quiet = tempfile::tempdir().expect("an output directory");
        let played = run(
            &walk,
            &Options {
                out: Some(quiet.path().to_path_buf()),
                frames: false,
                ..Options::default()
            },
        )
        .await;
        assert_eq!(played.status, Status::Passed, "{played:#?}");
        let dir = quiet.path().join("classic/fixture-walk/default");
        assert!(dir.join("run.json").is_file());
        let pngs = std::fs::read_dir(&dir)
            .expect("the run's directory")
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "png"))
            })
            .count();
        assert_eq!(pngs, 0, "no frames were asked for");

        // A command the context does not bind fails the step, rather than
        // being dispatched by name past a broken binding.
        let unbound =
            parse(UNBOUND, Path::new("storyboards/list/fixture-unbound.toml")).expect("it loads");
        let played = run(&unbound, &Options::default()).await;
        assert_eq!(played.status, Status::Failed, "{played:#?}");
        assert_eq!(
            played.steps[1].outcome,
            StepOutcome::Unbound {
                context: "list".into()
            }
        );
    });
}

/// Every storyboard in the catalogue holds on Classic -- the nightly's read
/// of the catalogue (research R14).
///
/// Frames off: this is the checks, not the pictures. A storyboard whose
/// defect is still open (`proof = "open"`) is red by design and counts as
/// expected; one that starts passing is reported, because its issue may be
/// fixed and the storyboard should be pinned. Not applicable, not covered
/// and unavailable are counted, never failed: they are what the run could
/// not check, and the page shows them.
pub fn the_catalogue_holds_on_classic() {
    use postio_storyboard::format::{Proof, load};

    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../storyboards");
        let mut files = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).expect("the catalogue").flatten() {
                let path = entry.path();
                let skip = ["calibration", "gaps"]
                    .iter()
                    .any(|name| path.file_name().is_some_and(|f| f == *name));
                if path.is_dir() && !skip {
                    pending.push(path);
                } else if path.extension().is_some_and(|x| x == "toml") {
                    files.push(path);
                }
            }
        }
        files.sort();
        assert!(!files.is_empty(), "the catalogue holds no storyboards");

        let mut problems = Vec::new();
        for file in &files {
            let board = load(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let played = run(
                &board,
                &Options {
                    frames: false,
                    ..Options::default()
                },
            )
            .await;
            let open = board.proof == Some(Proof::Open);
            match (&played.status, open) {
                (Status::Failed, true) | (Status::Passed, false) => {}
                (Status::Failed, false) => {
                    let failed: Vec<String> = played
                        .steps
                        .iter()
                        .flat_map(|step| {
                            step.checks
                                .iter()
                                .filter(|c| c.outcome == Outcome::Fail)
                                .map(move |c| {
                                    format!("step {}: {} {}", step.step, c.path, c.expected)
                                })
                                .chain(
                                    (step.outcome != StepOutcome::Delivered)
                                        .then(|| format!("step {}: {:?}", step.step, step.outcome)),
                                )
                        })
                        .collect();
                    problems.push(format!("{}: failed -- {}", board.name, failed.join("; ")));
                }
                (Status::Passed, true) => problems.push(format!(
                    "{}: passes, but its defect is marked open -- if its issue is fixed, \
                     make it proof = \"pinned\"",
                    board.name
                )),
                (Status::Error { message }, _) => {
                    problems.push(format!("{}: the runner errored: {message}", board.name));
                }
                _ => {}
            }
        }
        assert!(
            problems.is_empty(),
            "the catalogue does not hold on Classic:\n  {}",
            problems.join("\n  ")
        );
    });
}
