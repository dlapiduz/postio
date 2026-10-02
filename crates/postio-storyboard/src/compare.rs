//! Base versus branch: which storyboards a branch changed, and how
//! (data-model § Comparison).
//!
//! The maintainer's attention is the scarce thing, so the page shows a
//! storyboard in full only when the branch changed what it does. "Changed"
//! is about *behaviour*: the branch's storyboards are played against the
//! base's code (research R8), and two runs are compared step by step on the
//! plain frame's hash and on the shared fields of the observation.
//!
//! Two things are deliberately not a change:
//!
//! * **The outline.** It is drawn by the runner over the frame a person
//!   would see, and it moves with focus; comparing outlined frames would make
//!   every focus change read twice. The plain frame's hash is compared.
//! * **The widget path and `app.*`.** They name one toolkit's internals and
//!   move with refactors that change nothing a person sees.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bundle::{Class, Classification};
use crate::page::{self, Filmstrip};
use crate::run::{Run, Status};

/// What happened to one storyboard, on one app, in one variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Same frames, same observations.
    Unchanged,
    /// The base played it and the branch differs.
    Changed,
    /// Only the branch has it.
    New,
    /// Only the base has it.
    Removed,
    /// The base could not play it (an older runner, a step it lacks).
    BaseUnavailable,
}

/// How one step differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepDiff {
    /// The step, 0 being the starting state.
    pub step: usize,
    /// The plain frame's hash differs.
    pub frame_changed: bool,
    /// The shared observation fields that differ, as dotted paths.
    pub observation_changed: Vec<String>,
    /// The step's outcome or checks came out differently.
    pub status_changed: bool,
}

impl StepDiff {
    fn any(&self) -> bool {
        self.frame_changed || !self.observation_changed.is_empty() || self.status_changed
    }
}

/// One storyboard's comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    /// The run directory, `<app>/<storyboard>/<variant>`, which is how base
    /// and branch runs are matched.
    pub run: String,
    /// What happened to it.
    pub kind: Kind,
    /// The steps that differ (empty unless `Changed`).
    pub steps: Vec<StepDiff>,
    /// The base and branch were built from different seeds.
    pub seed_changed: bool,
}

/// Compares one storyboard's base and branch runs.
pub fn compare(run: &str, base: Option<&Run>, branch: Option<&Run>) -> Comparison {
    let mut comparison = Comparison {
        run: run.to_owned(),
        kind: Kind::Unchanged,
        steps: vec![],
        seed_changed: false,
    };
    let (base, branch) = match (base, branch) {
        (None, None) => return comparison,
        (None, Some(_)) => {
            comparison.kind = Kind::New;
            return comparison;
        }
        (Some(_), None) => {
            comparison.kind = Kind::Removed;
            return comparison;
        }
        (Some(base), Some(branch)) => (base, branch),
    };
    if matches!(
        base.status,
        Status::Unavailable { .. } | Status::Error { .. }
    ) {
        comparison.kind = Kind::BaseUnavailable;
        return comparison;
    }
    comparison.seed_changed = base.seed != branch.seed;
    let steps = base.steps.len().max(branch.steps.len());
    for n in 0..steps {
        let diff = match (base.steps.get(n), branch.steps.get(n)) {
            (Some(b), Some(r)) => StepDiff {
                step: r.step,
                frame_changed: b.frame.as_ref().map(|f| &f.hash)
                    != r.frame.as_ref().map(|f| &f.hash),
                observation_changed: observation_diff(&b.observation, &r.observation),
                status_changed: b.outcome != r.outcome || b.checks != r.checks,
            },
            (_, Some(r)) => StepDiff {
                step: r.step,
                frame_changed: true,
                observation_changed: vec![],
                status_changed: true,
            },
            (Some(b), None) => StepDiff {
                step: b.step,
                frame_changed: true,
                observation_changed: vec![],
                status_changed: true,
            },
            (None, None) => continue,
        };
        if diff.any() {
            comparison.steps.push(diff);
        }
    }
    if !comparison.steps.is_empty() {
        comparison.kind = Kind::Changed;
    }
    comparison
}

/// The shared observation fields that differ, as dotted paths. The widget
/// path and `app.*` are left out (module docs).
fn observation_diff(
    base: &postio_ui::observe::Observation,
    branch: &postio_ui::observe::Observation,
) -> Vec<String> {
    let flat = |observation: &postio_ui::observe::Observation| {
        let mut rows = BTreeMap::new();
        if let Ok(value) = serde_json::to_value(observation) {
            flatten("", &value, &mut rows);
        }
        rows.retain(|path: &String, _| path != "keyboard.widget" && !path.starts_with("app."));
        rows
    };
    let (a, b) = (flat(base), flat(branch));
    let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    paths
        .into_iter()
        .filter(|path| a.get(*path) != b.get(*path))
        .cloned()
        .collect()
}

fn flatten(
    prefix: &str,
    value: &serde_json::Value,
    rows: &mut BTreeMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                // `app` is a map of dotted names; keep its keys whole.
                if path == "app" {
                    for (name, v) in inner.as_object().into_iter().flatten() {
                        rows.insert(format!("app.{name}"), v.clone());
                    }
                    continue;
                }
                flatten(&path, inner, rows);
            }
        }
        other => {
            rows.insert(prefix.to_owned(), other.clone());
        }
    }
}

/// Compares every run under two trees, matched by directory.
pub fn compare_trees(base: &Path, branch: &Path) -> io::Result<Vec<Comparison>> {
    let index = |root: &Path| -> io::Result<BTreeMap<String, Run>> {
        Ok(page::collect(root, "")?
            .into_iter()
            .map(|strip| (strip.dir, strip.run))
            .collect())
    };
    let (base, branch) = (index(base)?, index(branch)?);
    let names: BTreeSet<&String> = base.keys().chain(branch.keys()).collect();
    Ok(names
        .into_iter()
        .map(|name| compare(name, base.get(name), branch.get(name)))
        .collect())
}

