//! The pickers' state and keys (terminal.md, "Pickers"): snooze, remind,
//! label and move, a framed box over the focused row.
//!
//! The words, the presets, what a typed date means and which labels count as
//! applied are `postio_ui::pickers` and `postio_ui::schedule`; this holds
//! the field, the chosen row and the keys, which come from the keymap's
//! `Picker` context. It does no I/O: a key returns a [`Step`], and `app.rs`
//! turns that into the command it sends.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use postio_core::{CommandId, Keymap};
use postio_model::{AccountId, Label, LabelId, Mailbox, MailboxId};
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::pickers as rules;
use tui_input::backend::crossterm::EventHandler;

use crate::app::Focus;
use crate::input::Keys;

/// Which picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `s`: mail leaves the inbox until a time.
    Snooze,
    /// `h`: bring the conversation back if nobody replies by a time.
    Remind,
    /// `l`: put labels on, or take them off.
    Label,
    /// `m`: move to a folder.
    Move,
}

/// What a choice asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// A time, for a snooze or a reminder.
    When(DateTime<Local>),
    /// A label put on or taken off; the picker stays open.
    Label {
        /// Which.
        label: LabelId,
        /// On, or off.
        on: bool,
    },
    /// A label nobody has yet: make it, then put it on.
    Create {
        /// Its name.
        name: String,
        /// Whether the picker closes once it is made.
        close: bool,
    },
    /// A folder to move to.
    Move(MailboxId),
}

/// What a key or a click asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Draw again.
    Stay,
    /// Put the picker away.
    Close,
    /// Do this, and put the picker away.
    Choose(Pick),
    /// Do this, and keep the picker open.
    Keep(Pick),
}

/// One row of the list, as it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The heading this row starts a section with.
    pub section: Option<&'static str>,
    /// Its number key, when it has one.
    pub number: Option<usize>,
    /// A label's name and stored colour, for its dot.
    pub dot: Option<(String, Option<String>)>,
    /// What it says.
    pub name: String,
    /// What it says at the right: a time, a count, "✓ applied".
    pub detail: String,
}

/// What a row stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Preset(usize),
    Create(String),
    Label(Label),
    Folder(MailboxId),
}

/// The picker, open.
#[derive(Debug)]
pub struct Picker {
    kind: Kind,
    /// Whether it chooses for the draft being written rather than for mail.
    draft: bool,
    target: String,
    from: Focus,
    now: DateTime<Local>,
    field: tui_input::Input,
    /// Whether the keyboard is in the field: the typed date after `Tab`, and
    /// the filter always.
    in_field: bool,
    selected: usize,
    presets: Vec<(&'static str, DateTime<Local>)>,
    account: Option<AccountId>,
    labels: Vec<Label>,
    counts: HashMap<LabelId, u32>,
    applied: HashSet<LabelId>,
    folders: Vec<Mailbox>,
    recent: Vec<MailboxId>,
}

impl Picker {
    fn new(kind: Kind, target: &str, from: Focus) -> Picker {
        Picker {
            kind,
            draft: false,
            target: target.to_owned(),
            from,
            now: Local::now(),
            field: tui_input::Input::default(),
            in_field: matches!(kind, Kind::Label | Kind::Move),
            selected: 0,
            presets: Vec::new(),
            account: None,
            labels: Vec::new(),
            counts: HashMap::new(),
            applied: HashSet::new(),
            folders: Vec::new(),
            recent: Vec::new(),
        }
    }

    /// A snooze or remind picker, with its presets as of `now`.
    pub fn when(kind: Kind, target: &str, from: Focus, now: DateTime<Local>) -> Picker {
        let mut picker = Picker::new(kind, target, from);
        picker.now = now;
        picker.presets = match kind {
            Kind::Remind => postio_ui::schedule::remind_presets(now),
            _ => postio_ui::schedule::snooze_presets(now),
        }
        .to_vec();
        picker
    }

    /// The remind picker for the draft being written.
    pub fn for_draft(target: &str, from: Focus, now: DateTime<Local>) -> Picker {
        let mut picker = Picker::when(Kind::Remind, target, from, now);
        picker.draft = true;
        picker
    }

    /// A label picker; its labels arrive when the host has read them.
    pub fn labelling(target: &str, from: Focus, account: AccountId) -> Picker {
        let mut picker = Picker::new(Kind::Label, target, from);
        picker.account = Some(account);
        picker
    }

    /// A move picker over `folders`; Recent arrives when the host has read it.
    pub fn moving(target: &str, from: Focus, mut folders: Vec<Mailbox>) -> Picker {
        folders.retain(rules::is_destination);
        rules::order_destinations(&mut folders);
        let mut picker = Picker::new(Kind::Move, target, from);
        picker.folders = folders;
        picker
    }

    /// Which picker.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Whether it chooses for the draft being written.
    pub fn is_for_draft(&self) -> bool {
        self.draft
    }

    /// What it acts on, as its title row names it.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Where the keyboard was when it opened.
    pub fn from(&self) -> Focus {
        self.from
    }

