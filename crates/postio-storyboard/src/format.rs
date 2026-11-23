//! The storyboard file: its types and its loader, the one place that reads the grammar
//! (`contracts/storyboard-format.md`) so runners cannot disagree. It parses and shapes but does not
//! judge: whether a command id exists or a chord parses is the lint's question, because those need
//! the registry and keymap.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::apply::{App, unprovided_for_named_apps};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path}: `apps` names {app}, which does not provide `{command}`")]
    Unprovided {
        path: PathBuf,
        app: App,
        command: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Storyboard {
    pub name: String,
    /// Required, and the lint says so; the loader keeps it optional so a missing one is a named
    /// error and not a parse failure.
    pub source: Option<Source>,
    pub proof: Option<Proof>,
    pub seed: Option<String>,
    pub preset: Option<String>,
    pub apps: Apps,
    pub vary: BTreeMap<String, Vec<String>>,
    pub design: Option<String>,
    pub routing: Routing,
    pub calibration: Option<Calibration>,
    pub steps: Vec<Step>,
    pub overrides: BTreeMap<App, BTreeMap<String, StepOverride>>,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Issue,
    Commit,
    Spec,
    Design,
    Flow,
    Maintainer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proof {
    Base,
    Open,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Apps {
    #[default]
    Auto,
    Named(Vec<App>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    #[default]
    Chain,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Calibration {
    MustPass,
    MustFail,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub id: Option<String>,
    pub input: Input,
    pub check: Checks,
    pub expect: Option<String>,
    pub design: Option<String>,
    pub settle: Option<Settle>,
    pub routing: Option<Routing>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Command(String),
    Key(String),
    Type(String),
    Wait(Wait),
    Event(EnvEvent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wait {
    Ms(u64),
    Until(Checks),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvEvent {
    NewMail,
    MailboxesChanged,
    ConnectionLost,
    ConnectionRestored,
    BackfillProgress,
    BodyArrived,
    Reply,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settle {
    pub until: Option<Checks>,
    pub max_ms: Option<u64>,
    pub watch_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(transparent)]
pub struct Checks(pub BTreeMap<String, Check>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Check {
    Table(Checks),
    Leaf(Leaf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Leaf {
    Literal(Value),
    SameAs(StepRef),
    Changed,
    Unchanged,
    Absent,
    OneOf(Vec<Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepRef {
    Id(String),
    Index(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOverride {
    pub check: Option<Checks>,
    pub expect: Option<String>,
    pub skip: Option<Skip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skip {
    pub reason: String,
}

impl Checks {
    pub fn get(&self, path: &str) -> Option<&Check> {
        let mut parts = path.split('.');
        let mut node = self.0.get(parts.next()?)?;
        for part in parts {
            match node {
                Check::Table(inner) => node = inner.0.get(part)?,
                Check::Leaf(_) => return None,
            }
        }
        Some(node)
    }

    pub fn leaves(&self) -> Vec<(String, &Leaf)> {
        let mut out = Vec::new();
        self.collect("", &mut out);
        out
    }

    fn collect<'a>(&'a self, prefix: &str, out: &mut Vec<(String, &'a Leaf)>) {
        for (key, node) in &self.0 {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match node {
                Check::Leaf(leaf) => out.push((path, leaf)),
                Check::Table(inner) => inner.collect(&path, out),
            }
        }
    }
}

impl Storyboard {
    pub fn seed(&self) -> &str {
        self.seed.as_deref().unwrap_or("small")
    }
}

pub fn load(path: &Path) -> Result<Storyboard, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|error| LoadError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    parse(&text, path)
}

pub fn parse(text: &str, path: &Path) -> Result<Storyboard, LoadError> {
    let raw: RawStoryboard = toml::from_str(text).map_err(|error| LoadError::Parse {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let board = Storyboard {
        name: raw.name.unwrap_or(stem),
        source: raw.source,
        proof: raw.proof,
        seed: raw.seed,
        preset: raw.preset,
        apps: raw.apps.unwrap_or_default(),
        vary: raw
            .vary
            .into_iter()
            .map(|(axis, values)| (axis, values.into_iter().map(|v| v.0).collect()))
            .collect(),
        design: raw.design,
        routing: raw.routing.unwrap_or_default(),
        calibration: raw.calibration,
        steps: raw.step,
        overrides: raw
            .app
            .into_iter()
            .map(|(app, section)| (app, section.step))
            .collect(),
        path: path.to_owned(),
    };
    // Naming an app is a promise it can play the storyboard; `auto` makes no
    // promise and is only filtered (rule 8).
    match unprovided_for_named_apps(&board).into_iter().next() {
        Some((app, command)) => Err(LoadError::Unprovided {
            path: path.to_owned(),
            app,
            command,
        }),
        None => Ok(board),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStoryboard {
    name: Option<String>,
    source: Option<Source>,
    proof: Option<Proof>,
    seed: Option<String>,
    preset: Option<String>,
    apps: Option<Apps>,
    #[serde(default)]
    vary: BTreeMap<String, Vec<AxisValue>>,
    design: Option<String>,
    routing: Option<Routing>,
    calibration: Option<Calibration>,
    #[serde(default)]
    step: Vec<Step>,
    #[serde(default)]
    app: BTreeMap<App, RawAppSection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAppSection {
    #[serde(default)]
    step: BTreeMap<String, StepOverride>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AxisValueRaw {
    Text(String),
    Number(i64),
}

struct AxisValue(String);

impl<'de> Deserialize<'de> for AxisValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(match AxisValueRaw::deserialize(deserializer)? {
            AxisValueRaw::Text(text) => text,
            AxisValueRaw::Number(number) => number.to_string(),
        }))
    }
}

impl<'de> Deserialize<'de> for Apps {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(text) if text == "auto" => Ok(Apps::Auto),
            list @ Value::Array(_) => serde_json::from_value(list)
                .map(Apps::Named)
                .map_err(D::Error::custom),
            other => Err(D::Error::custom(format!(
                "apps is \"auto\" or a list of apps, not {other}"
            ))),
        }
    }
}

const INPUTS: &str = "command, key, type, wait, event";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    id: Option<String>,
    command: Option<String>,
    key: Option<String>,
    #[serde(rename = "type")]
    typed: Option<String>,
    wait: Option<Wait>,
    event: Option<EnvEvent>,
    check: Option<Checks>,
    expect: Option<String>,
    design: Option<String>,
    settle: Option<Settle>,
    routing: Option<Routing>,
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawStep::deserialize(deserializer)?;
        let mut inputs = Vec::new();
        inputs.extend(raw.command.map(Input::Command));
        inputs.extend(raw.key.map(Input::Key));
        inputs.extend(raw.typed.map(Input::Type));
        inputs.extend(raw.wait.map(Input::Wait));
        inputs.extend(raw.event.map(Input::Event));
        let found = inputs.len();
        let Some(input) = inputs.pop().filter(|_| found == 1) else {
            return Err(D::Error::custom(format!(
                "a step takes exactly one input of {INPUTS}, and this one has {found}"
            )));
        };
        Ok(Step {
            id: raw.id,
            input,
            check: raw.check.unwrap_or_default(),
            expect: raw.expect,
            design: raw.design,
            settle: raw.settle,
            routing: raw.routing,
        })
    }
}

const SENTINELS: [&str; 5] = ["same_as", "changed", "unchanged", "absent", "one_of"];

impl<'de> Deserialize<'de> for Checks {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        table_of(map).map_err(D::Error::custom)
    }
}

fn table_of(map: impl IntoIterator<Item = (String, Value)>) -> Result<Checks, String> {
    let mut out = BTreeMap::new();
    for (key, value) in map {
        let node = check_of(value).map_err(|message| format!("check `{key}`: {message}"))?;
        out.insert(key, node);
    }
    Ok(Checks(out))
}

fn check_of(value: Value) -> Result<Check, String> {
    match value {
        Value::Object(map) => {
            if !map.keys().any(|key| SENTINELS.contains(&key.as_str())) {
                return table_of(map).map(Check::Table);
            }
            if map.len() != 1 {
                return Err(format!(
                    "a leaf takes exactly one of {}",
                    SENTINELS.join(", ")
                ));
            }
            let (key, value) = map.into_iter().next().expect("length is one");
            let flag = |leaf: Leaf| match value {
                Value::Bool(true) => Ok(Check::Leaf(leaf)),
                _ => Err(format!("`{key}` is written `{key} = true`")),
            };
            match key.as_str() {
                "changed" => flag(Leaf::Changed),
                "unchanged" => flag(Leaf::Unchanged),
                "absent" => flag(Leaf::Absent),
                "same_as" => serde_json::from_value(value)
                    .map(|step| Check::Leaf(Leaf::SameAs(step)))
                    .map_err(|_| "`same_as` names a step id or a 1-based index".to_owned()),
                _ => match value {
                    Value::Array(values) => Ok(Check::Leaf(Leaf::OneOf(values))),
                    _ => Err("`one_of` takes a list".to_owned()),
                },
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Number(_) => {
            Ok(Check::Leaf(Leaf::Literal(value)))
        }
        other => Err(format!("{other} is not a literal, a table or a sentinel")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Storyboard {
        parse(text, Path::new("t.toml")).unwrap_or_else(|e| panic!("should load: {e}"))
    }

    fn err(text: &str) -> String {
        match parse(text, Path::new("t.toml")) {
            Ok(_) => panic!("should not load:\n{text}"),
            Err(e) => e.to_string(),
        }
    }

    const HEAD: &str = "source = { kind = \"flow\", ref = \"x\" }\n";

    fn lit(v: impl Into<Value>) -> Check {
        Check::Leaf(Leaf::Literal(v.into()))
    }

    #[test]
    fn two_inputs_in_one_step_fail_to_load() {
        let message = err(&format!(
            "{HEAD}[[step]]\ncommand = \"archive\"\nkey = \"j\"\n"
        ));
        assert!(message.contains("one input"), "{message}");
        let message = err(&format!(
            "{HEAD}[[step]]\ntype = \"a\"\nevent = \"new_mail\"\n"
        ));
        assert!(message.contains("one input"), "{message}");
    }

    #[test]
    fn a_step_with_no_input_fails_to_load() {
        let message = err(&format!("{HEAD}[[step]]\nexpect = \"nothing\"\n"));
        assert!(message.contains("one input"), "{message}");
    }

    #[test]
    fn dotted_check_keys_nest() {
        let board = ok(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ keyboard.region = \"list\", cursor.index = 1 }}\n"
        ));
        let checks = &board.steps[0].check;
        assert_eq!(checks.get("keyboard.region"), Some(&lit("list")));
        assert_eq!(checks.get("cursor.index"), Some(&lit(1)));
        assert!(matches!(checks.get("keyboard"), Some(Check::Table(_))));
        assert_eq!(checks.leaves().len(), 2);
    }

    #[test]
    fn the_string_changed_is_a_literal() {
        let board = ok(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ notice.text = \"changed\" }}\n"
        ));
        assert_eq!(
            board.steps[0].check.get("notice.text"),
            Some(&lit("changed"))
        );
    }

    #[test]
    fn every_leaf_form_parses() {
        let board = ok(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ a = {{ changed = true }}, b = {{ unchanged = true }}, c = {{ absent = true }}, d = {{ one_of = [1, 2] }} }}\n"
        ));
        let c = &board.steps[0].check;
        assert_eq!(c.get("a"), Some(&Check::Leaf(Leaf::Changed)));
        assert_eq!(c.get("b"), Some(&Check::Leaf(Leaf::Unchanged)));
        assert_eq!(c.get("c"), Some(&Check::Leaf(Leaf::Absent)));
        assert_eq!(
            c.get("d"),
            Some(&Check::Leaf(Leaf::OneOf(vec![1.into(), 2.into()])))
        );
    }

    #[test]
    fn a_sentinel_must_be_the_only_key_and_true() {
        let message = err(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ a = {{ changed = false }} }}\n"
        ));
        assert!(message.contains("changed"), "{message}");
        let message = err(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ a = {{ changed = true, absent = true }} }}\n"
        ));
        assert!(message.contains("one of"), "{message}");
    }

    #[test]
    fn same_as_accepts_an_id_and_an_index() {
        let board = ok(&format!(
            "{HEAD}[[step]]\ncommand = \"back\"\ncheck = {{ a = {{ same_as = \"down\" }}, b = {{ same_as = 1 }} }}\n"
        ));
        let c = &board.steps[0].check;
        assert_eq!(
            c.get("a"),
            Some(&Check::Leaf(Leaf::SameAs(StepRef::Id("down".into()))))
        );
        assert_eq!(
            c.get("b"),
            Some(&Check::Leaf(Leaf::SameAs(StepRef::Index(1))))
        );
    }

    #[test]
    fn an_app_section_parses_into_overrides() {
        let board = ok(&format!(
            "{HEAD}[[step]]\nid = \"archive\"\ncommand = \"archive\"\n\
             [app.focus.step.archive]\nexpect = \"Dense rows collapse.\"\n\
             [app.focus.step.3]\nskip = {{ reason = \"no x mode\" }}\n\
             [app.focus.step.2]\ncheck = {{ cursor.index = 0 }}\n"
        ));
        let focus = &board.overrides[&App::Focus];
        assert_eq!(
            focus["archive"].expect.as_deref(),
            Some("Dense rows collapse.")
        );
        assert_eq!(focus["3"].skip.as_ref().unwrap().reason, "no x mode");
        assert_eq!(
            focus["2"].check.as_ref().unwrap().get("cursor.index"),
            Some(&lit(0))
        );
    }

    #[test]
    fn event_accepts_only_the_seven_names() {
        for name in [
            "new_mail",
            "mailboxes_changed",
            "connection_lost",
            "connection_restored",
            "backfill_progress",
            "body_arrived",
            "reply",
        ] {
            let board = ok(&format!("{HEAD}[[step]]\nevent = \"{name}\"\n"));
            assert!(matches!(board.steps[0].input, Input::Event(_)), "{name}");
        }
        let message = err(&format!("{HEAD}[[step]]\nevent = \"power_cut\"\n"));
        assert!(message.contains("power_cut"), "{message}");
    }

    #[test]
    fn the_name_defaults_to_the_file_stem() {
        let board = parse(HEAD, Path::new("storyboards/list/walk-down.toml")).unwrap();
        assert_eq!(board.name, "walk-down");
        let board = parse(&format!("name = \"other\"\n{HEAD}"), Path::new("walk.toml")).unwrap();
        assert_eq!(board.name, "other");
    }

    #[test]
    fn unknown_fields_are_refused() {
        let message = err(&format!("{HEAD}colour = \"red\"\n"));
        assert!(message.contains("colour"), "{message}");
    }

    const GRAMMAR: &str = r##"
name    = "archive-walks-down"
source  = { kind = "issue", ref = "#1687" }
proof   = "pinned"
seed    = "thirty-threads"
preset  = "settings/account-form"
apps    = ["terminal", "focus"]
vary    = { scheme = ["light", "dark"], width = ["wide", "narrow"] }
design  = "01-inbox-reading"
routing = "chain"

[[step]]
id      = "down"
command = "select_next"
check   = { keyboard.region = "list", cursor.index = 1 }

[[step]]
key     = "mod+z"

[[step]]
type    = "invoice"

[[step]]
wait    = { until = { notice.undo = true } }

[[step]]
event   = "new_mail"

[[step]]
command = "archive"
check   = { cursor.index = { same_as = "down" }, notice.undo = true,
            rows.first_visible = { unchanged = true } }
expect  = "The row is gone and the one below takes its place. Nothing scrolls."
settle  = { watch_ms = 500 }
design  = "06-mouse-parity-undo"
routing = "real"

[app.focus.step.archive]
expect = "The dense row collapses; the bulk bar does not appear."

[app.focus.step.3]
skip = { reason = "Focus has no `x` selection mode; it selects with space" }
"##;

    #[test]
    fn the_contracts_grammar_example_parses_to_the_expected_value() {
        let board = ok(GRAMMAR);
        assert_eq!(board.name, "archive-walks-down");
        assert_eq!(
            board.source,
            Some(Source {
                kind: SourceKind::Issue,
                reference: "#1687".into()
            })
        );
        assert_eq!(board.proof, Some(Proof::Pinned));
        assert_eq!(board.seed(), "thirty-threads");
        assert_eq!(board.preset.as_deref(), Some("settings/account-form"));
        assert_eq!(board.apps, Apps::Named(vec![App::Terminal, App::Focus]));
        assert_eq!(board.vary["scheme"], ["light", "dark"]);
        assert_eq!(board.vary["width"], ["wide", "narrow"]);
        assert_eq!(board.design.as_deref(), Some("01-inbox-reading"));
        assert_eq!(board.routing, Routing::Chain);
        assert_eq!(board.steps.len(), 6);

        let inputs: Vec<&Input> = board.steps.iter().map(|s| &s.input).collect();
        assert_eq!(inputs[0], &Input::Command("select_next".into()));
        assert_eq!(inputs[1], &Input::Key("mod+z".into()));
        assert_eq!(inputs[2], &Input::Type("invoice".into()));
        assert_eq!(inputs[4], &Input::Event(EnvEvent::NewMail));
        let Input::Wait(Wait::Until(until)) = inputs[3] else {
            panic!("fourth step waits until a check holds: {:?}", inputs[3]);
        };
        assert_eq!(until.get("notice.undo"), Some(&lit(true)));

        let last = &board.steps[5];
        assert_eq!(
            last.check.get("cursor.index"),
            Some(&Check::Leaf(Leaf::SameAs(StepRef::Id("down".into()))))
        );
        assert_eq!(
            last.check.get("rows.first_visible"),
            Some(&Check::Leaf(Leaf::Unchanged))
        );
        assert_eq!(
            last.settle,
            Some(Settle {
                until: None,
                max_ms: None,
                watch_ms: Some(500)
            })
        );
        assert_eq!(last.design.as_deref(), Some("06-mouse-parity-undo"));
        assert_eq!(last.routing, Some(Routing::Real));
        assert_eq!(board.steps[0].id.as_deref(), Some("down"));
        assert_eq!(board.overrides[&App::Focus].len(), 2);
    }

    #[test]
    fn apps_auto_is_the_default_and_a_spelling() {
        assert_eq!(ok(HEAD).apps, Apps::Auto);
        assert_eq!(ok(&format!("apps = \"auto\"\n{HEAD}")).apps, Apps::Auto);
        let message = err(&format!("apps = \"all\"\n{HEAD}"));
        assert!(message.contains("auto"), "{message}");
    }

    #[test]
    fn a_zero_step_storyboard_is_a_screen() {
        assert!(ok(HEAD).steps.is_empty());
    }
}
