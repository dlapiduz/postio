//! Every storyboard in the catalogue holds on Focus (specs/008-storyboards,
//! research R14).
//!
//! POSTIO-MEASUREMENT: it plays the whole catalogue, one storyboard after
//! another, several minutes, so it runs on the nightly timer rather than in
//! the merge path (`.config/nextest.toml`), beside `every_command`. It is
//! not a measurement: it is the nightly's read of the catalogue, which is
//! what the marker schedules. `scripts/storyboards.sh run` plays the same
//! catalogue sharded, when you want it now; run this one with
//!
//! ```text
//! cargo nextest run -p postio-focus --test focus_suite --profile nightly storyboard_catalogue
//! ```

use std::path::Path;

use postio_focus::demo::storyboard::{Options, run};
use postio_storyboard::check::Outcome;
use postio_storyboard::format::{Proof, load};
use postio_storyboard::run::{Status, StepOutcome};

use crate::support;

/// Every storyboard in the catalogue holds on Focus -- the nightly's read
/// of the catalogue (research R14).
///
/// Frames off: this is the checks, not the pictures. A storyboard whose
/// defect is still open (`proof = "open"`) is red by design and counts as
/// expected; one that starts passing is reported, because its issue may be
/// fixed and the storyboard should be pinned. Not applicable, not covered
/// and unavailable are counted, never failed: they are what the run could
/// not check, and the page shows them.
pub fn the_catalogue_holds_on_focus() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
            "the catalogue does not hold on Focus:\n  {}",
            problems.join("\n  ")
        );
    });
}
