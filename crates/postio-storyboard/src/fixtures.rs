//! Runs and storyboards for tests that need a tree on disk.

use std::collections::BTreeMap;
use std::path::Path;

use postio_ui::observe::Observation;
use serde_json::json;

use crate::apply::App;
use crate::run::{Delivery, Played, Run, RunWriter, Settle, Status, StepOutcome, StepRun};

pub fn observation() -> Observation {
    serde_json::from_value(json!({
        "window": "open",
        "view": "list",
        "scope": "Inbox",
        "keyboard": { "region": "list", "field": null, "typing": false, "reachable": true, "widget": "w" },
        "cursor": { "index": 1, "id": "7", "subject": "Quarterly" },
        "rows": { "first_visible": 0, "count": 11 },
        "selection": { "count": 0 },
        "overlay": { "kind": "none", "mode": null },
        "notice": { "text": null, "tone": null, "undo": false },
        "banner": { "title": null },
        "reading": { "id": null, "focused": null, "scroll": null },
        "composer": { "open": false, "detached": false },
        "back_depth": null
    }))
    .expect("fixture")
}

pub fn step(n: usize) -> StepRun {
    StepRun {
        step: n,
        id: None,
        input: None,
        expect: Some(format!("expectation {n}")),
        outcome: StepOutcome::Delivered,
        observation: observation(),
        checks: vec![],
        settle: Settle::Settled { ms: 10 },
        frame: None,
        outlined: Some(RunWriter::outlined(n)),
    }
}

pub fn run(name: &str, app: App, variant: &[(&str, &str)], steps: usize) -> Run {
    Run {
        storyboard: Played {
            name: name.into(),
            hash: "h".into(),
        },
        app,
        variant: variant
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        ignored_axes: vec![],
        tree_key: "treekey".into(),
        commit: "c".into(),
        delivery: Delivery::Chain,
        renderer: "cairo".into(),
        stride: 2,
        seed: "small".into(),
        preset: None,
        steps: (0..steps).map(step).collect(),
        status: Status::Passed,
    }
}

/// Writes a run, and an empty file for every outlined frame it names.
pub fn write(root: &Path, run: &Run) {
    let writer = RunWriter::new(root, run.app, &run.storyboard.name, &run.variant).expect("dir");
    writer.write(run).expect("run.json");
    for step in &run.steps {
        if let Some(outlined) = &step.outlined {
            std::fs::write(writer.dir().join(outlined), b"png").expect("frame");
        }
    }
}

/// A one-step storyboard, optionally naming a canvas screen.
pub fn storyboard(design: Option<&str>) -> String {
    let design = design
        .map(|d| format!("design = \"{d}\"\n"))
        .unwrap_or_default();
    format!(
        "source = {{ kind = \"flow\", ref = \"x\" }}\n{design}[[step]]\ncommand = \"back\"\ndesign = \"step-screen\"\n"
    )
}
