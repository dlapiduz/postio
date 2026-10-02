//! Evaluating a step's checks against what the app said it saw.
//!
//! An app's runner serialises its `Observation` after every step and hands
//! the whole history here, so the rule for what `same_as`, `changed` or
//! `absent` means lives in one place and two runners cannot read a
//! storyboard differently (`contracts/runner.md`).
//!
//! Three outcomes, not two. A check on a field the app declares it does not
//! observe -- `back_depth` on both GTK apps, which have no back stack -- is
//! **not applicable**: it is never a pass, because nothing was checked, and
//! never a failure, because nothing is wrong (data-model § Checks).

use serde::Serialize;
use serde_json::Value;

use crate::format::{Check, Checks, Leaf, StepRef};

/// What one check came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The observation agrees.
    Pass,
    /// It does not.
    Fail,
    /// The app does not observe this field, so nothing was checked.
    NotApplicable,
}

/// One check's result, worded for whoever reads the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    /// The dotted path into the observation, such as `cursor.index`.
    pub path: String,
    /// What was expected, in words: `= 1`, `same as step "down" (= 1)`.
    pub expected: String,
    /// What the observation held; `None` when the field was absent.
    pub observed: Option<Value>,
    /// What it came to.
    pub outcome: Outcome,
}

/// Everything a step's checks are evaluated against.
pub struct History<'a> {
    /// Serialised observations: `[0]` is the starting state, `[i]` the state
    /// after step `i` (1-based, as storyboards number steps).
    pub observations: &'a [Value],
    /// Turns a `same_as` reference into a 1-based step number.
    pub step_number: &'a dyn Fn(&StepRef) -> Option<usize>,
    /// Paths the app declares it does not observe. A check at or under one
    /// of these is not applicable.
    pub unobserved: &'a [&'a str],
}

/// Evaluates `checks` for step `step` (1-based; 0 is the starting state).
pub fn evaluate(checks: &Checks, step: usize, history: &History<'_>) -> Vec<CheckResult> {
    let mut results = Vec::new();
    walk(checks, "", step, history, &mut results);
    results
}

fn walk(
    checks: &Checks,
    prefix: &str,
    step: usize,
    history: &History<'_>,
    results: &mut Vec<CheckResult>,
) {
    for (key, check) in &checks.0 {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match check {
            Check::Table(inner) => walk(inner, &path, step, history, results),
            Check::Leaf(leaf) => results.push(leaf_result(leaf, path, step, history)),
        }
    }
}

fn leaf_result(leaf: &Leaf, path: String, step: usize, history: &History<'_>) -> CheckResult {
    let observed = history
        .observations
        .get(step)
        .and_then(|observation| field(observation, &path));
    let unobserved = history
        .unobserved
        .iter()
        .any(|declared| path == *declared || path.starts_with(&format!("{declared}.")));
    if unobserved {
        return CheckResult {
            expected: describe(leaf, &path, step, history),
            path,
            observed: None,
            outcome: Outcome::NotApplicable,
        };
    }
    let previous = step
        .checked_sub(1)
        .and_then(|before| history.observations.get(before))
        .map(|observation| field(observation, &path));
    let pass = match leaf {
        Leaf::Literal(value) => observed.as_ref() == Some(value),
        Leaf::OneOf(values) => observed.as_ref().is_some_and(|seen| values.contains(seen)),
        Leaf::Absent => observed.is_none(),
        // At the starting state there is no step before, so neither can
        // hold: a storyboard that asks has asked a question with no answer.
        Leaf::Changed => previous.is_some_and(|before| before != observed),
        Leaf::Unchanged => previous.is_some_and(|before| before == observed),
        Leaf::SameAs(reference) => {
            same_as(reference, &path, history).is_some_and(|earlier| earlier == observed)
        }
    };
    CheckResult {
        expected: describe(leaf, &path, step, history),
        path,
        observed,
        outcome: if pass { Outcome::Pass } else { Outcome::Fail },
    }
}

/// The field at a dotted path. Missing and `null` are the same thing: an
/// `Option` the app serialised as `None`.
fn field(observation: &Value, path: &str) -> Option<Value> {
    let mut node = observation;
    for part in path.split('.') {
        node = node.get(part)?;
    }
    (!node.is_null()).then(|| node.clone())
}

