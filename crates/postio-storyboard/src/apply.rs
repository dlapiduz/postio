//! Which apps a storyboard applies to.
//!
//! Applicability is static (research R7): the loader and the lint can answer
//! it with no runner built, because it is a question about the command
//! registry and about what each runner declares, not about a run.

use std::collections::BTreeMap;

use postio_core::CommandId;
use postio_core::registry::{self, Frontend};
use serde::{Deserialize, Serialize};

use crate::format::{Apps, Input, Storyboard};

/// A Postio frontend a storyboard can be played against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    /// The GTK desktop client.
    Classic,
    /// The GTK client's focus layout (spec 007).
    Focus,
    /// The terminal client.
    Terminal,
    /// The macOS client.
    Macos,
}

impl std::fmt::Display for App {
    /// The name the storyboard file and `runner list` spell it with.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            App::Classic => "classic",
            App::Focus => "focus",
            App::Terminal => "terminal",
            App::Macos => "macos",
        })
    }
}

/// Whether `app` exists on this branch at all. macOS is built elsewhere and
/// is named in the vocabulary and nothing more (research R7).
pub fn present(app: App) -> bool {
    matches!(app, App::Classic | App::Focus | App::Terminal)
}

/// Whether `app` offers `command`, by what the registry says its frontend
/// is offered (`requires.offered_by`): the desktop does not offer what only
/// the terminal's composer has, Focus has no flag verb and Classic has no
/// has-action toggle. A command the
/// registry offers but the app never wired still counts as provided; that gap
/// is what the generated pass is for.
pub fn provides(app: App, command: CommandId) -> bool {
    let requires = registry::get(command).requires;
    match app {
        App::Classic => requires.offered_by(Frontend::Classic),
        App::Focus => requires.offered_by(Frontend::Focus),
        App::Terminal => requires.offered_by(Frontend::Terminal),
        App::Macos => false,
    }
}

/// What a runner declares it can build, as `runner list` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerInfo {
    /// The app.
    pub app: App,
    /// Seeds it can build.
    pub seeds: Vec<String>,
    /// Presets it names.
    pub presets: Vec<String>,
    /// Variant axes and the values it supports on each.
    pub axes: BTreeMap<String, Vec<String>>,
}

/// Whether a storyboard applies to an app, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    /// Play it.
    Applies,
    /// Do not; every reason is listed.
    NotApplicable(Vec<Reason>),
}

/// One reason a storyboard does not apply to an app.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Reason {
    /// The app is not built on this branch.
    #[error("not present on this branch")]
    NotPresentOnBranch,
    /// The storyboard's `apps` leaves it out.
    #[error("the storyboard does not name this app")]
    NotNamed,
    /// A command the app does not provide.
    #[error("the app does not provide `{0}`")]
    MissingCommand(String),
    /// A seed the app cannot build.
    #[error("the app cannot build seed `{0}`")]
    MissingSeed(String),
    /// A preset the app does not declare.
    #[error("the app does not declare preset `{0}`")]
    MissingPreset(String),
}

/// Whether `storyboard` applies to `app`, given what its runner declares.
///
/// Every reason is listed, not just the first, so one run of the lint says
/// everything standing between a storyboard and an app.
pub fn applies(storyboard: &Storyboard, app: App, info: &RunnerInfo) -> Applicability {
    if !present(app) {
        return Applicability::NotApplicable(vec![Reason::NotPresentOnBranch]);
    }
    let mut reasons = Vec::new();
    if let Apps::Named(named) = &storyboard.apps
        && !named.contains(&app)
    {
        reasons.push(Reason::NotNamed);
    }
    for command in commands_needed(storyboard, app) {
        if !provides(app, command) {
            reasons.push(Reason::MissingCommand(command.as_str().to_owned()));
        }
    }
    if !info.seeds.iter().any(|seed| seed == storyboard.seed()) {
        reasons.push(Reason::MissingSeed(storyboard.seed().to_owned()));
    }
    if let Some(preset) = &storyboard.preset
        && !info.presets.contains(preset)
    {
        reasons.push(Reason::MissingPreset(preset.clone()));
    }
    if reasons.is_empty() {
        Applicability::Applies
    } else {
        Applicability::NotApplicable(reasons)
    }
}

/// Commands an explicitly named, present app does not provide: `(app,
/// command id)`. Naming apps turns each into a load error, because the author
/// asked for that app and the storyboard cannot be played on it. Apps absent
/// from this branch are not judged; there is no registry to ask.
pub fn unprovided_for_named_apps(storyboard: &Storyboard) -> Vec<(App, String)> {
    let Apps::Named(named) = &storyboard.apps else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for app in named.iter().copied().filter(|app| present(*app)) {
        for command in commands_needed(storyboard, app) {
            if !provides(app, command) {
                out.push((app, command.as_str().to_owned()));
            }
        }
    }
    out
}

