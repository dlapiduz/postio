//! What a storyboard may say.
//!
//! The loader accepts any file that has the grammar's shape. The lint is the
//! rest of the contract: that the ids name real commands, the chords parse,
//! no address is a real one, every back-reference looks back, and a shared
//! storyboard checks only what every app can observe. Each failure is its own
//! named error, so a test (and a person) can say which rule a file broke.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use postio_config::keys::expand_mod;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_ui::keymap::Binding;
use serde_json::Value;

use crate::format::{Apps, Checks, Input, Leaf, SourceKind, Step, StepRef, Storyboard, Wait, load};

/// One rule a storyboard broke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LintError {
    /// There is no `source`.
    #[error("no `source`: a storyboard says why it exists")]
    MissingSource,
    /// An issue storyboard names no `proof`.
    #[error("`proof` is required when the source is an issue")]
    MissingProof,
    /// A `command` that is not in the registry.
    #[error("step {step}: `{command}` is not a command id")]
    UnknownCommand {
        /// The 1-based step.
        step: usize,
        /// What was written.
        command: String,
    },
    /// A `key` that does not parse once `mod` is expanded.
    #[error("step {step}: key `{key}` does not parse: {reason}")]
    BadChord {
        /// The 1-based step.
        step: usize,
        /// What was written.
        key: String,
        /// What the parser said.
        reason: String,
    },
    /// An address that is not on a reserved domain.
    #[error(
        "`{address}` is not on a reserved domain (example.com, .test, .invalid, .example, .localhost)"
    )]
    UnreservedAddress {
        /// The address.
        address: String,
    },
    /// A `same_as` that does not name an earlier step.
    #[error("step {step}: `same_as` {target} is not an earlier step")]
    SameAsNotEarlier {
        /// The 1-based step it is written on.
        step: usize,
        /// What it names.
        target: String,
    },
    /// An override that names a step the storyboard does not have.
    #[error("override for {app} names step `{step}`, which does not exist")]
    OverrideMissingStep {
        /// The app.
        app: String,
        /// The step reference.
        step: String,
    },
    /// An override for an app the storyboard does not name.
    #[error("override for {app}, which the storyboard's `apps` does not include")]
    OverrideForOtherApp {
        /// The app.
        app: String,
    },
    /// `calibration` outside `storyboards/calibration/`.
    #[error("`calibration` belongs only under storyboards/calibration/")]
    CalibrationOutsideDirectory,
    /// `app.*` or `keyboard.widget` checked in a storyboard that does not
    /// name exactly one app.
    #[error("step {step}: `{path}` is not shared; name exactly one app to check it")]
    SharedCheckOnPrivateField {
        /// The 1-based step.
        step: usize,
        /// The field.
        path: String,
    },
    /// The name is not the file stem.
    #[error("name `{name}` differs from the file stem `{stem}`")]
    NameIsNotStem {
        /// The name.
        name: String,
        /// The file stem.
        stem: String,
    },
    /// Two steps share an id.
    #[error("two steps are called `{id}`")]
    DuplicateStepId {
        /// The id.
        id: String,
    },
    /// Two storyboards share a name.
    #[error("the name `{name}` is also used by {other}")]
    DuplicateName {
        /// The name.
        name: String,
        /// The other file.
        other: PathBuf,
    },
}

