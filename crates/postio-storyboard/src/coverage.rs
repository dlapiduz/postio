//! Every command does something a person can see (spec US6, FR-028/029).
//!
//! "I pressed it and nothing happened" is the defect that reads as the
//! application ignoring you, and the macOS app has six open issues of exactly
//! that shape. A generated pass presses every command bound in every context
//! from that context's starting state, and this decides what each press came
//! to: an **effect** (the frame or the shared observation changed), **no
//! effect** (neither did), or a **listed gap** (it is known to do nothing
//! yet, with a reason). A listed gap that now has an effect is **stale**: the
//! list is wrong, and the pass says so rather than letting it rot.

use std::collections::BTreeMap;
use std::path::Path;

use postio_ui::observe::Observation;
use serde::{Deserialize, Serialize};

/// One entry in `storyboards/gaps/<app>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    /// The command id.
    pub command: String,
    /// The key context, by name (`list`, `reader`, ...).
    pub context: String,
    /// Why it has no visible effect yet.
    pub reason: String,
    /// The issue that tracks it, when there is one.
    #[serde(default)]
    pub tracked: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct GapFile {
    #[serde(default)]
    gap: Vec<Gap>,
}

/// Reads a gap list. A missing file is an empty list.
pub fn load_gaps(path: &Path) -> Result<Vec<Gap>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    toml::from_str::<GapFile>(&text)
        .map(|file| file.gap)
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// What one press came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum Effect {
    /// Something a person can see changed.
    Effect,
    /// Nothing changed, and nothing says that is expected.
    NoEffect,
    /// Nothing changed, as the gap list says.
    ListedGap {
        /// The gap list's reason.
        reason: String,
    },
    /// The gap list says nothing should change, and something did.
    StaleGap {
        /// The reason the list still gives.
        reason: String,
    },
    /// The command has no binding in this context after all, so it could not
    /// be pressed.
    Unbound,
    /// The keyboard is in a text field and the binding is a bare key, so
    /// typing wins: the letter is typed, not run. Correct, and not a gap.
    Typing,
    /// The key was pressed and nothing on the focus chain took it.
    Dropped,
}

/// One command in one context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Press {
    /// The command id.
    pub command: String,
    /// The key context it was pressed in.
    pub context: String,
    /// What it came to.
    #[serde(flatten)]
    pub effect: Effect,
}

/// What a press came to, from the state before and after it and the gap list.
pub fn judge(
    command: &str,
    context: &str,
    before: (&Observation, &str),
    after: (&Observation, &str),
    gaps: &[Gap],
) -> Press {
    let changed = before.1 != after.1 || !before.0.shared_eq(after.0);
    let listed = gaps
        .iter()
        .find(|gap| gap.command == command && gap.context == context);
    let effect = match (changed, listed) {
        (true, None) => Effect::Effect,
        (true, Some(gap)) => Effect::StaleGap {
            reason: gap.reason.clone(),
        },
        (false, Some(gap)) => Effect::ListedGap {
            reason: gap.reason.clone(),
        },
        (false, None) => Effect::NoEffect,
    };
    Press {
        command: command.to_owned(),
        context: context.to_owned(),
        effect,
    }
}

/// Counts by result, for the summary line and the exit code.
pub fn tally(presses: &[Press]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for press in presses {
        let key = match press.effect {
            Effect::Effect => "effect",
            Effect::NoEffect => "no_effect",
            Effect::ListedGap { .. } => "listed_gap",
            Effect::StaleGap { .. } => "stale_gap",
            Effect::Unbound => "unbound",
            Effect::Typing => "typing",
            Effect::Dropped => "dropped",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::observation;

    fn gap(command: &str, context: &str) -> Gap {
        Gap {
            command: command.into(),
            context: context.into(),
            reason: "nothing to show in the small seed".into(),
            tracked: None,
        }
    }

    #[test]
    fn a_changed_frame_or_observation_is_an_effect() {
        let o = observation();
        let mut moved = o.clone();
        moved.cursor.index = Some(2);
        assert_eq!(
            judge("next_message", "list", (&o, "a"), (&moved, "a"), &[]).effect,
            Effect::Effect
        );
        assert_eq!(
            judge("flag", "list", (&o, "a"), (&o, "b"), &[]).effect,
            Effect::Effect
        );
    }

    #[test]
    fn nothing_changed_is_no_effect_unless_listed() {
        let o = observation();
        assert_eq!(
            judge("flag", "list", (&o, "a"), (&o, "a"), &[]).effect,
            Effect::NoEffect
        );
        let listed = judge("flag", "list", (&o, "a"), (&o, "a"), &[gap("flag", "list")]);
        assert!(matches!(listed.effect, Effect::ListedGap { .. }));
        let elsewhere = judge(
            "flag",
            "reader",
            (&o, "a"),
            (&o, "a"),
            &[gap("flag", "list")],
        );
        assert_eq!(elsewhere.effect, Effect::NoEffect, "a gap is per context");
    }

    #[test]
    fn a_listed_gap_that_now_does_something_is_stale() {
        let o = observation();
        let mut moved = o.clone();
        moved.notice.text = Some("Flagged".into());
        let press = judge(
            "flag",
            "list",
            (&o, "a"),
            (&moved, "a"),
            &[gap("flag", "list")],
        );
        assert!(matches!(press.effect, Effect::StaleGap { .. }));
    }

    #[test]
    fn the_widget_path_alone_is_no_effect() {
        let o = observation();
        let mut moved = o.clone();
        moved.keyboard.widget = "Elsewhere".into();
        assert_eq!(
            judge("flag", "list", (&o, "a"), (&moved, "a"), &[]).effect,
            Effect::NoEffect
        );
    }

    #[test]
    fn gap_lists_parse_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("terminal.toml");
        std::fs::write(
            &path,
            "[[gap]]\ncommand = \"show_images\"\ncontext = \"reader\"\nreason = \"no blocked images\"\ntracked = 1745\n",
        )
        .expect("written");
        let gaps = load_gaps(&path).expect("parses");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].tracked, Some(1745));
        assert_eq!(load_gaps(&dir.path().join("none.toml")), Ok(vec![]));
        std::fs::write(&path, "[[gap]]\ncommand = \"x\"\n").expect("written");
        assert!(
            load_gaps(&path).is_err(),
            "a gap without a context or reason is refused"
        );
    }

    #[test]
    fn the_tally_counts_each_result() {
        let o = observation();
        let presses = [
            judge("a", "list", (&o, "a"), (&o, "b"), &[]),
            judge("b", "list", (&o, "a"), (&o, "a"), &[]),
            judge("c", "list", (&o, "a"), (&o, "a"), &[]),
        ];
        let counts = tally(&presses);
        assert_eq!(counts.get("effect"), Some(&1));
        assert_eq!(counts.get("no_effect"), Some(&2));
    }

    #[test]
    fn typing_and_dropped_are_counted_apart_from_unbound() {
        let press = |effect| Press {
            command: "reply".into(),
            context: "composer".into(),
            effect,
        };
        let counts = tally(&[
            press(Effect::Typing),
            press(Effect::Dropped),
            press(Effect::Unbound),
        ]);
        assert_eq!(counts.get("typing"), Some(&1));
        assert_eq!(counts.get("dropped"), Some(&1));
        assert_eq!(counts.get("unbound"), Some(&1));
    }
}
