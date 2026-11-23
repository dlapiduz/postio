//! The reviewer's answer, and the validator that decides whether it is one (`contracts/review.md`).
//! A review that cannot be traced back to a frame is worth nothing, so the validator is strict
//! about citation and says which rule each rejection broke.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bundle::Manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Pass,
    Fail,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Blocker,
    Wrong,
    Polish,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    #[default]
    Open,
    Fixed {
        rerun: String,
    },
    Contested {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(default)]
    pub storyboard: String,
    #[serde(default, deserialize_with = "step_name")]
    pub step: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub frame: String,
    pub verdict: Kind,
    #[serde(default)]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub says: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub resolution: Resolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub storyboard: String,
    #[serde(default, deserialize_with = "step_name")]
    pub step: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub frame: String,
    #[serde(default)]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub says: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub resolution: Resolution,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleId {
    #[serde(default)]
    pub tree_key: String,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub template: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    #[serde(default)]
    pub bundle: BundleId,
    #[serde(default)]
    pub reviewer: Reviewer,
    #[serde(default)]
    pub verdicts: Vec<Verdict>,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Contest {
    #[serde(rename = "ref")]
    pub reference: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("verdicts.json: {0}")]
    Unreadable(String),
    #[error("{what} {index}: missing citation field `{field}`")]
    MissingCitation {
        what: &'static str,
        index: usize,
        field: &'static str,
    },
    #[error("{what} {index}: frame `{frame}` is not in the bundle")]
    FrameAbsent {
        what: &'static str,
        index: usize,
        frame: String,
    },
    #[error("no verdict for {0}")]
    StepWithoutVerdict(String),
    #[error("{what} {index}: a {kind} needs a severity (blocker, wrong or polish)")]
    MissingSeverity {
        what: &'static str,
        index: usize,
        kind: &'static str,
    },
    #[error("{what} {index}: `says` is empty")]
    EmptySays { what: &'static str, index: usize },
    #[error("contests.toml: `{0}` matches no verdict or finding")]
    ContestWithoutTarget(String),
}

pub fn reference(storyboard: &str, step: &str, app: &str, variant: &str) -> String {
    format!("{storyboard}/{step}/{app}/{variant}")
}

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

fn frame_in_bundle(bundle: &Path, frame: &str) -> bool {
    let path = Path::new(frame);
    path.is_relative()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        && bundle.join(path).is_file()
}

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

/// Batches are taken in number order; the bundle and reviewer come from the first.
pub fn merge(bundle: &Path) -> Result<usize, String> {
    let mut batches: Vec<(usize, std::path::PathBuf)> = std::fs::read_dir(bundle)
        .map_err(|error| format!("{}: {error}", bundle.display()))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix("verdicts.")?.strip_suffix(".json")?;
            Some((number.parse().ok()?, entry.path()))
        })
        .collect();
    batches.sort_by_key(|(number, _)| *number);
    let mut merged: Option<Review> = None;
    for (_, path) in &batches {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let review: Review =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        match &mut merged {
            None => merged = Some(review),
            Some(all) => {
                all.verdicts.extend(review.verdicts);
                all.findings.extend(review.findings);
            }
        }
    }
    if let Some(all) = merged {
        let json = serde_json::to_string_pretty(&all).map_err(|error| error.to_string())?;
        std::fs::write(bundle.join("verdicts.json"), json + "\n")
            .map_err(|error| format!("verdicts.json: {error}"))?;
    }
    Ok(batches.len())
}

