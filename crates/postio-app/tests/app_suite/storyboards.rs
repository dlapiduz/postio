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