/// Every rule `storyboard` breaks. Empty means clean.
pub fn lint(storyboard: &Storyboard) -> Vec<LintError> {
    let mut errors = Vec::new();

    match &storyboard.source {
        None => errors.push(LintError::MissingSource),
        Some(source) if source.kind == SourceKind::Issue && storyboard.proof.is_none() => {
            errors.push(LintError::MissingProof);
        }
        Some(_) => {}
    }

    if let Some(stem) = storyboard.path.file_stem().map(|s| s.to_string_lossy())
        && storyboard.name != stem
    {
        errors.push(LintError::NameIsNotStem {
            name: storyboard.name.clone(),
            stem: stem.into_owned(),
        });
    }

    let in_calibration = storyboard.path.parent().is_some_and(|dir| {
        dir.components()
            .any(|part| part.as_os_str() == "calibration")
    });
    if storyboard.calibration.is_some() && !in_calibration {
        errors.push(LintError::CalibrationOutsideDirectory);
    }

    let one_app = matches!(&storyboard.apps, Apps::Named(apps) if apps.len() == 1);
    let mut seen_ids = BTreeSet::new();
    for (index, step) in storyboard.steps.iter().enumerate() {
        let number = index + 1;
        if let Some(id) = &step.id
            && !seen_ids.insert(id.as_str())
        {
            errors.push(LintError::DuplicateStepId { id: id.clone() });
        }
        match &step.input {
            Input::Command(command) if CommandId::from_str(command).is_err() => {
                errors.push(LintError::UnknownCommand {
                    step: number,
                    command: command.clone(),
                });
            }
            Input::Key(key) => {
                let expanded = expand_mod(key, Platform::Freedesktop);
                if let Err(reason) = Binding::from_str(&expanded) {
                    errors.push(LintError::BadChord {
                        step: number,
                        key: key.clone(),
                        reason: reason.to_string(),
                    });
                }
            }
            _ => {}
        }
        for checks in step_checks(step) {
            lint_references(storyboard, checks, number, &mut errors);
            if !one_app {
                for (path, _) in checks.leaves() {
                    if path == "keyboard.widget" || path == "app" || path.starts_with("app.") {
                        errors.push(LintError::SharedCheckOnPrivateField { step: number, path });
                    }
                }
            }
        }
    }

    for (app, steps) in &storyboard.overrides {
        let name = app.to_string();
        if let Apps::Named(apps) = &storyboard.apps
            && !apps.contains(app)
        {
            errors.push(LintError::OverrideForOtherApp { app: name.clone() });
        }
        for (reference, over) in steps {
            match position_of(storyboard, reference) {
                Some(number) => {
                    if let Some(checks) = &over.check {
                        lint_references(storyboard, checks, number, &mut errors);
                    }
                }
                None => errors.push(LintError::OverrideMissingStep {
                    app: name.clone(),
                    step: reference.clone(),
                }),
            }
        }
    }

    // Addresses can be in any string at all, so look at all of them rather
    // than at the fields someone thought of.
    let value = serde_json::to_value(storyboard).unwrap_or(Value::Null);
    let mut strings = Vec::new();
    collect_strings(&value, &mut strings);
    for text in strings {
        for address in addresses_in(text) {
            if !is_reserved(&address) {
                errors.push(LintError::UnreservedAddress { address });
            }
        }
    }
    errors
}

/// The checks a step carries: its own, those of a wait, and those of a settle.
fn step_checks(step: &Step) -> Vec<&Checks> {
    let mut out = vec![&step.check];
    if let Input::Wait(Wait::Until(until)) = &step.input {
        out.push(until);
    }
    if let Some(until) = step.settle.as_ref().and_then(|s| s.until.as_ref()) {
        out.push(until);
    }
    out
}

/// The 1-based position a step reference names: an explicit id first, then
/// an index. `None` when it names nothing.
fn position_of(storyboard: &Storyboard, reference: &str) -> Option<usize> {
    storyboard
        .steps
        .iter()
        .position(|step| step.id.as_deref() == Some(reference))
        .map(|index| index + 1)
        .or_else(|| {
            let index: usize = reference.parse().ok()?;
            (1..=storyboard.steps.len())
                .contains(&index)
                .then_some(index)
        })
}

fn lint_references(
    storyboard: &Storyboard,
    checks: &Checks,
    number: usize,
    errors: &mut Vec<LintError>,
) {
    for (_, leaf) in checks.leaves() {
        let Leaf::SameAs(target) = leaf else { continue };
        let reference = match target {
            StepRef::Id(id) => id.clone(),
            StepRef::Index(index) => index.to_string(),
        };
        // Step 0, the starting state, is earlier than every step.
        let starting_state = reference == "0";
        if !starting_state && !position_of(storyboard, &reference).is_some_and(|at| at < number) {
            errors.push(LintError::SameAsNotEarlier {
                step: number,
                target: reference,
            });
        }
    }
}

