//! One storyboard played on one app in one variant, written as `run.json` beside its frames. The
//! rules for what a run came to live here, so an unbound step, a dropped key or a blank frame fails
//! a run the same way whichever app was played.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use postio_ui::observe::Observation;
use serde::{Deserialize, Serialize};

use crate::apply::App;
use crate::check::{CheckResult, Outcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Direct,
    Chain,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Played {
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Status {
    Passed,
    Failed,
    NotApplicable { reason: String },
    NotCovered { reason: String },
    Unavailable { reason: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum StepOutcome {
    Delivered,
    Unbound { context: String },
    Dropped,
    NothingToTypeInto,
    Skipped { reason: String },
    NotCovered { delivery: Delivery },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "settle")]
pub enum Settle {
    Settled { ms: u64 },
    Jumped { frames: Vec<String> },
    Blanked { frames: Vec<String> },
    Unsettled { ms: u64 },
    NotSampled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub path: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivered {
    pub step: String,
    pub chord: Option<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRun {
    pub step: usize,
    pub id: Option<String>,
    pub input: Option<Delivered>,
    #[serde(default)]
    pub expect: Option<String>,
    pub outcome: StepOutcome,
    pub observation: Observation,
    pub checks: Vec<CheckResult>,
    pub settle: Settle,
    pub frame: Option<Frame>,
    pub outlined: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub storyboard: Played,
    pub app: App,
    pub variant: BTreeMap<String, String>,
    pub ignored_axes: Vec<String>,
    pub tree_key: String,
    pub commit: String,
    pub delivery: Delivery,
    pub renderer: String,
    pub stride: u32,
    pub seed: String,
    pub preset: Option<String>,
    pub steps: Vec<StepRun>,
    pub status: Status,
}

/// A run fails when any check failed, any step was unbound, dropped or had nowhere to type, or any
/// frame went blank. Jumps and unsettled steps are reported but do not fail it, because a late load
/// can be legitimate. A step not covered by its delivery mode makes the run not covered, never
/// passed, unless something already failed it.
pub fn status(steps: &[StepRun]) -> Status {
    let failed = steps.iter().any(|step| {
        matches!(
            step.outcome,
            StepOutcome::Unbound { .. } | StepOutcome::Dropped | StepOutcome::NothingToTypeInto
        ) || matches!(step.settle, Settle::Blanked { .. })
            || step
                .checks
                .iter()
                .any(|check| check.outcome == Outcome::Fail)
    });
    if failed {
        return Status::Failed;
    }
    let uncovered: Vec<String> = steps
        .iter()
        .filter_map(|step| match &step.outcome {
            StepOutcome::NotCovered { delivery } => Some(format!(
                "step {} needs real input; this run delivered by {}",
                step.step,
                match delivery {
                    Delivery::Direct => "direct dispatch",
                    Delivery::Chain => "the focus chain",
                    Delivery::Real => "real input",
                }
            )),
            _ => None,
        })
        .collect();
    if uncovered.is_empty() {
        Status::Passed
    } else {
        Status::NotCovered {
            reason: uncovered.join("; "),
        }
    }
}

/// Stable, because base and branch runs are matched by it.
pub fn variant_key(variant: &BTreeMap<String, String>) -> String {
    if variant.is_empty() {
        return "default".to_owned();
    }
    variant
        .iter()
        .map(|(axis, value)| format!("{axis}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Every name it hands out is relative to `run.json`, so a tree can be moved or cached whole.
pub struct RunWriter {
    dir: PathBuf,
}

impl RunWriter {
    pub fn new(
        out: &Path,
        app: App,
        storyboard: &str,
        variant: &BTreeMap<String, String>,
    ) -> io::Result<Self> {
        let app = serde_json::to_value(app)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{app:?}").to_lowercase());
        let dir = out.join(app).join(storyboard).join(variant_key(variant));
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn frame(step: usize) -> String {
        format!("{step:02}.png")
    }

    pub fn outlined(step: usize) -> String {
        format!("{step:02}.outlined.png")
    }

    pub fn extra(step: usize, n: usize) -> String {
        format!("{step:02}.s{n}.png")
    }

    pub fn write(&self, run: &Run) -> io::Result<PathBuf> {
        let path = self.dir.join("run.json");
        let json = serde_json::to_string_pretty(run).map_err(io::Error::other)?;
        std::fs::write(&path, json + "\n")?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation() -> Observation {
        serde_json::from_value(json!({
            "window": "open",
            "view": "list",
            "scope": "Inbox",
            "keyboard": { "region": "list", "field": null, "typing": false, "reachable": true, "widget": "PostioWindow/ListView" },
            "cursor": { "index": 0, "id": "1", "subject": "Hello" },
            "rows": { "first_visible": 0, "count": 11 },
            "selection": { "count": 0 },
            "overlay": { "kind": "none", "mode": null },
            "notice": { "text": null, "tone": null, "undo": false },
            "banner": { "title": null },
            "reading": { "id": null, "focused": null, "scroll": null },
            "composer": { "open": false, "detached": false },
            "back_depth": null
        }))
        .expect("the fixture matches the observation's shape")
    }

    fn step(n: usize, outcome: StepOutcome, settle: Settle, check: Option<Outcome>) -> StepRun {
        StepRun {
            step: n,
            id: None,
            input: None,
            expect: None,
            outcome,
            observation: observation(),
            checks: check
                .into_iter()
                .map(|outcome| CheckResult {
                    path: "cursor.index".into(),
                    expected: "= 1".into(),
                    observed: Some(json!(1)),
                    outcome,
                })
                .collect(),
            settle,
            frame: None,
            outlined: None,
        }
    }

    fn settled() -> Settle {
        Settle::Settled { ms: 120 }
    }

    #[test]
    fn delivered_steps_with_passing_checks_pass() {
        let steps = [
            step(0, StepOutcome::Delivered, settled(), None),
            step(1, StepOutcome::Delivered, settled(), Some(Outcome::Pass)),
        ];
        assert_eq!(status(&steps), Status::Passed);
    }

    #[test]
    fn a_failed_check_fails_the_run() {
        let steps = [step(
            1,
            StepOutcome::Delivered,
            settled(),
            Some(Outcome::Fail),
        )];
        assert_eq!(status(&steps), Status::Failed);
    }

    #[test]
    fn not_applicable_checks_do_not_fail_the_run() {
        let steps = [step(
            1,
            StepOutcome::Delivered,
            settled(),
            Some(Outcome::NotApplicable),
        )];
        assert_eq!(status(&steps), Status::Passed);
    }

    #[test]
    fn an_undeliverable_step_fails_the_run() {
        for outcome in [
            StepOutcome::Unbound {
                context: "reader".into(),
            },
            StepOutcome::Dropped,
            StepOutcome::NothingToTypeInto,
        ] {
            let steps = [step(1, outcome.clone(), settled(), None)];
            assert_eq!(status(&steps), Status::Failed, "{outcome:?}");
        }
    }

    #[test]
    fn a_blank_frame_fails_but_a_jump_or_a_long_wait_does_not() {
        let blank = [step(
            1,
            StepOutcome::Delivered,
            Settle::Blanked {
                frames: vec!["01.s1.png".into()],
            },
            None,
        )];
        assert_eq!(status(&blank), Status::Failed);
        let jumped = [step(
            1,
            StepOutcome::Delivered,
            Settle::Jumped {
                frames: vec!["01.s1.png".into(), "01.s2.png".into()],
            },
            None,
        )];
        assert_eq!(status(&jumped), Status::Passed);
        let slow = [step(
            1,
            StepOutcome::Delivered,
            Settle::Unsettled { ms: 3000 },
            None,
        )];
        assert_eq!(status(&slow), Status::Passed);
    }

    #[test]
    fn a_step_not_covered_is_never_a_pass() {
        let steps = [
            step(1, StepOutcome::Delivered, settled(), Some(Outcome::Pass)),
            step(
                2,
                StepOutcome::NotCovered {
                    delivery: Delivery::Chain,
                },
                settled(),
                None,
            ),
        ];
        match status(&steps) {
            Status::NotCovered { reason } => assert!(reason.contains("step 2"), "{reason}"),
            other => panic!("expected not covered, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_outranks_not_covered() {
        let steps = [
            step(1, StepOutcome::Delivered, settled(), Some(Outcome::Fail)),
            step(
                2,
                StepOutcome::NotCovered {
                    delivery: Delivery::Chain,
                },
                settled(),
                None,
            ),
        ];
        assert_eq!(status(&steps), Status::Failed);
    }

    #[test]
    fn variant_keys_are_stable_and_default_when_empty() {
        assert_eq!(variant_key(&BTreeMap::new()), "default");
        let variant: BTreeMap<String, String> = [("width", "narrow"), ("scheme", "dark")]
            .into_iter()
            .map(|(a, v)| (a.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(variant_key(&variant), "scheme=dark,width=narrow");
    }

    #[test]
    fn the_writer_lays_out_the_tree_and_names_frames_relatively() {
        let out = tempfile::tempdir().expect("a temp dir");
        let writer = RunWriter::new(
            out.path(),
            App::Terminal,
            "archive-walks-down",
            &BTreeMap::new(),
        )
        .expect("a writer");
        assert_eq!(
            writer.dir(),
            out.path().join("terminal/archive-walks-down/default")
        );
        assert_eq!(RunWriter::frame(1), "01.png");
        assert_eq!(RunWriter::outlined(0), "00.outlined.png");
        assert_eq!(RunWriter::extra(3, 2), "03.s2.png");
        let run = Run {
            storyboard: Played {
                name: "archive-walks-down".into(),
                hash: "abc".into(),
            },
            app: App::Terminal,
            variant: BTreeMap::new(),
            ignored_axes: vec![],
            tree_key: "k".into(),
            commit: "c".into(),
            delivery: Delivery::Chain,
            renderer: "cairo".into(),
            stride: 2,
            seed: "small".into(),
            preset: None,
            steps: vec![StepRun {
                frame: Some(Frame {
                    path: RunWriter::frame(0),
                    hash: "h".into(),
                }),
                ..step(0, StepOutcome::Delivered, settled(), None)
            }],
            status: Status::Passed,
        };
        let written = writer.write(&run).expect("written");
        let back: Run = serde_json::from_str(&std::fs::read_to_string(written).expect("readable"))
            .expect("it reads back");
        assert_eq!(back, run);
        assert!(!back.steps[0].frame.as_ref().unwrap().path.contains('/'));
    }
}