/// The field as it was at the referenced step: `None` when the reference
/// names no step, `Some(None)` when the step had no value there.
fn same_as(reference: &StepRef, path: &str, history: &History<'_>) -> Option<Option<Value>> {
    let number = (history.step_number)(reference)?;
    let observation = history.observations.get(number)?;
    Some(field(observation, path))
}

fn describe(leaf: &Leaf, path: &str, step: usize, history: &History<'_>) -> String {
    let shown = |value: &Option<Value>| match value {
        Some(value) => value.to_string(),
        None => "absent".to_owned(),
    };
    match leaf {
        Leaf::Literal(value) => format!("= {value}"),
        Leaf::OneOf(values) => format!(
            "one of [{}]",
            values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Leaf::Absent => "absent".to_owned(),
        Leaf::Changed | Leaf::Unchanged => {
            let word = if matches!(leaf, Leaf::Changed) {
                "changed from"
            } else {
                "unchanged from"
            };
            match step
                .checked_sub(1)
                .and_then(|before| history.observations.get(before))
            {
                Some(before) => format!("{word} {}", shown(&field(before, path))),
                None => format!("{word} the step before, but this is the starting state"),
            }
        }
        Leaf::SameAs(reference) => {
            let named = match reference {
                StepRef::Id(id) => format!("\"{id}\""),
                StepRef::Index(n) => n.to_string(),
            };
            match same_as(reference, path, history) {
                Some(earlier) => format!("same as step {named} (= {})", shown(&earlier)),
                None => format!("same as step {named}, which is not an earlier step"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn checks(pairs: &[(&str, Leaf)]) -> Checks {
        // Builds the nested tree a dotted TOML key would, so the evaluator is
        // tested on the shape the format really produces.
        let mut root = Checks(BTreeMap::new());
        for (path, leaf) in pairs {
            let mut node = &mut root;
            let parts: Vec<&str> = path.split('.').collect();
            for part in &parts[..parts.len() - 1] {
                let entry = node
                    .0
                    .entry((*part).to_owned())
                    .or_insert_with(|| Check::Table(Checks(BTreeMap::new())));
                node = match entry {
                    Check::Table(table) => table,
                    Check::Leaf(_) => unreachable!("test paths do not overlap"),
                };
            }
            node.0
                .insert(parts[parts.len() - 1].to_owned(), Check::Leaf(leaf.clone()));
        }
        root
    }

    fn by_id(reference: &StepRef) -> Option<usize> {
        match reference {
            StepRef::Id(id) if id == "down" => Some(1),
            StepRef::Id(_) => None,
            StepRef::Index(n) => Some(*n as usize),
        }
    }

    fn observations() -> Vec<Value> {
        vec![
            json!({ "cursor": { "index": 0 }, "keyboard": { "region": "list" }, "notice": { "text": null } }),
            json!({ "cursor": { "index": 1 }, "keyboard": { "region": "list" }, "notice": { "text": null } }),
            json!({ "cursor": { "index": 1 }, "keyboard": { "region": "list" }, "notice": { "text": "Archived", "undo": true } }),
        ]
    }

    fn run(pairs: &[(&str, Leaf)], step: usize, unobserved: &[&str]) -> Vec<CheckResult> {
        let observations = observations();
        evaluate(
            &checks(pairs),
            step,
            &History {
                observations: &observations,
                step_number: &by_id,
                unobserved,
            },
        )
    }

    fn outcomes(results: &[CheckResult]) -> Vec<(String, Outcome)> {
        results
            .iter()
            .map(|r| (r.path.clone(), r.outcome.clone()))
            .collect()
    }

    #[test]
    fn a_literal_passes_when_equal_and_fails_when_not() {
        let passed = run(&[("cursor.index", Leaf::Literal(json!(1)))], 1, &[]);
        assert_eq!(outcomes(&passed), [("cursor.index".into(), Outcome::Pass)]);
        let failed = run(
            &[("keyboard.region", Leaf::Literal(json!("search")))],
            1,
            &[],
        );
        assert_eq!(failed[0].outcome, Outcome::Fail);
        assert_eq!(failed[0].observed, Some(json!("list")));
        assert_eq!(failed[0].expected, "= \"search\"");
    }

    #[test]
    fn same_as_reads_the_named_earlier_step() {
        let passed = run(
            &[("cursor.index", Leaf::SameAs(StepRef::Id("down".into())))],
            2,
            &[],
        );
        assert_eq!(passed[0].outcome, Outcome::Pass);
        assert_eq!(passed[0].expected, "same as step \"down\" (= 1)");
        let failed = run(&[("cursor.index", Leaf::SameAs(StepRef::Index(0)))], 2, &[]);
        assert_eq!(failed[0].outcome, Outcome::Fail);
    }

    #[test]
    fn same_as_an_unknown_step_fails_rather_than_passing() {
        let results = run(
            &[("cursor.index", Leaf::SameAs(StepRef::Id("nowhere".into())))],
            2,
            &[],
        );
        assert_eq!(results[0].outcome, Outcome::Fail);
    }

    #[test]
    fn changed_and_unchanged_compare_with_the_step_before() {
        assert_eq!(
            run(&[("cursor.index", Leaf::Changed)], 1, &[])[0].outcome,
            Outcome::Pass
        );
        assert_eq!(
            run(&[("cursor.index", Leaf::Changed)], 2, &[])[0].outcome,
            Outcome::Fail
        );
        assert_eq!(
            run(&[("cursor.index", Leaf::Unchanged)], 2, &[])[0].outcome,
            Outcome::Pass
        );
        assert_eq!(
            run(&[("cursor.index", Leaf::Unchanged)], 1, &[])[0].outcome,
            Outcome::Fail
        );
    }

    #[test]
    fn the_starting_state_has_no_step_before_it() {
        let results = run(&[("cursor.index", Leaf::Unchanged)], 0, &[]);
        assert_eq!(results[0].outcome, Outcome::Fail);
    }

    #[test]
    fn absent_means_null_or_missing() {
        assert_eq!(
            run(&[("notice.text", Leaf::Absent)], 1, &[])[0].outcome,
            Outcome::Pass
        );
        assert_eq!(
            run(&[("notice.undo", Leaf::Absent)], 1, &[])[0].outcome,
            Outcome::Pass
        );
        assert_eq!(
            run(&[("notice.text", Leaf::Absent)], 2, &[])[0].outcome,
            Outcome::Fail
        );
    }

    #[test]
    fn one_of_passes_on_any_listed_value() {
        let leaf = Leaf::OneOf(vec![json!("search"), json!("list")]);
        assert_eq!(
            run(&[("keyboard.region", leaf.clone())], 1, &[])[0].outcome,
            Outcome::Pass
        );
        let other = Leaf::OneOf(vec![json!("reader")]);
        assert_eq!(
            run(&[("keyboard.region", other)], 1, &[])[0].outcome,
            Outcome::Fail
        );
    }

    #[test]
    fn a_missing_field_fails_a_literal_and_names_nothing_observed() {
        let results = run(&[("overlay.kind", Leaf::Literal(json!("finder")))], 1, &[]);
        assert_eq!(results[0].outcome, Outcome::Fail);
        assert_eq!(results[0].observed, None);
    }

    #[test]
    fn an_unobserved_field_is_not_applicable_never_pass_or_fail() {
        let results = run(
            &[("back_depth", Leaf::Literal(json!(0)))],
            1,
            &["back_depth"],
        );
        assert_eq!(results[0].outcome, Outcome::NotApplicable);
        let nested = run(&[("app.focus.bulk", Leaf::Absent)], 1, &["app.focus"]);
        assert_eq!(nested[0].outcome, Outcome::NotApplicable);
    }

    #[test]
    fn every_leaf_in_a_nested_tree_is_reported_once() {
        let results = run(
            &[
                ("cursor.index", Leaf::Literal(json!(1))),
                ("keyboard.region", Leaf::Literal(json!("list"))),
                ("notice.text", Leaf::Absent),
            ],
            1,
            &[],
        );
        let mut paths: Vec<_> = results.iter().map(|r| r.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["cursor.index", "keyboard.region", "notice.text"]);
    }
}
