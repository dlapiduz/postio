//! The reviewer's answer, and the validator that decides whether it is one
//! (`contracts/review.md` § `verdicts.json`, FR-019).
//!
//! A review that cannot be traced back to a frame is worth nothing to the
//! person who has to act on it, so the validator is strict about citation
//! and says which rule each rejection broke.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bundle::Manifest;

/// What a reviewer says about one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The frame meets the expectation.
    Pass,
    /// It does not.
    Fail,
    /// The reviewer cannot tell; the maintainer is asked.
    Question,
}

/// How much a failure or finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The branch cannot land with it.
    Blocker,
    /// A person would be misled or slowed.
    Wrong,
    /// Worth listing; does not hold the branch.
    Polish,
}

/// What the implementing session did about it (FR-020).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Nothing yet.
    #[default]
    Open,
    /// Fixed, and this re-run passed.
    Fixed {
        /// The run that proves it.
        rerun: String,
    },
    /// The implementer says the reviewer is wrong; the maintainer decides.
    Contested {
        /// Why.
        reason: String,
    },
}

/// A verdict on one step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// The storyboard cited.
    #[serde(default)]
    pub storyboard: String,
    /// Its step, by id or index.
    #[serde(default)]
    pub step: String,
    /// The app.
    #[serde(default)]
    pub app: String,
    /// The variant directory name.
    #[serde(default)]
    pub variant: String,
    /// The outlined frame, relative to the bundle.
    #[serde(default)]
    pub frame: String,
    /// Pass, fail or question.
    pub verdict: Kind,
    /// Required on a fail.
    #[serde(default)]
    pub severity: Option<Severity>,
    /// What a person would notice.
    #[serde(default)]
    pub says: String,
    /// The rule it rests on.
    #[serde(default)]
    pub rule: String,
    /// Set by the implementing session.
    #[serde(default)]
    pub resolution: Resolution,
}

/// Something no expectation covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The storyboard cited.
    #[serde(default)]
    pub storyboard: String,
    /// Its step, by id or index.
    #[serde(default)]
    pub step: String,
    /// The app.
    #[serde(default)]
    pub app: String,
    /// The variant directory name.
    #[serde(default)]
    pub variant: String,
    /// The outlined frame, relative to the bundle.
    #[serde(default)]
    pub frame: String,
    /// Required.
    #[serde(default)]
    pub severity: Option<Severity>,
    /// What a person would notice.
    #[serde(default)]
    pub says: String,
    /// The rule it rests on.
    #[serde(default)]
    pub rule: String,
    /// Set by the implementing session.
    #[serde(default)]
    pub resolution: Resolution,
}

/// The bundle a review answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleId {
    /// The review key.
    #[serde(default)]
    pub tree_key: String,
    /// The base.
    #[serde(default)]
    pub base: Option<String>,
}

/// Who reviewed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    /// The agent definition.
    #[serde(default)]
    pub agent: String,
    /// The model.
    #[serde(default)]
    pub model: String,
    /// The prompt template's blake3.
    #[serde(default)]
    pub template: String,
}

/// `verdicts.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// The bundle it answers.
    #[serde(default)]
    pub bundle: BundleId,
    /// The reviewer.
    #[serde(default)]
    pub reviewer: Reviewer,
    /// One per step that needed one.
    #[serde(default)]
    pub verdicts: Vec<Verdict>,
    /// What no expectation covered.
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// An entry in `contests.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Contest {
    /// `<storyboard>/<step>/<app>/<variant>`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Why the reviewer is wrong.
    pub reason: String,
}