    /// What is typed in its field.
    pub fn typed(&self) -> &str {
        self.field.value()
    }

    /// Where the caret is, in characters.
    pub fn caret(&self) -> usize {
        self.field.cursor()
    }

    /// Whether the keyboard is in the field.
    pub fn in_field(&self) -> bool {
        self.in_field
    }

    /// The clock its presets were made at.
    pub fn now(&self) -> DateTime<Local> {
        self.now
    }

    /// The account whose labels it lists.
    pub fn account(&self) -> Option<AccountId> {
        self.account
    }

    /// The title.
    pub fn title(&self) -> &'static str {
        match self.kind {
            Kind::Snooze => rules::SNOOZE_TITLE,
            Kind::Remind => rules::REMIND_TITLE,
            Kind::Label => rules::LABEL_TITLE,
            Kind::Move => rules::MOVE_TITLE,
        }
    }

    /// What its footnote says.
    pub fn footnote(&self, keymap: &Keymap) -> String {
        match self.kind {
            Kind::Snooze => rules::snooze_footnote(keymap),
            Kind::Remind => rules::remind_footnote(self.now),
            Kind::Label => rules::label_footnote(keymap),
            Kind::Move => rules::move_footnote(keymap),
        }
    }

    /// What the field says while it is empty, or `None` for the date field,
    /// which is drawn as its own line.
    pub fn placeholder(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Label => Some(rules::LABEL_FILTER),
            Kind::Move => Some(rules::MOVE_FILTER),
            _ => None,
        }
    }

    /// What the line under the list says of what is typed: when the words
    /// land, or how to write a date.
    pub fn date_hint(&self) -> Option<String> {
        if !matches!(self.kind, Kind::Snooze | Kind::Remind) || self.field.value().trim().is_empty()
        {
            return None;
        }
        Some(match rules::typed(self.field.value(), self.now) {
            Some(at) => rules::when_label(at, self.now),
            None => "A day and a time: \u{201c}tue 9am\u{201d}".to_owned(),
        })
    }

    /// The labels, their counts and which the conversations carry arrived.
    pub fn learn_labels(
        &mut self,
        account: AccountId,
        labels: Vec<Label>,
        counts: Vec<(LabelId, u32)>,
        applied: HashSet<LabelId>,
    ) {
        self.account = Some(account);
        self.labels = labels;
        self.counts = counts.into_iter().collect();
        self.applied = applied;
    }

    /// The last destinations arrived.
    pub fn learn_recent(&mut self, recent: Vec<MailboxId>) {
        self.recent = recent;
    }

    /// A label the host made: listed, and applied.
    pub fn made(&mut self, label: Label, close: bool) {
        self.applied.insert(label.id);
        let labels = std::mem::take(&mut self.labels);
        self.labels = rules::with_label(labels, label);
        if !close {
            self.field = tui_input::Input::default();
            self.selected = 0;
        }
    }

    fn items(&self) -> Vec<(Item, Row)> {
        let filter = self.field.value();
        match self.kind {
            Kind::Snooze | Kind::Remind => self
                .presets
                .iter()
                .enumerate()
                .map(|(index, (name, at))| {
                    (
                        Item::Preset(index),
                        Row {
                            section: None,
                            number: Some(index + 1),
                            dot: None,
                            name: (*name).to_owned(),
                            detail: rules::when_label(*at, self.now),
                        },
                    )
                })
                .collect(),
            Kind::Label => rules::label_rows(&self.labels, filter)
                .into_iter()
                .map(|row| match row {
                    rules::LabelRow::Create(name) => (
                        Item::Create(name.clone()),
                        Row {
                            section: None,
                            number: None,
                            dot: None,
                            name: rules::create_label(&name),
                            detail: String::new(),
                        },
                    ),
                    rules::LabelRow::Label(label) => {
                        let detail = rules::label_detail(
                            self.applied.contains(&label.id),
                            self.counts.get(&label.id).copied().unwrap_or(0),
                        );
                        (
                            Item::Label(label.clone()),
                            Row {
                                section: None,
                                number: None,
                                dot: Some((label.name.clone(), label.color.clone())),
                                name: label.name,
                                detail,
                            },
                        )
                    }
                })
                .collect(),
            Kind::Move => {
                let mut number = 0;
                rules::move_rows(&self.folders, &self.recent, filter)
                    .into_iter()
                    .map(|row| {
                        let numbered = row.numbered.then(|| {
                            number += 1;
                            number
                        });
                        (
                            Item::Folder(row.folder),
                            Row {
                                section: row.section,
                                number: numbered,
                                dot: None,
                                name: row.name,
                                detail: row.count,
                            },
                        )
                    })
                    .collect()
            }
        }
    }

    /// The rows, as they are drawn.
    pub fn rows(&self) -> Vec<Row> {
        self.items().into_iter().map(|(_, row)| row).collect()
    }

    /// The chosen row.
    pub fn chosen(&self) -> usize {
        self.selected.min(self.items().len().saturating_sub(1))
    }

    fn pick(&self, item: &Item) -> Option<Pick> {
        match item {
            Item::Preset(index) => self.presets.get(*index).map(|(_, at)| Pick::When(*at)),
            Item::Create(name) => Some(Pick::Create {
                name: name.clone(),
                close: false,
            }),
            Item::Label(label) => Some(Pick::Label {
                label: label.id,
                on: !self.applied.contains(&label.id),
            }),
            Item::Folder(folder) => Some(Pick::Move(*folder)),
        }
    }

    /// Choose the `index`th row: what a click does, and what `Enter` does on
    /// the chosen one.
    pub fn choose(&mut self, index: usize) -> Step {
        let Some((item, _)) = self.items().into_iter().nth(index) else {
            return Step::Stay;
        };
        self.selected = index;
        match (&item, self.kind) {
            (Item::Label(_), _) => self.toggle(index),
            (Item::Create(name), _) => Step::Choose(Pick::Create {
                name: name.clone(),
                close: true,
            }),
            _ => self.pick(&item).map_or(Step::Stay, Step::Choose),
        }
    }

    /// Put the `index`th label on or take it off, keeping the picker open.
    fn toggle(&mut self, index: usize) -> Step {
        let Some((item, _)) = self.items().into_iter().nth(index) else {
            return Step::Stay;
        };
        let Some(pick) = self.pick(&item).filter(|_| self.kind == Kind::Label) else {
            return Step::Stay;
        };
        if let Pick::Label { label, on } = &pick {
            if *on {
                self.applied.insert(*label);
            } else {
                self.applied.remove(label);
            }
        }
        Step::Keep(pick)
    }

    /// The wheel: the chosen row moves.
    pub fn wheel(&mut self, down: bool) {
        let last = self.items().len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 3).min(last)
        } else {
            self.selected.saturating_sub(3)
        };
    }

    /// Which row a number key chooses: the `number`th of the numbered rows.
    fn numbered(&self, number: usize) -> Option<usize> {
        self.items()
            .iter()
            .position(|(_, row)| row.number == Some(number))
    }

    /// A key in the picker.
    pub fn key(&mut self, key: &KeyEvent, keys: &mut Keys) -> Step {
        match key.code {
            KeyCode::Down => {
                let last = self.items().len().saturating_sub(1);
                self.selected = (self.chosen() + 1).min(last);
                return Step::Stay;
            }
            KeyCode::Up => {
                self.selected = self.chosen().saturating_sub(1);
                return Step::Stay;
            }
            _ => {}
        }
        let filter_empty =
            matches!(self.kind, Kind::Label | Kind::Move) && self.field.value().is_empty();
        let bare = (key.modifiers - KeyModifiers::SHIFT).is_empty();
        let digit_or_space = matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit() || c == ' ');
        let typing = rules::is_typing(self.in_field, filter_empty, bare, digit_or_space);
        if let Outcome::Command(id) = keys.press(key, KeyContext::Picker, typing) {
            if let Ok(command) = id.parse::<CommandId>() {
                return self.command(command);
            }
            return Step::Stay;
        }
        if self.in_field {
            // `Enter` in a field is the field's own: it confirms what is typed.
            if key.code == KeyCode::Enter {
                return self.confirm();
            }
            self.type_in(key);
        }
        Step::Stay
    }

    fn type_in(&mut self, key: &KeyEvent) {
        let before = self.field.value().to_owned();
        self.field.handle_event(&crossterm::event::Event::Key(*key));
        if self.field.value() != before {
            self.selected = 0;
        }
    }

    /// Run a picker command.
    pub fn command(&mut self, command: CommandId) -> Step {
        let number = match command {
            CommandId::PickerChoose1 => Some(1),
            CommandId::PickerChoose2 => Some(2),
            CommandId::PickerChoose3 => Some(3),
            CommandId::PickerChoose4 => Some(4),
            _ => None,
        };
        if let Some(number) = number {
            return match self.numbered(number) {
                Some(index) => self.choose(index),
                None => Step::Stay,
            };
        }
        match command {
            CommandId::Back => Step::Close,
            CommandId::PickerTypeDate => {
                if matches!(self.kind, Kind::Snooze | Kind::Remind) {
                    self.in_field = true;
                }
                Step::Stay
            }
            CommandId::PickerToggle => {
                let at = self.chosen();
                self.toggle(at)
            }
            CommandId::PickerConfirm => self.confirm(),
            _ => Step::Stay,
        }
    }

    fn confirm(&mut self) -> Step {
        let typed = self.field.value().trim().to_owned();
        match self.kind {
            Kind::Snooze | Kind::Remind if self.in_field && !typed.is_empty() => {
                match rules::typed(&typed, self.now) {
                    Some(at) => Step::Choose(Pick::When(at)),
                    None => Step::Stay,
                }
            }
            Kind::Label => {
                let at = self.chosen();
                match self.items().into_iter().nth(at) {
                    Some((Item::Create(name), _)) => {
                        Step::Choose(Pick::Create { name, close: true })
                    }
                    _ => Step::Close,
                }
            }
            _ => {
                let at = self.chosen();
                self.choose(at)
            }
        }
    }
}
