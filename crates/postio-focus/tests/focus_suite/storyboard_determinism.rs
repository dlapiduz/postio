//! Two processes film the same storyboard identically on Focus (spec 008
//! SC-003).
//!
//! A storyboard's frames are compared byte for byte -- a branch against its
//! base, a run against the one before it -- and that comparison is worthless
//! if the same build can draw the same step two ways. The default renderer
//! did exactly that across processes (docs/notes/2026-10-01-what-a-
//! storyboard-capture-costs.md), and the clock leaked into the rows until
//! they read the seam, so this asserts the end result rather than any one
//! cause: two separate processes, the same storyboard, the same observations
//! and the same pixels.
//!
//! One case, two roles. Run plainly, it starts this binary twice more on
//! itself with `POSTIO_DETERMINISM_OUT` set; each child plays the storyboard
//! into its own directory and returns. The parent compares what they wrote.
//! Separate processes, because sameness *within* one process is the easy
//! half, and the base-versus-branch diff is always across two.
//!
//! The children play on a compositor of their own, as
//! `scripts/storyboards.sh` does: on the suite's shared one, another case's
//! window can be the active one, and an inactive window draws in GTK's
//! backdrop style, so the same step came out two ways (seen when this ran
//! beside `observe` and `storyboards`).

use std::path::{Path, PathBuf};
use std::process::Command;

use postio_focus::demo::storyboard::{Options, run};
use postio_storyboard::format::parse;
use postio_storyboard::run::{Run, Status};

use crate::support;

const NAME: &str = "storyboard_determinism::two_processes_film_a_storyboard_identically";
const CHILD: &str = "POSTIO_DETERMINISM_OUT";

const WALK: &str = r#"
source = { kind = "flow", ref = "SC-003 fixture" }

[[step]]
command = "next_message"

[[step]]
command = "next_message"

[[step]]
command = "archive"
"#;

fn read(dir: &Path) -> Run {
    let path = dir.join("focus/determinism-walk/default/run.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("a run")
}

pub fn two_processes_film_a_storyboard_identically() {
    if let Some(out) = std::env::var_os(CHILD) {
        child(PathBuf::from(out));
        return;
    }
    if !support::display() {
        return;
    }

    let first = tempfile::tempdir().expect("a directory for the first run");
    let second = tempfile::tempdir().expect("a directory for the second run");
    let me = std::env::current_exe().expect("this binary");
    let headless = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-headless.sh");
    let display = format!("postio-determinism-{}", std::process::id());
    for out in [first.path(), second.path()] {
        let status = Command::new(&headless)
            .arg(&me)
            .args([NAME, "--exact"])
            .env("POSTIO_TEST_DISPLAY", &display)
            .env("POSTIO_TEST_GEOMETRY", "1920x1200")
            .env(CHILD, out)
            // The renderer the runner pins. Nameable, because naming the
            // default one is how this was seen failing: it draws the same
            // step two ways in two processes.
            .env(
                "GSK_RENDERER",
                std::env::var("POSTIO_DETERMINISM_RENDERER").unwrap_or_else(|_| "cairo".into()),
            )
            .env("TZ", "UTC")
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .status()
            .expect("the child starts");
        assert!(status.success(), "a child run failed: {status}");
    }
    let _ = Command::new(&headless)
        .arg("--stop")
        .env("POSTIO_TEST_DISPLAY", &display)
        .status();
    let (a, b) = (read(first.path()), read(second.path()));
    assert_eq!(a.status, Status::Passed, "{a:#?}");
    assert_eq!(a.steps.len(), 4, "the starting state and three steps");

    for (x, y) in a.steps.iter().zip(&b.steps) {
        assert_eq!(
            x.observation, y.observation,
            "step {} was observed differently by two processes",
            x.step
        );
        assert_eq!(x.checks, y.checks, "step {}", x.step);
        let (fx, fy) = (
            x.frame.as_ref().expect("a frame"),
            y.frame.as_ref().expect("a frame"),
        );
        assert_eq!(
            fx.hash, fy.hash,
            "step {}'s frame differs between two processes",
            x.step
        );
        // The outlined frame too: it is what a reviewer reads, and an outline
        // that wandered would make every review read a change.
        let outlined = |dir: &Path, step: &postio_storyboard::run::StepRun| {
            std::fs::read(
                dir.join("focus/determinism-walk/default")
                    .join(step.outlined.as_ref().expect("an outlined frame")),
            )
            .expect("the outlined frame is on disk")
        };
        assert!(
            outlined(first.path(), x) == outlined(second.path(), y),
            "step {}'s outlined frame differs between two processes",
            x.step
        );
    }
}

fn child(out: PathBuf) {
    crate::gtk_case(async move {
        assert!(
            support::display(),
            "the child has no display, so it cannot film anything"
        );
        let walk =
            parse(WALK, Path::new("storyboards/list/determinism-walk.toml")).expect("it loads");
        let played = run(
            &walk,
            &Options {
                out: Some(out),
                ..Options::default()
            },
        )
        .await;
        assert_eq!(played.status, Status::Passed, "{played:#?}");
    });
}
