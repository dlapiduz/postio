//! The storyboard file: its types and its loader.
//!
//! The grammar is `specs/008-storyboards/contracts/storyboard-format.md`, and
//! this module is the one place that reads it, so no runner can come to
//! disagree with another about what a file means. It parses and shapes; it
//! does not judge. Whether a command id exists, a chord parses or an address
//! is reserved is the lint's question (`lint.rs`), because those need the
//! registry and the keymap and a file that merely says something wrong is
//! still a file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::apply::{App, unprovided_for_named_apps};

/// Why a storyboard file did not load.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("{path}: {message}")]
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        message: String,
    },
    /// The file is not a storyboard.
    #[error("{path}: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What was wrong with it.
        message: String,
    },
    /// A named app lacks a command the storyboard presses.
    #[error("{path}: `apps` names {app}, which does not provide `{command}`")]
    Unprovided {
        /// The file.
        path: PathBuf,
        /// The app.
        app: App,
        /// The command id.
        command: String,
    },
}

/// One interaction.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Storyboard {
    /// The storyboard's name: the file stem unless the file says otherwise.
    pub name: String,
    /// Why this storyboard exists. Required, and the lint says so; the loader
    /// keeps it optional so a missing one is a named error and not a parse
    /// failure.
    pub source: Option<Source>,
    /// Where the red evidence for an issue's storyboard comes from.
    pub proof: Option<Proof>,
    /// The seed, as the file spelled it. See [`Storyboard::seed`].
    pub seed: Option<String>,
    /// A window condition the app names.
    pub preset: Option<String>,
    /// Which apps it is played against.
    pub apps: Apps,
    /// Variant axes and the values asked for on each.
    pub vary: BTreeMap<String, Vec<String>>,
    /// A canvas screen id.
    pub design: Option<String>,
    /// Whether the whole storyboard needs real routing.
    pub routing: Routing,
    /// A calibration storyboard's expected verdict.
    pub calibration: Option<Calibration>,
    /// The steps. None is a screen.
    pub steps: Vec<Step>,
    /// Per-app overrides, keyed by app and then by step id or index.
    pub overrides: BTreeMap<App, BTreeMap<String, StepOverride>>,
    /// The file it was read from. Empty for text that did not come from one.
    #[serde(skip)]
    pub path: PathBuf,
}

/// Why a storyboard exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// What kind of thing `reference` points at.
    pub kind: SourceKind,
    /// The thing itself: `#1687`, a commit, a canvas screen.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// What a [`Source`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// An issue.
    Issue,
    /// A commit.
    Commit,
    /// A spec requirement.
    Spec,
    /// A canvas screen.
    Design,
    /// An end-to-end flow.
    Flow,
    /// The maintainer's own words.
    Maintainer,
}

/// Where an issue storyboard's red evidence comes from (research R0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proof {
    /// Against the base's code.
    Base,
    /// The defect is still open.
    Open,
    /// A pinned earlier commit.
    Pinned,
}

/// Which apps a storyboard names.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Apps {
    /// Every app that provides what it uses.
    #[default]
    Auto,
    /// These apps, and no others.
    Named(Vec<App>),
}

/// Whether routing is chained or real.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    /// The runner's chain of responders.
    #[default]
    Chain,
    /// The real window's routing.
    Real,
}

/// A calibration storyboard's expected verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Calibration {
    /// A known-true storyboard.
    MustPass,
    /// A known-false storyboard.
    MustFail,
}

/// One step: exactly one input, and what to expect after it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    /// For overrides and `same_as`. Absent means the 1-based index.
    pub id: Option<String>,
    /// What the step does.
    pub input: Input,
    /// Partial checks on the observation after the step.
    pub check: Checks,
    /// Prose for the reviewer.
    pub expect: Option<String>,
    /// A canvas screen id that overrides the storyboard's.
    pub design: Option<String>,
    /// How to wait for the app to settle.
    pub settle: Option<Settle>,
    /// Per-step routing.
    pub routing: Option<Routing>,
}

/// The one thing a step does.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// Press the app's binding for a command, by its string id.
    Command(String),
    /// A raw chord.
    Key(String),
    /// Text for the widget that holds the keyboard.
    Type(String),
    /// Wait.
    Wait(Wait),
    /// An environment change.
    Event(EnvEvent),
}

/// What a wait is for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wait {
    /// A fixed time.
    Ms(u64),
    /// Until the checks hold.
    Until(Checks),
}

/// An environment change a step can cause (research R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvEvent {
    /// A message arrives.
    NewMail,
    /// The mailbox list changes.
    MailboxesChanged,
    /// The connection drops.
    ConnectionLost,
    /// The connection returns.
    ConnectionRestored,
    /// A backfill makes progress.
    BackfillProgress,
    /// A body that was missing arrives.
    BodyArrived,
}

/// How a step waits for the app to settle (research R4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settle {
    /// Settled when these checks hold.
    pub until: Option<Checks>,
    /// Give up after this long.
    pub max_ms: Option<u64>,
    /// Keep watching this long after settling, to catch a late change.
    pub watch_ms: Option<u64>,
}

/// Checks: nested tables shaped like the observation, ending in [`Leaf`]s.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(transparent)]
pub struct Checks(pub BTreeMap<String, Check>);

/// A node in a [`Checks`] tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Check {
    /// A field with more fields under it.
    Table(Checks),
    /// What one field must be.
    Leaf(Leaf),
}

/// What one observed field must be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Leaf {
    /// Equals this.
    Literal(Value),
    /// Equals the same field at an earlier step.
    SameAs(StepRef),
    /// Differs from the previous step.
    Changed,
    /// Equals the previous step.
    Unchanged,
    /// Is `None`.
    Absent,
    /// Equals any of these.
    OneOf(Vec<Value>),
}

/// A step named by its id or by its 1-based index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepRef {
    /// A step id.
    Id(String),
    /// A 1-based index.
    Index(u64),
}

/// What one app does differently at one step.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOverride {
    /// Replaces the step's checks.
    pub check: Option<Checks>,
    /// Replaces the step's prose.
    pub expect: Option<String>,
    /// The step does not exist in this app.
    pub skip: Option<Skip>,
}

/// Why a step does not exist in an app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skip {
    /// The reason, shown with the skipped step.
    pub reason: String,
}

impl Checks {
    /// The node at a dotted path, such as `keyboard.region`.
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

    /// Every leaf with its dotted path, in path order.
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
    /// The seed, `small` unless the file names another.
    pub fn seed(&self) -> &str {
        self.seed.as_deref().unwrap_or("small")
    }
}

/// Read and parse one storyboard file.
pub fn load(path: &Path) -> Result<Storyboard, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|error| LoadError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    parse(&text, path)
}

/// Parse storyboard text. `path` names it in errors and supplies the default
/// name, its file stem.
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

/// The file as TOML shapes it. `[[step]]` and `[app.<app>.step.<ref>]` are
/// the file's spelling; [`Storyboard`] names them for what they hold.
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

/// An axis value as written: `"dark"`, or a bare `200` for the text axis.
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

/// The inputs a step may carry, for the message that says it took the wrong
/// number of them.
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

/// The table keys that make a table a leaf. Nothing else is special.
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
    fn event_accepts_only_the_six_names() {
        for name in [
            "new_mail",
            "mailboxes_changed",
            "connection_lost",
            "connection_restored",
            "backfill_progress",
            "body_arrived",
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

    /// The contract's grammar block, verbatim.
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
