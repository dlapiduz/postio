//! What a storyboard may say.
//!
//! The loader accepts any file that has the grammar's shape. The lint is the
//! rest of the contract: that the ids name real commands, the chords parse,
//! no address is a real one, every back-reference looks back, and a shared
//! storyboard checks only what every app can observe. Each failure is its own
//! named error, so a test (and a person) can say which rule a file broke.

use std::path::{Path, PathBuf};

use crate::format::Storyboard;

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
pub fn lint(_storyboard: &Storyboard) -> Vec<LintError> {
    Vec::new()
}

/// Load and lint every storyboard under `root`, returning what went wrong as
/// `(file, message)`. Gap lists (`gaps/`) are not storyboards, and
/// non-`.toml` files (the README, `.gitkeep`) are not either.
pub fn lint_catalogue(_root: &Path) -> Vec<(PathBuf, String)> {
    Vec::new()
}

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

/// Labels any one of which makes a domain reserved, as the personal-data
/// check spells them. A test reads that script, so the two cannot drift.
const RESERVED_LABELS: [&str; 1] = ["example"];
/// Top-level domains that are reserved on their own.
const RESERVED_TLDS: [&str; 3] = ["test", "invalid", "localhost"];