/// Why a review is rejected. Each is a rule of FR-019.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    /// `verdicts.json` is missing or is not the schema.
    #[error("verdicts.json: {0}")]
    Unreadable(String),
    /// A citation field is empty or absent.
    #[error("{what} {index}: missing citation field `{field}`")]
    MissingCitation {
        /// `verdict` or `finding`.
        what: &'static str,
        /// Its position, from 1.
        index: usize,
        /// The field.
        field: &'static str,
    },
    /// The cited frame is not in the bundle.
    #[error("{what} {index}: frame `{frame}` is not in the bundle")]
    FrameAbsent {
        /// `verdict` or `finding`.
        what: &'static str,
        /// Its position, from 1.
        index: usize,
        /// The path cited.
        frame: String,
    },
    /// A step the manifest lists has no verdict.
    #[error("no verdict for {0}")]
    StepWithoutVerdict(String),
    /// A fail, or a finding, with no severity.
    #[error("{what} {index}: a {kind} needs a severity (blocker, wrong or polish)")]
    MissingSeverity {
        /// `verdict` or `finding`.
        what: &'static str,
        /// Its position, from 1.
        index: usize,
        /// `fail` or `finding`.
        kind: &'static str,
    },
    /// `says` is empty.
    #[error("{what} {index}: `says` is empty")]
    EmptySays {
        /// `verdict` or `finding`.
        what: &'static str,
        /// Its position, from 1.
        index: usize,
    },
    /// A contest names nothing in the review.
    #[error("contests.toml: `{0}` matches no verdict or finding")]
    ContestWithoutTarget(String),
}

/// The reference a contest uses for a citation.
pub fn reference(storyboard: &str, step: &str, app: &str, variant: &str) -> String {
    format!("{storyboard}/{step}/{app}/{variant}")
}

/// The citation fields of a verdict or finding, in the order they are
/// reported.
fn citation<'a>(
    storyboard: &'a str,
    step: &'a str,
    app: &'a str,
    variant: &'a str,
    frame: &'a str,
) -> [(&'static str, &'a str); 5] {
    [
        ("storyboard", storyboard),
        ("step", step),
        ("app", app),
        ("variant", variant),
        ("frame", frame),
    ]
}

/// A frame path is in the bundle when it is relative, stays inside it and
/// exists (through the `runs` and `base` links).
fn frame_in_bundle(bundle: &Path, frame: &str) -> bool {
    let path = Path::new(frame);
    path.is_relative()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        && bundle.join(path).is_file()
}

