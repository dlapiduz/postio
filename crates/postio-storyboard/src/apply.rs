//! Which apps a storyboard applies to.
//!
//! Applicability is static (research R7): the loader and the lint can answer
//! it with no runner built, because it is a question about the command
//! registry and about what each runner declares, not about a run.

use std::collections::BTreeMap;

use postio_core::CommandId;
use postio_core::registry::{self, Requirement};
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

/// Whether `app` exists on this branch at all. Focus and macOS are built on
/// other branches; here they are named in the vocabulary and nothing more.
/// The Focus lane changes this, and `provides`, where its code lands
/// (research R7).
pub fn present(app: App) -> bool {
    matches!(app, App::Classic | App::Terminal)
}

/// Whether `app` offers `command`, by what the registry says it requires: the
/// desktop does not offer what only the terminal's composer has, and the
/// terminal does not offer what is about drawing pixels. A command the
/// registry offers but the app never wired still counts as provided; that gap
/// is what the generated pass is for.
pub fn provides(app: App, command: CommandId) -> bool {
    let requires = registry::get(command).requires;
    match app {
        App::Classic => !requires.contains(Requirement::Terminal),
        App::Terminal => !requires.contains(Requirement::Graphical),
        App::Focus | App::Macos => false,
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

/// Every variant a storyboard asks this app for: the cross product of the
/// values it lists on each axis the app supports, plus the axes it asked
/// for that the app does not have (FR-017). A storyboard that varies nothing
/// has one variant, the default, which is empty.
pub fn variants(
    storyboard: &Storyboard,
    info: &RunnerInfo,
) -> (Vec<BTreeMap<String, String>>, Vec<String>) {
    let mut ignored = Vec::new();
    let mut all: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    for (axis, asked) in &storyboard.vary {
        let Some(supported) = info.axes.get(axis) else {
            ignored.push(axis.clone());
            continue;
        };
        let values: Vec<&String> = asked.iter().filter(|v| supported.contains(v)).collect();
        if values.is_empty() {
            ignored.push(axis.clone());
            continue;
        }
        all = all
            .into_iter()
            .flat_map(|variant| {
                values.iter().map(move |value| {
                    let mut next = variant.clone();
                    next.insert(axis.clone(), (*value).clone());
                    next
                })
            })
            .collect();
    }
    all.sort_by_key(crate::run::variant_key);
    (all, ignored)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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
                !terminal,
                "{}",
                spec.id.as_str()
            );
            assert_eq!(
                provides(App::Terminal, spec.id),
                !graphical,
                "{}",
                spec.id.as_str()
            );
            assert!(!provides(App::Focus, spec.id));
            assert!(!provides(App::Macos, spec.id));
        }
    }

    #[test]
    fn shared_commands_apply_to_classic_and_focus_is_not_on_this_branch() {
        let board = board("", &["next_message", "back"]).unwrap();
        assert_eq!(
            applies(&board, App::Classic, &info(App::Classic)),
            Applicability::Applies
        );
        let focus = applies(&board, App::Focus, &info(App::Focus));
        let Applicability::NotApplicable(reasons) = focus else {
            panic!("focus is not present on this branch: {focus:?}");
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
        assert!(board("apps = [\"focus\"]\n", &[command]).is_ok());
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

    #[test]
    fn variants_are_the_cross_product_of_what_is_asked_and_supported() {
        let board = parse(
            "source = { kind = \"flow\", ref = \"x\" }\n\
             vary = { scheme = [\"light\", \"dark\"], width = [\"wide\", \"narrow\"], density = [\"compact\"] }\n",
            Path::new("list/walk.toml"),
        )
        .expect("loads");
        let mut info = info(App::Classic);
        info.axes = [
            (
                "scheme".to_owned(),
                vec!["light".to_owned(), "dark".to_owned()],
            ),
            (
                "width".to_owned(),
                vec!["wide".to_owned(), "normal".to_owned(), "narrow".to_owned()],
            ),
        ]
        .into_iter()
        .collect();
        let (all, ignored) = variants(&board, &info);
        let keys: Vec<String> = all.iter().map(crate::run::variant_key).collect();
        assert_eq!(
            keys,
            [
                "scheme=dark,width=narrow",
                "scheme=dark,width=wide",
                "scheme=light,width=narrow",
                "scheme=light,width=wide"
            ]
        );
        assert_eq!(
            ignored,
            ["density"],
            "an axis the app lacks is said, not played"
        );
    }

    #[test]
    fn an_unsupported_value_is_left_out_and_nothing_asked_is_the_default() {
        let board = parse(
            "source = { kind = \"flow\", ref = \"x\" }\nvary = { scheme = [\"dark\", \"sepia\"] }\n",
            Path::new("list/walk.toml"),
        )
        .expect("loads");
        let mut info = info(App::Classic);
        info.axes = [(
            "scheme".to_owned(),
            vec!["light".to_owned(), "dark".to_owned()],
        )]
        .into_iter()
        .collect();
        let (all, _) = variants(&board, &info);
        assert_eq!(
            all.iter().map(crate::run::variant_key).collect::<Vec<_>>(),
            ["scheme=dark"]
        );
        let plain = parse(
            "source = { kind = \"flow\", ref = \"x\" }\n",
            Path::new("list/walk.toml"),
        )
        .expect("loads");
        let (all, ignored) = variants(&plain, &info);
        assert_eq!(all, [BTreeMap::new()]);
        assert!(ignored.is_empty());
    }
}