/// The commands `app` would have to press, once: those of steps it does not
/// skip. A command that is not an id is the lint's to report, not this
/// module's.
fn commands_needed(storyboard: &Storyboard, app: App) -> Vec<CommandId> {
    let skipped = |number: usize, id: Option<&str>| {
        storyboard.overrides.get(&app).is_some_and(|steps| {
            [Some(number.to_string()), id.map(str::to_owned)]
                .into_iter()
                .flatten()
                .any(|reference| steps.get(&reference).is_some_and(|o| o.skip.is_some()))
        })
    };
    let mut out = Vec::new();
    for (index, step) in storyboard.steps.iter().enumerate() {
        if skipped(index + 1, step.id.as_deref()) {
            continue;
        }
        if let Input::Command(id) = &step.input
            && let Ok(command) = id.parse::<CommandId>()
            && !out.contains(&command)
        {
            out.push(command);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use postio_core::registry::Requirement;

    use super::*;
    use crate::format::parse;

    const HEAD: &str = "source = { kind = \"flow\", ref = \"x\" }\n";

    fn board(extra_head: &str, steps: &[&str]) -> Result<Storyboard, crate::format::LoadError> {
        let mut text = format!("{extra_head}{HEAD}");
        for step in steps {
            text.push_str(&format!("[[step]]\ncommand = \"{step}\"\n"));
        }
        parse(&text, Path::new("t.toml"))
    }

    fn info(app: App) -> RunnerInfo {
        RunnerInfo {
            app,
            seeds: vec!["small".into(), "thirty-threads".into()],
            presets: vec!["settings/account-form".into()],
            axes: BTreeMap::new(),
        }
    }

    fn terminal_only() -> &'static str {
        registry::all()
            .find(|spec| spec.requires.contains(Requirement::Terminal))
            .expect("the registry has a terminal-only command")
            .id
            .as_str()
    }

    #[test]
    fn provides_follows_the_registrys_requirements() {
        for spec in registry::all() {
            let terminal = spec.requires.contains(Requirement::Terminal);
            let graphical = spec.requires.contains(Requirement::Graphical);
            assert_eq!(
                provides(App::Classic, spec.id),
                spec.requires.offered_by(Frontend::Classic),
                "{}",
                spec.id.as_str()
            );
            assert_eq!(
                provides(App::Focus, spec.id),
                spec.requires.offered_by(Frontend::Focus),
                "{}",
                spec.id.as_str()
            );
            assert_eq!(
                provides(App::Terminal, spec.id),
                spec.requires.offered_by(Frontend::Terminal),
                "{}",
                spec.id.as_str()
            );
            // The registry's own answer never contradicts the coarse one.
            if terminal {
                assert!(!provides(App::Classic, spec.id));
            }
            if graphical {
                assert!(!provides(App::Terminal, spec.id));
            }
            assert!(!provides(App::Macos, spec.id));
        }
    }

    #[test]
    fn focus_provides_what_only_focus_has_and_not_the_flag() {
        assert!(provides(App::Focus, CommandId::ToggleHasAction));
        assert!(!provides(App::Classic, CommandId::ToggleHasAction));
        assert!(!provides(App::Focus, CommandId::Flag));
        assert!(provides(App::Classic, CommandId::Flag));
    }

    #[test]
    fn shared_commands_apply_to_classic_and_focus() {
        let board = board("", &["next_message", "back"]).unwrap();
        assert_eq!(
            applies(&board, App::Classic, &info(App::Classic)),
            Applicability::Applies
        );
        assert_eq!(
            applies(&board, App::Focus, &info(App::Focus)),
            Applicability::Applies
        );
        // Macos is the one still not built.
        let macos = applies(&board, App::Macos, &info(App::Macos));
        let Applicability::NotApplicable(reasons) = macos else {
            panic!("macos is not present on this branch: {macos:?}");
        };
        assert_eq!(reasons, vec![Reason::NotPresentOnBranch]);
        assert_eq!(reasons[0].to_string(), "not present on this branch");
    }

    #[test]
    fn a_terminal_command_does_not_apply_to_classic_and_is_named() {
        let command = terminal_only();
        let board = board("", &["next_message", command]).unwrap();
        assert_eq!(
            applies(&board, App::Classic, &info(App::Classic)),
            Applicability::NotApplicable(vec![Reason::MissingCommand(command.into())])
        );
        assert_eq!(
            applies(&board, App::Terminal, &info(App::Terminal)),
            Applicability::Applies
        );
    }

    #[test]
    fn naming_an_app_that_lacks_a_command_is_a_load_error() {
        let command = terminal_only();
        let error = board("apps = [\"classic\"]\n", &[command])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(command) && error.contains("classic"),
            "{error}"
        );
        // Naming an app that is not built here cannot be checked, so loads.
        assert!(board("apps = [\"macos\"]\n", &[command]).is_ok());
        // A step the app skips does not need the command.
        let text = format!(
            "apps = [\"classic\"]\n{HEAD}[[step]]\ncommand = \"{command}\"\n[app.classic.step.1]\nskip = {{ reason = \"terminal only\" }}\n"
        );
        assert!(parse(&text, Path::new("t.toml")).is_ok());
    }

    #[test]
    fn a_storyboard_that_does_not_name_the_app_does_not_apply() {
        let board = board("apps = [\"terminal\"]\n", &["back"]).unwrap();
        assert_eq!(
            applies(&board, App::Classic, &info(App::Classic)),
            Applicability::NotApplicable(vec![Reason::NotNamed])
        );
    }

    #[test]
    fn a_seed_or_preset_the_app_does_not_declare_means_not_applicable() {
        let board = board(
            "seed = \"huge\"\npreset = \"add-account/browser\"\n",
            &["back"],
        )
        .unwrap();
        assert_eq!(
            applies(&board, App::Classic, &info(App::Classic)),
            Applicability::NotApplicable(vec![
                Reason::MissingSeed("huge".into()),
                Reason::MissingPreset("add-account/browser".into()),
            ])
        );
        let known = self::board(
            "seed = \"thirty-threads\"\npreset = \"settings/account-form\"\n",
            &["back"],
        )
        .unwrap();
        assert_eq!(
            applies(&known, App::Classic, &info(App::Classic)),
            Applicability::Applies
        );
        // The default seed is `small`.
        let plain = self::board("", &["back"]).unwrap();
        let mut bare = info(App::Classic);
        bare.seeds.clear();
        assert_eq!(
            applies(&plain, App::Classic, &bare),
            Applicability::NotApplicable(vec![Reason::MissingSeed("small".into())])
        );
    }
}
