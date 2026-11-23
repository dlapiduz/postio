//! Which apps a storyboard applies to. Static (research R7): answerable with no runner built, from
//! the command registry and what each runner declares.

use std::collections::BTreeMap;

use postio_core::CommandId;
use postio_core::registry::{self, Frontend};
use serde::{Deserialize, Serialize};

use crate::format::{Apps, Input, Storyboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    Focus,
    Terminal,
    Macos,
}

impl std::fmt::Display for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            App::Focus => "focus",
            App::Terminal => "terminal",
            App::Macos => "macos",
        })
    }
}

/// macOS is built elsewhere and is named in the vocabulary and nothing more.
pub fn present(app: App) -> bool {
    matches!(app, App::Focus | App::Terminal)
}

/// By the registry's `requires.offered_by`: a command the registry offers but the app never wired
/// still counts as provided; that gap is what the generated pass is for.
pub fn provides(app: App, command: CommandId) -> bool {
    let requires = registry::get(command).requires;
    match app {
        App::Focus => requires.offered_by(Frontend::Focus),
        App::Terminal => requires.offered_by(Frontend::Terminal),
        App::Macos => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerInfo {
    pub app: App,
    pub seeds: Vec<String>,
    pub presets: Vec<String>,
    pub axes: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    Applies,
    NotApplicable(Vec<Reason>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Reason {
    #[error("not present on this branch")]
    NotPresentOnBranch,
    #[error("the storyboard does not name this app")]
    NotNamed,
    #[error("the app does not provide `{0}`")]
    MissingCommand(String),
    #[error("the app cannot build seed `{0}`")]
    MissingSeed(String),
    #[error("the app does not declare preset `{0}`")]
    MissingPreset(String),
}

/// Every reason is listed, not just the first, so one lint run says everything standing between a
/// storyboard and an app.
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

/// Naming apps turns each into a load error, because the author asked for that app. Apps absent
/// from this branch are not judged: there is no registry to ask.
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

/// An axis whose asked values include its default keeps it; one asking only for others
/// (`inbox-dark` asks `scheme = ["dark"]`) takes the first of them, so a storyboard is never filmed
/// in a variant it did not ask for.
pub fn default_variant(
    storyboard: &Storyboard,
    info: &RunnerInfo,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut variant = BTreeMap::new();
    let mut ignored = Vec::new();
    for (axis, asked) in &storyboard.vary {
        let Some(supported) = info.axes.get(axis) else {
            ignored.push(axis.clone());
            continue;
        };
        if supported
            .first()
            .is_some_and(|default| asked.contains(default))
        {
            continue;
        }
        match asked.iter().find(|value| supported.contains(value)) {
            Some(value) => {
                variant.insert(axis.clone(), value.clone());
            }
            None => ignored.push(axis.clone()),
        }
    }
    (variant, ignored)
}

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
            if terminal {
                assert!(!provides(App::Focus, spec.id));
            }
            if graphical {
                assert!(!provides(App::Terminal, spec.id));
            }
            assert!(!provides(App::Macos, spec.id));
        }
    }

    #[test]
    fn focus_provides_what_only_focus_has() {
        assert!(provides(App::Focus, CommandId::ToggleHasAction));
        assert!(provides(App::Focus, CommandId::Flag));
    }

    #[test]
    fn shared_commands_apply_to_focus() {
        let board = board("", &["next_message", "back"]).unwrap();
        assert_eq!(
            applies(&board, App::Focus, &info(App::Focus)),
            Applicability::Applies
        );
        let macos = applies(&board, App::Macos, &info(App::Macos));
        let Applicability::NotApplicable(reasons) = macos else {
            panic!("macos is not present on this branch: {macos:?}");
        };
        assert_eq!(reasons, vec![Reason::NotPresentOnBranch]);
        assert_eq!(reasons[0].to_string(), "not present on this branch");
    }

    #[test]
    fn a_terminal_command_does_not_apply_to_focus_and_is_named() {
        let command = terminal_only();
        let board = board("", &["next_message", command]).unwrap();
        assert_eq!(
            applies(&board, App::Focus, &info(App::Focus)),
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
        let error = board("apps = [\"focus\"]\n", &[command])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(command) && error.contains("focus"),
            "{error}"
        );
        assert!(board("apps = [\"macos\"]\n", &[command]).is_ok());
        let text = format!(
            "apps = [\"focus\"]\n{HEAD}[[step]]\ncommand = \"{command}\"\n[app.focus.step.1]\nskip = {{ reason = \"terminal only\" }}\n"
        );
        assert!(parse(&text, Path::new("t.toml")).is_ok());
    }

    #[test]
    fn a_storyboard_that_does_not_name_the_app_does_not_apply() {
        let board = board("apps = [\"terminal\"]\n", &["back"]).unwrap();
        assert_eq!(
            applies(&board, App::Focus, &info(App::Focus)),
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
            applies(&board, App::Focus, &info(App::Focus)),
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
            applies(&known, App::Focus, &info(App::Focus)),
            Applicability::Applies
        );
        let plain = self::board("", &["back"]).unwrap();
        let mut bare = info(App::Focus);
        bare.seeds.clear();
        assert_eq!(
            applies(&plain, App::Focus, &bare),
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
        let mut info = info(App::Focus);
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
    fn the_default_variant_keeps_an_app_default_and_takes_what_is_asked_otherwise() {
        let board = parse(
            "source = { kind = \"flow\", ref = \"x\" }\n\
             vary = { scheme = [\"dark\"], width = [\"wide\", \"normal\"], density = [\"compact\"] }\n",
            Path::new("screens/inbox-dark.toml"),
        )
        .expect("loads");
        let mut info = info(App::Focus);
        info.axes = [
            (
                "scheme".to_owned(),
                vec!["light".to_owned(), "dark".to_owned()],
            ),
            (
                "width".to_owned(),
                vec!["normal".to_owned(), "wide".to_owned()],
            ),
        ]
        .into_iter()
        .collect();
        let (variant, ignored) = default_variant(&board, &info);
        assert_eq!(
            crate::run::variant_key(&variant),
            "scheme=dark",
            "a storyboard that asks only for dark is dark; one that allows the default width keeps it"
        );
        assert_eq!(ignored, ["density"]);
        let plain = parse(
            "source = { kind = \"flow\", ref = \"x\" }\n",
            Path::new("list/walk.toml"),
        )
        .expect("loads");
        assert!(default_variant(&plain, &info).0.is_empty());
    }

    #[test]
    fn an_unsupported_value_is_left_out_and_nothing_asked_is_the_default() {
        let board = parse(
            "source = { kind = \"flow\", ref = \"x\" }\nvary = { scheme = [\"dark\", \"sepia\"] }\n",
            Path::new("list/walk.toml"),
        )
        .expect("loads");
        let mut info = info(App::Focus);
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