/// A bundle classifier from comparisons: what the reviewer is asked about.
pub fn classifier(comparisons: &[Comparison]) -> impl Fn(&Filmstrip) -> Classification + '_ {
    move |strip: &Filmstrip| {
        let relative = strip.dir.trim_start_matches("runs/");
        match comparisons.iter().find(|c| c.run == relative) {
            Some(c) if c.kind == Kind::Unchanged => Classification {
                class: Class::Unchanged,
                changed_steps: Some(BTreeSet::new()),
            },
            Some(c) if c.kind == Kind::Changed => Classification {
                class: Class::Changed,
                changed_steps: Some(c.steps.iter().map(|s| s.step).collect()),
            },
            _ => Classification {
                class: Class::New,
                changed_steps: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::App;
    use crate::fixtures::{run, write};
    use crate::run::Frame;

    fn framed(mut r: Run) -> Run {
        for step in &mut r.steps {
            step.frame = Some(Frame {
                path: format!("{:02}.png", step.step),
                hash: format!("hash-{}", step.step),
            });
        }
        r
    }

    #[test]
    fn identical_runs_are_unchanged() {
        let a = framed(run("walk", App::Classic, &[], 3));
        let c = compare("classic/walk/default", Some(&a), Some(&a.clone()));
        assert_eq!(c.kind, Kind::Unchanged);
        assert!(c.steps.is_empty());
    }

    #[test]
    fn a_different_frame_hash_is_a_change_on_that_step() {
        let base = framed(run("walk", App::Classic, &[], 3));
        let mut branch = base.clone();
        branch.steps[2].frame.as_mut().unwrap().hash = "other".into();
        let c = compare("classic/walk/default", Some(&base), Some(&branch));
        assert_eq!(c.kind, Kind::Changed);
        assert_eq!(c.steps.len(), 1);
        assert_eq!(c.steps[0].step, 2);
        assert!(c.steps[0].frame_changed);
    }

    #[test]
    fn the_outline_alone_is_not_a_change() {
        let base = framed(run("walk", App::Classic, &[], 2));
        let mut branch = base.clone();
        branch.steps[1].outlined = Some("elsewhere.png".into());
        let c = compare("classic/walk/default", Some(&base), Some(&branch));
        assert_eq!(c.kind, Kind::Unchanged);
    }

    #[test]
    fn observation_changes_are_named_and_internals_are_not() {
        let base = framed(run("walk", App::Classic, &[], 2));
        let mut branch = base.clone();
        branch.steps[1].observation.cursor.index = Some(4);
        branch.steps[1].observation.keyboard.widget = "Other/Path".into();
        branch.steps[1]
            .observation
            .app
            .insert("classic.pane".into(), serde_json::json!("reader"));
        let c = compare("classic/walk/default", Some(&base), Some(&branch));
        assert_eq!(c.kind, Kind::Changed);
        assert_eq!(c.steps[0].observation_changed, ["cursor.index"]);
    }

    #[test]
    fn a_different_outcome_is_a_change() {
        let base = framed(run("walk", App::Classic, &[], 2));
        let mut branch = base.clone();
        branch.steps[1].outcome = crate::run::StepOutcome::Dropped;
        let c = compare("classic/walk/default", Some(&base), Some(&branch));
        assert!(c.steps[0].status_changed);
    }

    #[test]
    fn new_removed_and_unavailable_runs_are_classed() {
        let a = framed(run("walk", App::Classic, &[], 2));
        assert_eq!(compare("x", None, Some(&a)).kind, Kind::New);
        assert_eq!(compare("x", Some(&a), None).kind, Kind::Removed);
        let mut gone = a.clone();
        gone.status = Status::Unavailable {
            reason: "the base runner lacks the step".into(),
        };
        gone.steps.clear();
        assert_eq!(
            compare("x", Some(&gone), Some(&a)).kind,
            Kind::BaseUnavailable
        );
    }

    #[test]
    fn extra_steps_on_the_branch_are_changes() {
        let base = framed(run("walk", App::Classic, &[], 2));
        let branch = framed(run("walk", App::Classic, &[], 3));
        let c = compare("x", Some(&base), Some(&branch));
        assert_eq!(c.kind, Kind::Changed);
        assert_eq!(c.steps.iter().map(|s| s.step).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn a_seed_change_is_said_once() {
        let base = framed(run("walk", App::Classic, &[], 2));
        let mut branch = base.clone();
        branch.seed = "thirty-threads".into();
        assert!(compare("x", Some(&base), Some(&branch)).seed_changed);
    }

    #[test]
    fn trees_are_matched_by_run_directory() {
        let base = tempfile::tempdir().expect("temp");
        let branch = tempfile::tempdir().expect("temp");
        let same = framed(run("same", App::Classic, &[], 2));
        write(base.path(), &same);
        write(branch.path(), &same);
        write(branch.path(), &framed(run("fresh", App::Classic, &[], 2)));
        write(base.path(), &framed(run("gone", App::Classic, &[], 2)));
        let all = compare_trees(base.path(), branch.path()).expect("compared");
        let kinds: BTreeMap<_, _> = all.iter().map(|c| (c.run.as_str(), c.kind)).collect();
        assert_eq!(kinds["classic/same/default"], Kind::Unchanged);
        assert_eq!(kinds["classic/fresh/default"], Kind::New);
        assert_eq!(kinds["classic/gone/default"], Kind::Removed);
    }
}