fn collect_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// The email addresses in `text`, shaped as the personal-data check shapes
/// them: a local part, an `@`, and a dotted domain ending in letters.
fn addresses_in(text: &str) -> Vec<String> {
    let local_char = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain_char = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    let mut found = Vec::new();
    for (at, _) in text.match_indices('@') {
        let local = text[..at]
            .chars()
            .rev()
            .take_while(|c| local_char(*c))
            .count();
        let domain: String = text[at + 1..]
            .chars()
            .take_while(|c| domain_char(*c))
            .collect();
        let domain = domain.trim_end_matches('.');
        let tld = domain.rsplit('.').next().unwrap_or("");
        if local > 0
            && domain.contains('.')
            && tld.len() >= 2
            && tld.chars().all(|c| c.is_ascii_alphabetic())
        {
            found.push(format!("{}@{domain}", &text[at - local..at]));
        }
    }
    found
}

fn is_reserved(address: &str) -> bool {
    let domain = address
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut labels = domain.split('.');
    let tld = domain.rsplit('.').next().unwrap_or("");
    RESERVED_TLDS.contains(&tld) || labels.any(|label| RESERVED_LABELS.contains(&label))
}

/// Load and lint every storyboard under `root`, returning what went wrong as
/// `(file, message)`. Gap lists (`gaps/`) are not storyboards, and
/// non-`.toml` files (the README, `.gitkeep`) are not either.
pub fn lint_catalogue(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort();
    let mut problems = Vec::new();
    let mut names: BTreeMap<String, PathBuf> = BTreeMap::new();
    for file in files {
        let board = match load(&file) {
            Ok(board) => board,
            Err(error) => {
                problems.push((file, error.to_string()));
                continue;
            }
        };
        for error in lint(&board) {
            problems.push((file.clone(), error.to_string()));
        }
        match names.get(&board.name) {
            Some(other) => {
                let error = LintError::DuplicateName {
                    name: board.name.clone(),
                    other: other.clone(),
                };
                problems.push((file, error.to_string()));
            }
            None => {
                names.insert(board.name.clone(), file);
            }
        }
    }
    problems
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "gaps") {
                collect_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "toml") {
            out.push(path);
        }
    }
}