fn step_name<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Step {
        Name(String),
        Index(u64),
    }
    Ok(match Step::deserialize(deserializer)? {
        Step::Name(name) => name,
        Step::Index(index) => index.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::App;
    use crate::bundle::{self, Inputs, all_new};
    use crate::fixtures::{run, storyboard, write};
    use serde_json::{Value, json};

    fn bundled() -> (tempfile::TempDir, Review) {
        let dir = tempfile::tempdir().expect("temp");
        let runs = dir.path().join("runs");
        write(&runs, &run("archive-walks-down", App::Terminal, &[], 2));
        let catalogue = dir.path().join("storyboards/list");
        std::fs::create_dir_all(&catalogue).expect("dir");
        std::fs::write(catalogue.join("archive-walks-down.toml"), storyboard(None)).expect("board");
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
            "app": "terminal",
            "variant": "default",
            "frame": format!("runs/terminal/archive-walks-down/default/0{step}.outlined.png"),
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
    fn a_step_written_as_a_number_is_its_index() {
        let mut value = complete();
        value["verdicts"][1]["step"] = json!(1);
        let review: Review = serde_json::from_value(value).expect("a number is a step");
        assert_eq!(review.verdicts[1].step, "1");
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
            "storyboard": "archive-walks-down", "step": "1", "app": "terminal",
            "frame": "runs/terminal/archive-walks-down/default/01.outlined.png",
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
            json!("runs/terminal/archive-walks-down/default/09.outlined.png");
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
                "archive-walks-down/1/terminal/default".into()
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
            r#"[{"storyboard":"archive-walks-down","step":"1","app":"terminal","variant":"default",
            "frame":"runs/terminal/archive-walks-down/default/01.outlined.png",
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
            "[[contest]]\nref = \"archive-walks-down/1/terminal/default\"\nreason = \"The cursor is where the person left it.\"\n",
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
            "[[contest]]\nref = \"nope/1/terminal/default\"\nreason = \"x\"\n",
        )
        .expect("contests");
        assert_eq!(
            check(&bundle).expect_err("dangling"),
            [Rejection::ContestWithoutTarget(
                "nope/1/terminal/default".into()
            )]
        );
    }

    #[test]
    fn the_schema_in_the_contract_deserializes() {
        let review: Review = serde_json::from_value(json!({
            "bundle": { "tree_key": "k", "base": "b" },
            "reviewer": { "agent": "ux-reviewer", "model": "m", "template": "t" },
            "verdicts": [{
                "storyboard": "s", "step": "archive", "app": "terminal", "variant": "default",
                "frame": "f", "verdict": "fail", "severity": "wrong", "says": "x", "rule": "r"
            }],
            "findings": [{
                "storyboard": "s", "step": "archive", "app": "terminal", "variant": "scheme=dark",
                "frame": "f", "severity": "polish", "says": "x", "rule": "r"
            }]
        }))
        .expect("schema");
        assert_eq!(review.verdicts[0].verdict, Kind::Fail);
        assert_eq!(review.findings[0].severity, Some(Severity::Polish));
    }

    #[test]
    fn batches_merge_into_one_review_in_number_order() {
        let dir = tempfile::tempdir().expect("temp");
        let batch = |n: usize, storyboard: &str| {
            serde_json::json!({
                "bundle": { "tree_key": "k", "base": null },
                "reviewer": { "agent": "ux-reviewer", "model": "m", "template": "t" },
                "verdicts": [{
                    "storyboard": storyboard, "step": "1", "app": "terminal",
                    "variant": "default", "frame": "runs/x.png",
                    "verdict": "pass", "says": "fine"
                }],
                "findings": []
            })
            .to_string()
            .pipe(|text| std::fs::write(dir.path().join(format!("verdicts.{n}.json")), text))
        };
        batch(2, "second").expect("written");
        batch(1, "first").expect("written");
        assert_eq!(merge(dir.path()), Ok(2));
        let merged: Review = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("verdicts.json")).expect("merged"),
        )
        .expect("a review");
        let order: Vec<_> = merged
            .verdicts
            .iter()
            .map(|v| v.storyboard.as_str())
            .collect();
        assert_eq!(order, ["first", "second"]);
        assert_eq!(merged.bundle.tree_key, "k");
    }

    trait Pipe: Sized {
        fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
            f(self)
        }
    }
    impl Pipe for String {}
}
