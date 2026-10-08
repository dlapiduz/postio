//! A storyboard, played on Focus through the real window.
//!
//! The runner core (`postio_gtk::demo::storyboard`) is what every Focus
//! filmstrip goes through, so this pins what it promises
//! (specs/008-storyboards contracts/runner.md): a command is *pressed*, by
//! the binding Focus has for it in the context it is in, so a broken binding
//! fails the step; every step is observed and checked; and `run.json` and
//! the frames land where the output tree says.

use std::path::Path;

use postio_gtk::demo::storyboard::{Options, run};
use postio_storyboard::check::Outcome;
use postio_storyboard::format::parse;
use postio_storyboard::run::{Status, StepOutcome};

use crate::support;

const WALK: &str = r#"
source = { kind = "flow", ref = "T076 fixture" }

[[step]]
id = "down"
command = "next_message"
# Focus opens with the cursor on the first row; `j` moves it to the second.
check = { keyboard.region = "list", cursor.index = { changed = true } }

[[step]]
command = "archive"
check = { notice.undo = true }
"#;

// `send` is the composer's; the list has no binding for it.
const UNBOUND: &str = r#"
source = { kind = "flow", ref = "T076 fixture" }

[[step]]
command = "send"
"#;

pub fn a_storyboard_plays_on_focus_and_writes_its_run() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
            "next_message is pressed as Focus's own binding"
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

        let dir = out.path().join("focus/fixture-walk/default");
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
        let dir = quiet.path().join("focus/fixture-walk/default");
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
