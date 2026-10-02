//! Every storyboard in the catalogue holds on Classic (specs/008-storyboards,
//! research R14).
//!
//! POSTIO-MEASUREMENT: it plays the whole catalogue, one storyboard after
//! another, about three minutes, so it runs on the nightly timer rather than
//! in the merge path (`.config/nextest.toml`), beside `every_command`. It is
//! not a measurement: it is the nightly's read of the catalogue, which is
//! what the marker schedules. `scripts/storyboards.sh run` plays the same
//! catalogue in about a minute, sharded, when you want it now.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound.

use std::path::Path;

use gtk::gdk;
use postio_app::demo::storyboard::{Options, run};
use postio_gtk::{app, fonts, style};
use postio_storyboard::check::Outcome;
use postio_storyboard::run::{Status, StepOutcome};

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
                                    format!(
                                        "step {}: {} {} (saw {})",
                                        step.step,
                                        c.path,
                                        c.expected,
                                        c.observed.as_ref().map_or_else(
                                            || "absent".to_owned(),
                                            ToString::to_string
                                        )
                                    )
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
