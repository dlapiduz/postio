//! Every command Focus binds does something a person can see, or is on the
//! gap list with a reason (specs/008-storyboards US6, FR-028/029).
//!
//! The generated pass presses each command by its own chord, from a fresh
//! seeded window in each context's starting state, and judges whether the
//! frame or the shared observation changed. A command with no visible effect
//! that `storyboards/gaps/focus.toml` does not name fails this case, and so
//! does a listed gap that now has an effect -- the list is wrong, and saying
//! so is how it stays true.
//!
//! POSTIO-MEASUREMENT: it opens a fresh window for every command in every
//! context, about six minutes, so it runs on the nightly timer rather than
//! the merge path. `.config/nextest.toml`'s `profile.default` filter is what
//! holds it back; run it with
//!
//! ```text
//! cargo nextest run -p postio-focus --test focus_suite --profile nightly every_command
//! ```

use std::path::Path;

use postio_storyboard::coverage::Effect;

use crate::support;

pub fn every_bound_command_shows_or_is_a_listed_gap() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let all = postio_focus::demo::storyboard::every_command(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../storyboards/gaps/focus.toml"),
        )
        .await
        .expect("the gap list parses");
        let unreached: Vec<String> = all
            .iter()
            .filter_map(|c| {
                c.unreachable
                    .as_ref()
                    .map(|why| format!("{}: {why}", c.context))
            })
            .collect();
        assert!(
            unreached.is_empty(),
            "a context the pass could not reach:\n  {}",
            unreached.join("\n  ")
        );
        let pressed: usize = all.iter().map(|c| c.presses.len()).sum();
        assert!(pressed > 100, "the pass pressed only {pressed} commands");
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
