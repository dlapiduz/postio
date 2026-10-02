//! One storyboard, played on one app in one variant: what happened at every
//! step, written as `run.json` beside its frames (data-model § Run).
//!
//! The runner fills these in; the rules for what a run *came to* live here,
//! so a step that was unbound, a key that was dropped on the way to the
//! window, or a frame that went blank fails the run the same way whichever
//! app was played.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use postio_ui::observe::Observation;
use serde::{Deserialize, Serialize};

use crate::apply::App;
use crate::check::{CheckResult, Outcome};

/// How input reached the app (research R3). Every run says which, because
/// what a run can see depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Straight to the window's key handler.
    Direct,
    /// Along the real focus chain, through every key controller on it.
    Chain,
    /// Through the compositor. Not built yet.
    Real,
}

/// The storyboard a run played, pinned to the file's contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Played {
    /// Its name.
    pub name: String,
    /// blake3 of the file, so a cached run is never reused for an edited one.
    pub hash: String,
}

/// What a run came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Status {
    /// Every check passed and every step was delivered.
    Passed,
    /// A check failed, a step could not be delivered, or a frame went blank.
    Failed,
    /// The storyboard does not apply to this app.
    NotApplicable {
        /// Why.
        reason: String,
    },
    /// Some step needs a delivery mode this run did not have. Never a pass.
    NotCovered {
        /// Which steps, and what they needed.
        reason: String,
    },
    /// The runner could not play it, as when a base predates a step kind.
    Unavailable {
        /// Why.
        reason: String,
    },
    /// The runner failed.
    Error {
        /// What it said.
        message: String,
    },
}

/// What happened to one step's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum StepOutcome {
    /// It was delivered.
    Delivered,
    /// The command has no binding in the context the app was in.
    Unbound {
        /// The key context, by name.
        context: String,
    },
    /// The key went to a widget that is gone, or to nothing.
    Dropped,
    /// There was text to type and nowhere to type it.
    NothingToTypeInto,
    /// An override says this step does not exist in this app.
    Skipped {
        /// Why.
        reason: String,
    },
    /// The step needs a delivery mode this run did not have.
    NotCovered {
        /// The mode it ran under.
        delivery: Delivery,
    },
}

/// What the screen did after a step (research R4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "settle")]
pub enum Settle {
    /// It stopped changing.
    Settled {
        /// After how long.
        ms: u64,
    },
    /// It settled and then changed again. The frames before and after.
    Jumped {
        /// Paths relative to `run.json`.
        frames: Vec<String>,
    },
    /// A sampled frame was empty or a single colour.
    Blanked {
        /// Paths relative to `run.json`.
        frames: Vec<String>,
    },
    /// It never stopped changing.
    Unsettled {
        /// How long the runner waited.
        ms: u64,
    },
    /// No frames were taken (`--no-frames`).
    NotSampled,
}

/// A frame on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// Relative to `run.json`.
    pub path: String,
    /// blake3 of the pixels, which is what a comparison compares.
    pub hash: String,
}

/// What was delivered, so a reader can tell which chord a command became.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivered {
    /// The step as written: `command archive`, `key mod+z`, `type "inv"`.
    pub step: String,
    /// The chord pressed, if a key was.
    pub chord: Option<String>,
    /// The key context it was resolved in.
    pub context: Option<String>,
}

/// One step of a run. Step 0 is the starting state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRun {
    /// 1-based, as storyboards number steps; 0 is the starting state.
    pub step: usize,
    /// The step's id, when it has one.
    pub id: Option<String>,
    /// What was delivered. `None` at the starting state.
    pub input: Option<Delivered>,
    /// The step's prose expectation, for this app (an override's if it has
    /// one), carried so a run can be read and reviewed on its own.
    #[serde(default)]
    pub expect: Option<String>,
    /// What happened to it.
    pub outcome: StepOutcome,
    /// Where everything was afterwards.
    pub observation: Observation,
    /// The step's checks.
    pub checks: Vec<CheckResult>,
    /// What the screen did.
    pub settle: Settle,
    /// The plain frame, which comparisons hash.
    pub frame: Option<Frame>,
    /// The frame with the keyboard's region outlined, which people look at.
    pub outlined: Option<String>,
}

/// One storyboard on one app in one variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// What was played.
    pub storyboard: Played,
    /// Where.
    pub app: App,
    /// The variant, axis by axis.
    pub variant: BTreeMap<String, String>,
    /// Axes the storyboard asked for that this app does not have.
    pub ignored_axes: Vec<String>,
    /// The review key of the tree it ran on (research R13).
    pub tree_key: String,
    /// The commit, for information only; a rebase changes it.
    pub commit: String,
    /// How input reached the app.
    pub delivery: Delivery,
    /// The GSK renderer the frames came from (research R5).
    pub renderer: String,
    /// Frames sampled every this many ticks (research R4).
    pub stride: u32,
    /// The seed the store was built from.
    pub seed: String,
    /// The window condition it started in, if any.
    pub preset: Option<String>,
    /// Every step, starting state first.
    pub steps: Vec<StepRun>,
    /// What it came to; see [`status`].
    pub status: Status,
}

/// What a run's steps add up to (data-model § Run).
///
/// A run **fails** when any check failed, any step was unbound, dropped or
/// had nowhere to type, or any frame went blank. Jumps and unsettled steps
/// are reported but do not fail it, because a late load can be legitimate;
/// the reviewer judges those. A run with any step not covered by its
/// delivery mode is **not covered**, never passed, unless something already
/// failed it.
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

/// The directory name for a variant: `scheme=dark,width=narrow`, or
/// `default` when nothing varies. Stable, because base and branch runs are
/// matched by it.
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

/// Where one run's files go: `<out>/<app>/<storyboard>/<variant>/`
/// (contracts/runner.md § The output tree). Every name it hands out is
/// relative to `run.json`, so a tree can be moved or cached whole.
pub struct RunWriter {
    dir: PathBuf,
}

impl RunWriter {
    /// The writer for one run, creating its directory.
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

    /// The run's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The plain frame's name for a step.
    pub fn frame(step: usize) -> String {
        format!("{step:02}.png")
    }

    /// The outlined frame's name for a step.
    pub fn outlined(step: usize) -> String {
        format!("{step:02}.outlined.png")
    }

    /// The name of the `n`th extra frame a step kept (a jump or a blank).
    pub fn extra(step: usize, n: usize) -> String {
        format!("{step:02}.s{n}.png")
    }

    /// Writes `run.json`.
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
            App::Classic,
            "archive-walks-down",
            &BTreeMap::new(),
        )
        .expect("a writer");
        assert_eq!(
            writer.dir(),
            out.path().join("classic/archive-walks-down/default")
        );
        assert_eq!(RunWriter::frame(1), "01.png");
        assert_eq!(RunWriter::outlined(0), "00.outlined.png");
        assert_eq!(RunWriter::extra(3, 2), "03.s2.png");
        let run = Run {
            storyboard: Played {
                name: "archive-walks-down".into(),
                hash: "abc".into(),
            },
            app: App::Classic,
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