/// Labels any one of which makes a domain reserved, as the personal-data
/// check spells them. A test reads that script, so the two cannot drift.
const RESERVED_LABELS: [&str; 1] = ["example"];
/// Top-level domains that are reserved on their own.
const RESERVED_TLDS: [&str; 3] = ["test", "invalid", "localhost"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    const GOOD: &str = "source = { kind = \"flow\", ref = \"x\" }\n";

    fn lint_at(text: &str, path: &str) -> Vec<LintError> {
        let board = parse(text, Path::new(path)).unwrap_or_else(|e| panic!("should load: {e}"));
        lint(&board)
    }

    fn lint_text(text: &str) -> Vec<LintError> {
        lint_at(text, "list/walk.toml")
    }

    #[test]
    fn a_clean_storyboard_has_no_errors() {
        let text = format!(
            "{GOOD}[[step]]\ncommand = \"next_message\"\n[[step]]\nkey = \"mod+z\"\n\
             [[step]]\ntype = \"ada@example.com\"\ncheck = {{ cursor.index = 1 }}\n"
        );
        assert_eq!(lint_text(&text), vec![]);
    }

    #[test]
    fn a_missing_source_is_an_error() {
        assert!(lint_text("seed = \"small\"\n").contains(&LintError::MissingSource));
    }

    #[test]
    fn an_issue_needs_a_proof() {
        let text = "source = { kind = \"issue\", ref = \"#1\" }\n";
        assert!(lint_text(text).contains(&LintError::MissingProof));
        let text = "proof = \"open\"\nsource = { kind = \"issue\", ref = \"#1\" }\n";
        assert_eq!(lint_text(text), vec![]);
        // Only an issue is asked for one.
        assert_eq!(lint_text(GOOD), vec![]);
    }

    #[test]
    fn an_unknown_command_id_is_an_error() {
        let errors = lint_text(&format!(
            "{GOOD}[[step]]\ncommand = \"next_message\"\n[[step]]\ncommand = \"teleport\"\n"
        ));
        assert_eq!(
            errors,
            vec![LintError::UnknownCommand {
                step: 2,
                command: "teleport".into()
            }]
        );
    }

    #[test]
    fn a_chord_that_does_not_parse_after_expanding_mod_is_an_error() {
        for key in ["mod+", "hyper+z", "ctrl+nonesuch"] {
            let errors = lint_text(&format!("{GOOD}[[step]]\nkey = \"{key}\"\n"));
            assert!(
                matches!(errors.as_slice(), [LintError::BadChord { step: 1, .. }]),
                "{key}: {errors:?}"
            );
        }
        for key in ["j", "shift+tab", "mod+z", "g g", "mod+shift+k"] {
            assert_eq!(
                lint_text(&format!("{GOOD}[[step]]\nkey = \"{key}\"\n")),
                vec![],
                "{key}"
            );
        }
    }

    #[test]
    fn an_address_on_a_real_domain_is_an_error() {
        let errors = lint_text(&format!(
            "{GOOD}[[step]]\ntype = \"user@realmail.com\"\nexpect = \"ada@example.com and bob@mail.test are fine\"\n"
        ));
        assert_eq!(
            errors,
            vec![LintError::UnreservedAddress {
                address: "user@realmail.com".into()
            }]
        );
    }

    #[test]
    fn addresses_are_found_in_every_string() {
        let text = "source = { kind = \"flow\", ref = \"a@real.org\" }\n\
                    [[step]]\nwait = { until = { notice.text = \"b@real.org\" } }\n\
                    [app.classic.step.1]\nexpect = \"c@real.org\"\n";
        let found: Vec<_> = lint_text(text)
            .into_iter()
            .filter(|e| matches!(e, LintError::UnreservedAddress { .. }))
            .collect();
        assert_eq!(found.len(), 3, "{found:?}");
    }

    #[test]
    fn the_reserved_domains_match_the_personal_data_check() {
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/checks/check-no-personal-data.py"),
        )
        .expect("the personal-data check is in the repository");
        for label in RESERVED_LABELS {
            assert!(
                script.contains(&format!("{label}(?:")),
                "script has no `{label}` label rule"
            );
        }
        let tlds = format!("(?:{})", RESERVED_TLDS.join("|"));
        assert!(
            script.contains(&tlds),
            "the script's reserved TLDs are not {tlds}"
        );
    }

    #[test]
    fn same_as_the_starting_state_is_earlier_than_every_step() {
        // Step 0 is where the storyboard began, so "back to how it was" is
        // `same_as = 0` -- a reference to the past, never to the future.
        let text = format!(
            "{GOOD}[[step]]\ncommand = \"back\"\ncheck = {{ cursor.index = {{ same_as = 0 }} }}\n"
        );
        assert_eq!(lint_text(&text), vec![]);
    }

    #[test]
    fn same_as_pointing_forward_is_an_error() {
        let text = format!(
            "{GOOD}[[step]]\nid = \"a\"\ncommand = \"back\"\ncheck = {{ cursor.index = {{ same_as = \"b\" }} }}\n\
             [[step]]\nid = \"b\"\ncommand = \"back\"\n"
        );
        assert_eq!(
            lint_text(&text),
            vec![LintError::SameAsNotEarlier {
                step: 1,
                target: "b".into()
            }]
        );
        // Itself, an index at or past it, and a stranger are all not earlier.
        for target in ["1", "2", "9", "\"nobody\""] {
            let text = format!(
                "{GOOD}[[step]]\ncommand = \"back\"\n[[step]]\ncommand = \"back\"\ncheck = {{ cursor.index = {{ same_as = {target} }} }}\n"
            );
            let expected = target != "1";
            assert_eq!(!lint_text(&text).is_empty(), expected, "{target}");
        }
    }

    #[test]
    fn an_override_naming_a_missing_step_is_an_error() {
        let text = format!(
            "{GOOD}[[step]]\nid = \"a\"\ncommand = \"back\"\n\
             [app.focus.step.a]\nexpect = \"fine\"\n[app.focus.step.1]\nexpect = \"fine\"\n\
             [app.focus.step.nope]\nexpect = \"x\"\n[app.focus.step.7]\nexpect = \"x\"\n"
        );
        let errors = lint_text(&text);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.contains(&LintError::OverrideMissingStep {
            app: "focus".into(),
            step: "nope".into()
        }));
        assert!(errors.contains(&LintError::OverrideMissingStep {
            app: "focus".into(),
            step: "7".into()
        }));
    }

    #[test]
    fn an_override_for_an_app_the_storyboard_excludes_is_an_error() {
        let text = format!(
            "apps = [\"classic\"]\n{GOOD}[[step]]\ncommand = \"back\"\n[app.focus.step.1]\nexpect = \"x\"\n"
        );
        assert_eq!(
            lint_text(&text),
            vec![LintError::OverrideForOtherApp {
                app: "focus".into()
            }]
        );
    }

    #[test]
    fn calibration_is_only_allowed_under_calibration() {
        let text = format!("calibration = \"must_fail\"\n{GOOD}");
        assert_eq!(
            lint_at(&text, "list/walk.toml"),
            vec![LintError::CalibrationOutsideDirectory]
        );
        assert_eq!(lint_at(&text, "storyboards/calibration/walk.toml"), vec![]);
    }

    #[test]
    fn private_fields_need_exactly_one_named_app() {
        let step = "[[step]]\ncommand = \"back\"\ncheck = { app.classic.pane = \"list\", keyboard.widget = \"x\" }\n";
        let shared = lint_text(&format!("{GOOD}{step}"));
        assert_eq!(shared.len(), 2, "{shared:?}");
        assert!(
            shared
                .iter()
                .all(|e| matches!(e, LintError::SharedCheckOnPrivateField { step: 1, .. }))
        );
        let two = lint_text(&format!("apps = [\"classic\", \"terminal\"]\n{GOOD}{step}"));
        assert_eq!(two.len(), 2, "{two:?}");
        assert_eq!(
            lint_text(&format!("apps = [\"classic\"]\n{GOOD}{step}")),
            vec![]
        );
    }

    #[test]
    fn a_name_that_differs_from_the_file_stem_is_an_error() {
        let errors = lint_at(&format!("name = \"other\"\n{GOOD}"), "list/walk.toml");
        assert_eq!(
            errors,
            vec![LintError::NameIsNotStem {
                name: "other".into(),
                stem: "walk".into()
            }]
        );
        assert_eq!(
            lint_at(&format!("name = \"walk\"\n{GOOD}"), "list/walk.toml"),
            vec![]
        );
    }

    #[test]
    fn two_steps_with_one_id_are_an_error() {
        let text = format!(
            "{GOOD}[[step]]\nid = \"a\"\ncommand = \"back\"\n[[step]]\nid = \"a\"\ncommand = \"back\"\n"
        );
        assert_eq!(
            lint_text(&text),
            vec![LintError::DuplicateStepId { id: "a".into() }]
        );
    }

    #[test]
    fn the_catalogue_walk_reports_a_bad_file_and_skips_what_is_not_a_storyboard() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("list")).unwrap();
        std::fs::create_dir_all(root.join("gaps")).unwrap();
        std::fs::write(root.join("README.md"), "# not a storyboard").unwrap();
        std::fs::write(root.join("list/.gitkeep"), "").unwrap();
        std::fs::write(root.join("gaps/classic.toml"), "[[gap]]\ncommand = \"x\"\n").unwrap();
        std::fs::write(root.join("list/good.toml"), GOOD).unwrap();
        std::fs::write(root.join("list/bad.toml"), "seed = \"small\"\n").unwrap();
        std::fs::write(
            root.join("list/broken.toml"),
            "[[step]]\ncommand = \"a\"\nkey = \"b\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("flows")).unwrap();
        std::fs::write(root.join("flows/good.toml"), GOOD).unwrap();

        let problems = lint_catalogue(root);
        let files: Vec<String> = problems
            .iter()
            .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(files.contains(&"bad.toml".to_owned()), "{problems:?}");
        assert!(files.contains(&"broken.toml".to_owned()), "{problems:?}");
        assert!(
            problems.iter().any(|(_, m)| m.contains("also used")),
            "duplicate names are caught: {problems:?}"
        );
        assert!(!files.contains(&"classic.toml".to_owned()), "{problems:?}");
    }

    #[test]
    fn the_catalogue_loads_and_lints() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../storyboards");
        assert!(root.is_dir(), "the catalogue is at {}", root.display());
        let problems = lint_catalogue(&root);
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
