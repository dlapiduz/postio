//! Every command Classic binds does something a person can see, or is on the
//! gap list with a reason (specs/008-storyboards US6, FR-028/029).
//!
//! The generated pass presses each command by its own chord, from a fresh
//! seeded window in each context's starting state, and judges whether the
//! frame or the shared observation changed. A command with no visible effect
//! that `storyboards/gaps/classic.toml` does not name fails this case, and so
//! does a listed gap that now has an effect -- the list is wrong, and saying
//! so is how it stays true.
//!
//! POSTIO-MEASUREMENT: it opens a fresh window for every command in every
//! context, about three minutes, so it runs on the nightly timer rather than
//! the merge path. `.config/nextest.toml`'s `profile.default` filter is what
//! holds it back; run it with
//!
//! ```text
//! cargo nextest run -p postio-app --test app_suite --profile nightly every_command
//! ```

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound.

use std::path::Path;

use gtk::gdk;
use postio_gtk::{app, fonts, style};
use postio_storyboard::coverage::Effect;

pub fn every_bound_command_shows_or_is_a_listed_gap() {
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

        let all = postio_app::demo::storyboard::every_command(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../storyboards/gaps/classic.toml"),
        )
        .await
        .expect("the gap list parses");
        let pressed: usize = all.iter().map(|c| c.presses.len()).sum();
        assert!(pressed > 50, "the pass pressed only {pressed} commands");
        let problems: Vec<String> = all
            .iter()
            .flat_map(|c| &c.presses)
            .filter_map(|press| match &press.effect {
                Effect::NoEffect => Some(format!(
                    "{} {}: no visible effect, and the gap list does not name it",
                    press.context, press.command
                )),
                Effect::StaleGap { reason } => Some(format!(
                    "{} {}: listed as a gap ({reason}) but now has an effect -- remove it",
                    press.context, press.command
                )),
                _ => None,
            })
            .collect();
        assert!(
            problems.is_empty(),
            "the generated pass found:\n  {}",
            problems.join("\n  ")
        );
    });
}
