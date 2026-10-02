//! One storyboard, every app: where the apps agree, and where they diverge
//! (spec US3, FR-016).
//!
//! A shared storyboard describes one interaction for every app it applies
//! to. Where an app legitimately behaves differently -- Focus opens a dialog
//! where Classic fills a pane -- the storyboard says so with an override for
//! that app and step. Anywhere else, two apps reporting different shared
//! observations after the same step is a **divergence**: one of them is wrong,
//! or an override is missing, and either way somebody should look. Side by
//! side is the only view that shows it, because each app may pass its own
//! checks.
//!
//! Compared on [`Observation::shared_eq`]: the widget path and `app.*` name a
//! toolkit's internals, and every app has its own.

use std::collections::{BTreeMap, BTreeSet};

use postio_ui::observe::Observation;
use serde::{Deserialize, Serialize};

use crate::apply::App;
use crate::format::Storyboard;
use crate::run::{Run, StepOutcome, variant_key};

/// One step across the apps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityRow {
    /// The step, 0 being the starting state.
    pub step: usize,
    /// What each app observed after it.
    pub observations: BTreeMap<App, Observation>,
    /// Apps for which this step does not exist (an override's `skip`).
    pub skipped: Vec<App>,
    /// The apps disagree and no override explains it.
    pub diverging: bool,
}

/// One shared storyboard in one variant, across the apps that played it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parity {
    /// The storyboard.
    pub storyboard: String,
    /// The variant key, `default` when nothing varies.
    pub variant: String,
    /// The apps, in order.
    pub apps: Vec<App>,
    /// Step by step.
    pub rows: Vec<ParityRow>,
}

impl Parity {
    /// The steps that diverge.
    pub fn diverging(&self) -> impl Iterator<Item = &ParityRow> {
        self.rows.iter().filter(|row| row.diverging)
    }
}

/// Lines up `runs` by storyboard and variant, and marks divergence. Only
/// storyboards played by two or more apps have parity to show.
pub fn parity(runs: &[Run], storyboards: &BTreeMap<String, Storyboard>) -> Vec<Parity> {
    let mut groups: BTreeMap<(String, String), Vec<&Run>> = BTreeMap::new();
    for run in runs {
        groups
            .entry((run.storyboard.name.clone(), variant_key(&run.variant)))
            .or_default()
            .push(run);
    }
    let mut all = Vec::new();
    for ((name, variant), mut group) in groups {
        group.sort_by_key(|run| run.app);
        group.dedup_by_key(|run| run.app);
        if group.len() < 2 {
            continue;
        }
        let board = storyboards.get(&name);
        let steps = group.iter().map(|run| run.steps.len()).max().unwrap_or(0);
        let rows = (0..steps)
            .map(|n| {
                let mut observations = BTreeMap::new();
                let mut skipped = Vec::new();
                for run in &group {
                    let Some(step) = run.steps.get(n) else {
                        continue;
                    };
                    if matches!(step.outcome, StepOutcome::Skipped { .. }) {
                        skipped.push(run.app);
                    } else {
                        observations.insert(run.app, step.observation.clone());
                    }
                }
                let differ = observations
                    .values()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|pair| !pair[0].shared_eq(pair[1]));
                let explained = board.is_some_and(|board| overridden(board, n));
                ParityRow {
                    step: n,
                    observations,
                    skipped,
                    diverging: differ && !explained,
                }
            })
            .collect();
        all.push(Parity {
            storyboard: name,
            variant,
            apps: group.iter().map(|run| run.app).collect(),
            rows,
        });
    }
    all
}

/// Whether any app overrides step `n` (1-based; 0, the starting state, has
/// no step to override), by its id or its index.
fn overridden(board: &Storyboard, n: usize) -> bool {
    let Some(step) = n.checked_sub(1).and_then(|i| board.steps.get(i)) else {
        return false;
    };
    let keys: BTreeSet<String> = step
        .id
        .iter()
        .cloned()
        .chain(std::iter::once(n.to_string()))
        .collect();
    board
        .overrides
        .values()
        .any(|steps| steps.keys().any(|key| keys.contains(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::run;
    use crate::format::parse;
    use std::path::Path;

    fn board(text: &str) -> BTreeMap<String, Storyboard> {
        let b = parse(text, Path::new("list/walk.toml")).expect("loads");
        [(b.name.clone(), b)].into_iter().collect()
    }

    const PLAIN: &str = "source = { kind = \"flow\", ref = \"x\" }\n[[step]]\ncommand = \"next_message\"\n[[step]]\ncommand = \"archive\"\n";

    #[test]
    fn agreeing_apps_do_not_diverge() {
        let runs = [
            run("walk", App::Classic, &[], 3),
            run("walk", App::Focus, &[], 3),
        ];
        let all = parity(&runs, &board(PLAIN));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].apps, [App::Classic, App::Focus]);
        assert_eq!(all[0].diverging().count(), 0);
        assert_eq!(all[0].rows.len(), 3);
    }

    #[test]
    fn a_different_cursor_diverges_on_that_step() {
        let classic = run("walk", App::Classic, &[], 3);
        let mut focus = run("walk", App::Focus, &[], 3);
        focus.steps[2].observation.cursor.index = None;
        let all = parity(&[classic, focus], &board(PLAIN));
        let diverging: Vec<usize> = all[0].diverging().map(|row| row.step).collect();
        assert_eq!(diverging, [2]);
    }

    #[test]
    fn internals_do_not_diverge() {
        let classic = run("walk", App::Classic, &[], 2);
        let mut focus = run("walk", App::Focus, &[], 2);
        focus.steps[1].observation.keyboard.widget = "FocusWindow/List".into();
        focus.steps[1]
            .observation
            .app
            .insert("focus.bulk".into(), serde_json::json!(false));
        assert_eq!(
            parity(&[classic, focus], &board(PLAIN))[0]
                .diverging()
                .count(),
            0
        );
    }

    #[test]
    fn an_override_explains_a_difference() {
        let text = format!("{PLAIN}[app.focus.step.2]\nexpect = \"Focus differs here\"\n");
        let classic = run("walk", App::Classic, &[], 3);
        let mut focus = run("walk", App::Focus, &[], 3);
        focus.steps[2].observation.cursor.index = None;
        assert_eq!(
            parity(&[classic, focus], &board(&text))[0]
                .diverging()
                .count(),
            0
        );
    }

    #[test]
    fn a_skipped_step_is_shown_as_skipped_not_diverging() {
        let text = format!("{PLAIN}[app.focus.step.1]\nskip = {{ reason = \"no such key\" }}\n");
        let classic = run("walk", App::Classic, &[], 3);
        let mut focus = run("walk", App::Focus, &[], 3);
        focus.steps[1].outcome = StepOutcome::Skipped {
            reason: "no such key".into(),
        };
        focus.steps[1].observation.cursor.index = Some(9);
        let all = parity(&[classic, focus], &board(&text));
        assert_eq!(all[0].rows[1].skipped, [App::Focus]);
        assert!(!all[0].rows[1].diverging);
    }

    #[test]
    fn variants_are_grouped_and_lone_apps_have_no_parity() {
        let runs = [
            run("walk", App::Classic, &[("scheme", "dark")], 2),
            run("walk", App::Focus, &[("scheme", "dark")], 2),
            run("walk", App::Classic, &[], 2),
        ];
        let all = parity(&runs, &board(PLAIN));
        assert_eq!(all.len(), 1, "the default variant has one app only");
        assert_eq!(all[0].variant, "scheme=dark");
    }
}