/// Every rule of FR-019, against a manifest and the bundle's files.
pub fn validate(review: &Review, manifest: &Manifest, bundle: &Path) -> Vec<Rejection> {
    let mut found = Vec::new();
    let mut check_entry = |what: &'static str,
                           index: usize,
                           cited: [(&'static str, &str); 5],
                           severity_missing: Option<&'static str>,
                           says: &str| {
        for (field, value) in cited {
            if value.trim().is_empty() {
                found.push(Rejection::MissingCitation { what, index, field });
            }
        }
        let frame = cited[4].1;
        if !frame.trim().is_empty() && !frame_in_bundle(bundle, frame) {
            found.push(Rejection::FrameAbsent {
                what,
                index,
                frame: frame.to_owned(),
            });
        }
        if let Some(kind) = severity_missing {
            found.push(Rejection::MissingSeverity { what, index, kind });
        }
        if says.trim().is_empty() {
            found.push(Rejection::EmptySays { what, index });
        }
    };
    for (i, v) in review.verdicts.iter().enumerate() {
        let missing = (v.verdict == Kind::Fail && v.severity.is_none()).then_some("fail");
        check_entry(
            "verdict",
            i + 1,
            citation(&v.storyboard, &v.step, &v.app, &v.variant, &v.frame),
            missing,
            &v.says,
        );
    }
    for (i, f) in review.findings.iter().enumerate() {
        let missing = f.severity.is_none().then_some("finding");
        check_entry(
            "finding",
            i + 1,
            citation(&f.storyboard, &f.step, &f.app, &f.variant, &f.frame),
            missing,
            &f.says,
        );
    }
    let covered: BTreeSet<String> = review
        .verdicts
        .iter()
        .map(|v| reference(&v.storyboard, &v.step, &v.app, &v.variant))
        .collect();
    for batch in &manifest.batches {
        for step in &batch.steps {
            let wanted = reference(
                &step.storyboard,
                &step.step,
                &step.app.to_string(),
                &step.variant,
            );
            if !covered.contains(&wanted) {
                found.push(Rejection::StepWithoutVerdict(wanted));
            }
        }
    }
    found
}

/// Attaches contests to the verdicts and findings they name. A contest that
/// names neither is returned as a rejection: it is never silently dropped.
pub fn attach_contests(review: &mut Review, contests: &[Contest]) -> Vec<Rejection> {
    let mut dangling = Vec::new();
    for contest in contests {
        let resolution = Resolution::Contested {
            reason: contest.reason.clone(),
        };
        let mut hit = false;
        for v in &mut review.verdicts {
            if reference(&v.storyboard, &v.step, &v.app, &v.variant) == contest.reference {
                v.resolution = resolution.clone();
                hit = true;
            }
        }
        for f in &mut review.findings {
            if reference(&f.storyboard, &f.step, &f.app, &f.variant) == contest.reference {
                f.resolution = resolution.clone();
                hit = true;
            }
        }
        if !hit {
            dangling.push(Rejection::ContestWithoutTarget(contest.reference.clone()));
        }
    }
    dangling
}

/// The contests in `contests.toml`, or none when there is no file.
pub fn load_contests(bundle: &Path) -> Result<Vec<Contest>, String> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        contest: Vec<Contest>,
    }
    let path = bundle.join("contests.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str::<File>(&text)
        .map(|file| file.contest)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads and validates a bundle's review, with its contests attached.
pub fn check(bundle: &Path) -> Result<Review, Vec<Rejection>> {
    let manifest =
        crate::prompt::load_manifest(bundle).map_err(|e| vec![Rejection::Unreadable(e)])?;
    let path = bundle.join("verdicts.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| vec![Rejection::Unreadable(format!("{}: {e}", path.display()))])?;
    let mut review: Review = serde_json::from_str(&text)
        .map_err(|e| vec![Rejection::Unreadable(format!("{}: {e}", path.display()))])?;
    let mut rejections = validate(&review, &manifest, bundle);
    match load_contests(bundle) {
        Ok(contests) => rejections.extend(attach_contests(&mut review, &contests)),
        Err(error) => rejections.push(Rejection::Unreadable(error)),
    }
    if rejections.is_empty() {
        Ok(review)
    } else {
        Err(rejections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::App;
    use crate::bundle::{self, Inputs, all_new};
    use crate::fixtures::{run, storyboard, write};
    use serde_json::{Value, json};

    /// A bundle of one run of two steps, and a complete review of it.
    fn bundled() -> (tempfile::TempDir, Review) {
        let dir = tempfile::tempdir().expect("temp");
        let runs = dir.path().join("runs");
        write(&runs, &run("archive-walks-down", App::Classic, &[], 2));
        let catalogue = dir.path().join("storyboards/list");
        std::fs::create_dir_all(&catalogue).expect("dir");
        std::fs::write(catalogue.join("archive-walks-down.toml"), storyboard(None)).expect("board");
        // `storyboard()` has no name field; the name is the file's stem.
        std::fs::write(dir.path().join("acc.md"), "acceptance").expect("acc");
        bundle::build(
            &Inputs {
                runs: &runs,
                base: None,
                base_sha: None,
                acceptance: &dir.path().join("acc.md"),
                catalogue: &dir.path().join("storyboards"),
                design_dirs: &[],
                out: &dir.path().join("bundle"),
            },
            &all_new,
        )
        .expect("bundle");
        let review: Review = serde_json::from_value(complete()).expect("review");
        (dir, review)
    }

    fn verdict(step: &str, kind: &str) -> Value {
        json!({
            "storyboard": "archive-walks-down",
            "step": step,
            "app": "classic",
            "variant": "default",
            "frame": format!("runs/classic/archive-walks-down/default/0{step}.outlined.png"),
            "verdict": kind,
            "says": "The row below takes the cursor.",
            "rule": "ux-architect §2"
        })
    }

    fn complete() -> Value {
        json!({
            "bundle": { "tree_key": "treekey", "base": null },
            "reviewer": { "agent": "ux-reviewer", "model": "m", "template": "t" },
            "verdicts": [verdict("0", "pass"), verdict("1", "pass")],
            "findings": []
        })
    }

    fn errors(review: &Review, bundle_dir: &Path) -> Vec<Rejection> {
        let manifest = crate::prompt::load_manifest(bundle_dir).expect("manifest");
        validate(review, &manifest, bundle_dir)
    }

    fn rejected(value: Value) -> Vec<Rejection> {
        let (dir, _) = bundled();
        let review: Review = serde_json::from_value(value).expect("schema");
        errors(&review, &dir.path().join("bundle"))
    }

    #[test]
    fn a_complete_review_passes() {
        let (dir, review) = bundled();
        assert_eq!(errors(&review, &dir.path().join("bundle")), []);
    }

    #[test]
    fn a_verdict_missing_a_citation_field_is_rejected_by_name() {
        for field in ["storyboard", "step", "app", "variant", "frame"] {
            let mut value = complete();
            value["verdicts"][1]
                .as_object_mut()
                .expect("o")
                .remove(field);
            let found = rejected(value);
            assert!(
                found.iter().any(|r| matches!(r,
                    Rejection::MissingCitation { what: "verdict", index: 2, field: f } if *f == field)),
                "{field}: {found:?}"
            );
        }
    }

    #[test]
    fn a_finding_missing_a_citation_field_is_rejected_by_name() {
        let mut value = complete();
        value["findings"] = json!([{
            "storyboard": "archive-walks-down", "step": "1", "app": "classic",
            "frame": "runs/classic/archive-walks-down/default/01.outlined.png",
            "severity": "polish", "says": "A clipped label.", "rule": "canvas 01"
        }]);
        let found = rejected(value);
        assert!(
            found.iter().any(|r| matches!(
                r,
                Rejection::MissingCitation {
                    what: "finding",
                    index: 1,
                    field: "variant"
                }
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_frame_path_absent_from_the_bundle_is_rejected() {
        let mut value = complete();
        value["verdicts"][0]["frame"] =
            json!("runs/classic/archive-walks-down/default/09.outlined.png");
        let found = rejected(value);
        assert!(
            found.iter().any(|r| matches!(r,
                Rejection::FrameAbsent { what: "verdict", index: 1, frame } if frame.ends_with("09.outlined.png"))),
            "{found:?}"
        );
    }

    #[test]
    fn a_manifest_step_with_no_verdict_is_rejected() {
        let mut value = complete();
        value["verdicts"].as_array_mut().expect("a").pop();
        let found = rejected(value);
        assert_eq!(
            found,
            [Rejection::StepWithoutVerdict(
                "archive-walks-down/1/classic/default".into()
            )]
        );
    }

    #[test]
    fn a_fail_without_severity_is_rejected_but_a_pass_needs_none() {
        let mut value = complete();
        value["verdicts"][1]["verdict"] = json!("fail");
        let found = rejected(value.clone());
        assert_eq!(
            found,
            [Rejection::MissingSeverity {
                what: "verdict",
                index: 2,
                kind: "fail"
            }]
        );
        value["verdicts"][1]["severity"] = json!("wrong");
        assert_eq!(rejected(value), []);
    }

    #[test]
    fn a_finding_without_severity_is_rejected() {
        let mut value = complete();
        value["findings"] = serde_json::from_str(
            r#"[{"storyboard":"archive-walks-down","step":"1","app":"classic","variant":"default",
            "frame":"runs/classic/archive-walks-down/default/01.outlined.png",
            "says":"A clipped label.","rule":"canvas 01"}]"#,
        )
        .expect("json");
        assert_eq!(
            rejected(value),
            [Rejection::MissingSeverity {
                what: "finding",
                index: 1,
                kind: "finding"
            }]
        );
    }

    #[test]
    fn an_empty_says_is_rejected() {
        for says in ["", "   "] {
            let mut value = complete();
            value["verdicts"][0]["says"] = json!(says);
            assert_eq!(
                rejected(value),
                [Rejection::EmptySays {
                    what: "verdict",
                    index: 1
                }]
            );
        }
    }

    #[test]
    fn check_reads_the_file_and_names_every_rejection() {
        let (dir, _) = bundled();
        let bundle = dir.path().join("bundle");
        assert!(matches!(
            check(&bundle).expect_err("no file").as_slice(),
            [Rejection::Unreadable(_)]
        ));
        std::fs::write(bundle.join("verdicts.json"), "{ not json").expect("write");
        assert!(matches!(
            check(&bundle).expect_err("bad json").as_slice(),
            [Rejection::Unreadable(_)]
        ));
        let mut value = complete();
        value["verdicts"][0]["says"] = json!("");
        value["verdicts"].as_array_mut().expect("a").pop();
        std::fs::write(bundle.join("verdicts.json"), value.to_string()).expect("write");
        assert_eq!(check(&bundle).expect_err("two problems").len(), 2);
        std::fs::write(bundle.join("verdicts.json"), complete().to_string()).expect("write");
        assert_eq!(check(&bundle).expect("complete").verdicts.len(), 2);
    }

    #[test]
    fn contests_attach_to_the_verdict_and_finding_they_name() {
        let (dir, _) = bundled();
        let bundle = dir.path().join("bundle");
        let mut value = complete();
        value["verdicts"][1]["verdict"] = json!("fail");
        value["verdicts"][1]["severity"] = json!("wrong");
        std::fs::write(bundle.join("verdicts.json"), value.to_string()).expect("write");
        std::fs::write(
            bundle.join("contests.toml"),
            "[[contest]]\nref = \"archive-walks-down/1/classic/default\"\nreason = \"The cursor is where the person left it.\"\n",
        )
        .expect("contests");
        let review = check(&bundle).expect("complete");
        assert_eq!(review.verdicts[0].resolution, Resolution::Open);
        assert_eq!(
            review.verdicts[1].resolution,
            Resolution::Contested {
                reason: "The cursor is where the person left it.".into()
            }
        );
    }

    #[test]
    fn a_contest_naming_nothing_is_rejected_not_dropped() {
        let (dir, _) = bundled();
        let bundle = dir.path().join("bundle");
        std::fs::write(bundle.join("verdicts.json"), complete().to_string()).expect("write");
        std::fs::write(
            bundle.join("contests.toml"),
            "[[contest]]\nref = \"nope/1/classic/default\"\nreason = \"x\"\n",
        )
        .expect("contests");
        assert_eq!(
            check(&bundle).expect_err("dangling"),
            [Rejection::ContestWithoutTarget(
                "nope/1/classic/default".into()
            )]
        );
    }

    #[test]
    fn the_schema_in_the_contract_deserializes() {
        let review: Review = serde_json::from_value(json!({
            "bundle": { "tree_key": "k", "base": "b" },
            "reviewer": { "agent": "ux-reviewer", "model": "m", "template": "t" },
            "verdicts": [{
                "storyboard": "s", "step": "archive", "app": "classic", "variant": "default",
                "frame": "f", "verdict": "fail", "severity": "wrong", "says": "x", "rule": "r"
            }],
            "findings": [{
                "storyboard": "s", "step": "archive", "app": "classic", "variant": "scheme=dark",
                "frame": "f", "severity": "polish", "says": "x", "rule": "r"
            }]
        }))
        .expect("schema");
        assert_eq!(review.verdicts[0].verdict, Kind::Fail);
        assert_eq!(review.findings[0].severity, Some(Severity::Polish));
    }
}
