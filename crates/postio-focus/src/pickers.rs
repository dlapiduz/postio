//! The pickers at the row (snooze, remind, labels, move) and the toast's policy. A picker acts on
//! what a verb would aim at when it opened, wherever the cursor goes since. A queued send's toast
//! carries an Undo of its own, which Undo runs first (#1752).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Keymap, MessageTarget};
use postio_model::{AccountId, Label, LabelId, Mailbox, MailboxId, MessageId, ThreadId};
use postio_ui::focus_target::{Aim, AimRow};
use postio_ui::pickers as words;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{Everything, FocusController, Intent, Reply, Request, SurfaceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Snooze,
    Remind,
    Label,
    Move,
}

impl PickerKind {
    fn of(id: CommandId) -> Option<Self> {
        Some(match id {
            CommandId::Snooze => PickerKind::Snooze,
            CommandId::RemindIfNoReply => PickerKind::Remind,
            CommandId::AddLabel => PickerKind::Label,
            CommandId::Move => PickerKind::Move,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Row(u32),
    OpenMessage,
    Result(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerField {
    Date,
    Filter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// Never reused: a token from rows since redrawn runs nothing.
    pub token: u64,
    pub section: Option<String>,
    pub name: String,
    pub detail: String,
    pub key: Option<String>,
    pub dot: bool,
    pub color: Option<String>,
    pub applied: bool,
    pub create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    pub kind: PickerKind,
    pub anchor: Anchor,
    pub title: String,
    pub target: String,
    pub field: PickerField,
    pub placeholder: String,
    pub typed: String,
    pub hint: Option<String>,
    pub rows: Vec<PickerRow>,
    pub footnote: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelsRead {
    pub account: AccountId,
    pub labels: Vec<Label>,
    pub counts: Vec<(LabelId, u32)>,
    pub carried: Vec<(ThreadId, Label)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldersRead {
    pub folders: Vec<Mailbox>,
    pub recent: Vec<MailboxId>,
}

#[derive(Debug, Clone)]
struct Aimed {
    aims: Vec<MessageTarget>,
    everything: Option<Everything>,
    message: Option<MessageId>,
    from_list: bool,
    from_results: bool,
}

impl Aimed {
    fn threads(&self) -> Vec<ThreadId> {
        self.aims
            .iter()
            .filter_map(|aim| match aim {
                MessageTarget::Threads(threads) => Some(threads.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

#[derive(Debug, Clone)]
enum Choice {
    At(DateTime<Local>),
    Label(Label),
    Create(String),
    Folder(MailboxId),
}

#[derive(Debug, Clone)]
struct Shown {
    token: u64,
    choice: Choice,
    numbered: bool,
}

#[derive(Debug)]
struct Open {
    kind: PickerKind,
    anchor: Anchor,
    target: String,
    now: DateTime<Local>,
    aimed: Aimed,
    typed: String,
    shown: Vec<Shown>,
    account: Option<AccountId>,
    labels: Vec<Label>,
    counts: HashMap<LabelId, u32>,
    applied: HashSet<LabelId>,
    closing: bool,
    pending: Option<(String, bool)>,
    folders: Vec<Mailbox>,
    recent: Vec<MailboxId>,
    draft: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Pickers {
    pub(crate) clock: Option<DateTime<Local>>,
    open: Option<Open>,
    /// A read asked under an older stamp is for a picker no longer up.
    stamp: u64,
    next_token: u64,
    /// Recent before the store's own note of them lands, which a quick second `m` can beat.
    moved: Vec<MailboxId>,
}

const CHOOSE: [CommandId; 4] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
];

/// The controller's always; nothing with no picker up (`Context::Picker` resolves them only while
/// one is).
const KEYS: [CommandId; 7] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
    CommandId::PickerTypeDate,
    CommandId::PickerToggle,
    CommandId::PickerConfirm,
];

pub(crate) fn picker_key(id: CommandId) -> bool {
    KEYS.contains(&id)
}

pub(crate) fn opens_picker(id: CommandId) -> bool {
    PickerKind::of(id).is_some()
}

fn target(selection: &Selection, said: Option<(String, String)>) -> String {
    let selected = match selection {
        Selection::These(picked) => picked.len(),
        Selection::Everything { .. } => return words::EVERY_THREAD.to_owned(),
    };
    if selected > 1 {
        return words::target(selected, "", "");
    }
    match said {
        Some((sender, subject)) => words::target(1, &sender, &subject),
        None => String::new(),
    }
}

impl Pickers {
    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn view(&mut self, keymap: &Keymap) -> Option<PickerView> {
        let mut open = self.open.take()?;
        let rows = self.rows(&mut open, keymap);
        let (title, field, placeholder, hint, footnote) = match open.kind {
            PickerKind::Snooze => (
                words::SNOOZE_TITLE,
                PickerField::Date,
                words::DATE_PLACEHOLDER,
                Some(words::date_hint(&open.typed, open.now)),
                words::snooze_footnote(keymap),
            ),
            PickerKind::Remind => (
                words::REMIND_TITLE,
                PickerField::Date,
                words::DATE_PLACEHOLDER,
                Some(words::date_hint(&open.typed, open.now)),
                words::remind_footnote(open.now),
            ),
            PickerKind::Label => (
                words::LABEL_TITLE,
                PickerField::Filter,
                words::LABEL_FILTER,
                None,
                words::label_footnote(keymap),
            ),
            PickerKind::Move => (
                words::MOVE_TITLE,
                PickerField::Filter,
                words::MOVE_FILTER,
                None,
                words::move_footnote(keymap),
            ),
        };
        let view = PickerView {
            kind: open.kind,
            anchor: open.anchor,
            title: title.to_owned(),
            target: open.target.clone(),
            field,
            placeholder: placeholder.to_owned(),
            typed: open.typed.clone(),
            hint,
            rows,
            footnote,
        };
        self.open = Some(open);
        Some(view)
    }

    fn rows(&mut self, open: &mut Open, keymap: &Keymap) -> Vec<PickerRow> {
        let mut rows = Vec::new();
        let mut shown = Vec::new();
        let mut row = |this: &mut Self, choice: Choice, numbered: bool, line: PickerRow| {
            let token = this.token();
            shown.push(Shown {
                token,
                choice,
                numbered,
            });
            rows.push(PickerRow { token, ..line });
        };
        match open.kind {
            PickerKind::Snooze | PickerKind::Remind => {
                let presets = match open.kind {
                    PickerKind::Snooze => postio_ui::schedule::snooze_presets(open.now),
                    _ => postio_ui::schedule::remind_presets(open.now),
                };
                for (name, at) in presets {
                    let line = PickerRow {
                        name: name.to_owned(),
                        detail: words::when_label(at, open.now),
                        ..blank()
                    };
                    row(self, Choice::At(at), true, line);
                }
            }
            PickerKind::Label => {
                for entry in words::label_rows(&open.labels, &open.typed) {
                    match entry {
                        words::LabelRow::Create(name) => {
                            let line = PickerRow {
                                name: words::create_label(&name),
                                create: true,
                                ..blank()
                            };
                            row(self, Choice::Create(name), false, line);
                        }
                        words::LabelRow::Label(label) => {
                            let applied = open.applied.contains(&label.id);
                            let count = open.counts.get(&label.id).copied().unwrap_or(0);
                            let line = PickerRow {
                                name: label.name.clone(),
                                detail: words::label_detail(applied, count),
                                dot: true,
                                color: label.color.clone(),
                                applied,
                                ..blank()
                            };
                            row(self, Choice::Label(label), false, line);
                        }
                    }
                }
            }
            PickerKind::Move => {
                for folder in words::move_rows(&open.folders, &open.recent, &open.typed) {
                    let line = PickerRow {
                        section: folder.section.map(str::to_owned),
                        name: folder.name,
                        detail: folder.count,
                        ..blank()
                    };
                    row(self, Choice::Folder(folder.folder), folder.numbered, line);
                }
            }
        }
        let mut number = 0;
        for (line, each) in rows.iter_mut().zip(&shown) {
            if each.numbered {
                line.key = CHOOSE
                    .get(number)
                    .and_then(|choose| postio_ui::hints::key(keymap, *choose));
                number += 1;
            }
        }
        open.shown = shown;
        rows
    }

    fn choice(&self, token: u64) -> Option<Choice> {
        let open = self.open.as_ref()?;
        open.shown
            .iter()
            .find(|row| row.token == token)
            .map(|row| row.choice.clone())
    }

    fn numbered(&self, number: usize) -> Option<u64> {
        let open = self.open.as_ref()?;
        open.shown
            .iter()
            .filter(|row| row.numbered)
            .nth(number)
            .map(|row| row.token)
    }
}

fn blank() -> PickerRow {
    PickerRow {
        token: 0,
        section: None,
        name: String::new(),
        detail: String::new(),
        key: None,
        dot: false,
        color: None,
        applied: false,
        create: false,
    }
}

impl FocusController {
    pub(crate) fn picker_on_list(
        &mut self,
        id: postio_core::CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        // "Move only this" is the one message the cursor is on, whatever is
        // selected: a message verb never widens to a selection's threads.
        let only_this = id == CommandId::MoveThisMessage;
        let kind = if only_this {
            PickerKind::Move
        } else {
            PickerKind::of(id)?
        };
        let selection = if only_this {
            Selection::These(Vec::new())
        } else {
            self.cursor.selection()
        };
        let at = self.cursor.aim_row(rows);
        let (aims, everything) =
            match postio_ui::focus_target::aim_by(&selection, self.cursor.reach(), at.as_ref(), id)
            {
                Aim::Everything { except } => (
                    Vec::new(),
                    Some(Everything {
                        accounts: self.cursor.accounts().to_vec(),
                        except,
                        query: None,
                    }),
                ),
                Aim::Targets(aims) if aims.is_empty() => return Some(Vec::new()),
                Aim::Targets(aims) => (aims, None),
            };
        let position = self.cursor.position();
        let said = position.and_then(|position| rows.said(position));
        let aimed = Aimed {
            aims,
            everything,
            message: at.filter(|row| !row.digest).map(|row| row.id),
            from_list: true,
            from_results: false,
        };
        let anchor = Anchor::Row(position.unwrap_or(0));
        Some(self.open_picker(kind, anchor, target(&selection, said), aimed))
    }

    pub(crate) fn picker_on_message(
        &mut self,
        id: CommandId,
        reading: MessageId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        let kind = PickerKind::of(id)?;
        let position = rows.position_of(reading);
        let Some(row) = position
            .and_then(|at| rows.facts(at))
            .or_else(|| self.elsewhere_facts(reading))
        else {
            return Some(Vec::new());
        };
        let at = AimRow {
            id: row.id,
            threads: row.threads,
            digest: row.digest,
            child: false,
        };
        let Aim::Targets(aims) = postio_ui::focus_target::aim_by(
            &Selection::These(Vec::new()),
            &Default::default(),
            Some(&at),
            id,
        ) else {
            return Some(Vec::new());
        };
        if aims.is_empty() {
            return Some(Vec::new());
        }
        let said = position
            .and_then(|position| rows.said(position))
            .or_else(|| self.elsewhere_said(reading));
        let aimed = Aimed {
            aims,
            everything: None,
            message: Some(at.id),
            from_list: false,
            from_results: false,
        };
        let target = target(&Selection::These(Vec::new()), said);
        Some(self.open_picker(kind, Anchor::OpenMessage, target, aimed))
    }

    pub(crate) fn picker_on_results(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let kind = PickerKind::of(id)?;
        let results = self.results.as_ref()?;
        let Some(aim) = results.aim() else {
            return Some(Vec::new());
        };
        let position = results.cursor.unwrap_or(0);
        let focused = results.cursor_message().map(|(message, _)| message);
        let (aims, everything, target) = match aim {
            crate::results::ResultsAim::Targets {
                aims,
                conversations,
            } => {
                let target = if conversations > 1 {
                    words::target(conversations, "", "")
                } else {
                    self.result_row(position)
                        .map(|row| {
                            let subject: String =
                                row.subject.iter().map(|run| run.text.as_str()).collect();
                            words::target(1, &row.sender, &subject)
                        })
                        .unwrap_or_default()
                };
                (aims, None, target)
            }
            crate::results::ResultsAim::Matching { query, except } => {
                let target = words::target(results.selected() as usize, "", "");
                (
                    Vec::new(),
                    Some(Everything {
                        accounts: Vec::new(),
                        except,
                        query: Some(query),
                    }),
                    target,
                )
            }
        };
        let aimed = Aimed {
            aims,
            everything,
            message: focused,
            from_list: false,
            from_results: true,
        };
        Some(self.open_picker(kind, Anchor::Result(position), target, aimed))
    }

    pub(crate) fn picker_on_shown(&mut self, shown: MessageId) -> Vec<Step> {
        let aimed = Aimed {
            aims: vec![MessageTarget::Messages(vec![shown])],
            everything: None,
            message: Some(shown),
            from_list: false,
            from_results: false,
        };
        self.open_picker(
            PickerKind::Move,
            Anchor::OpenMessage,
            "This message".to_owned(),
            aimed,
        )
    }

    pub(crate) fn picker_for_draft(&mut self, target: String) -> Vec<Step> {
        let aimed = Aimed {
            aims: Vec::new(),
            everything: None,
            message: None,
            from_list: false,
            from_results: false,
        };
        let steps = self.open_picker(PickerKind::Remind, Anchor::OpenMessage, target, aimed);
        if let Some(open) = self.pickers.open.as_mut() {
            open.draft = true;
        }
        steps
    }

    fn open_picker(
        &mut self,
        kind: PickerKind,
        anchor: Anchor,
        target: String,
        aimed: Aimed,
    ) -> Vec<Step> {
        self.pickers.stamp += 1;
        let stamp = self.pickers.stamp;
        let now = self.pickers.clock.unwrap_or_else(postio_ui::clock::now);
        let ask = match kind {
            PickerKind::Label => Some(Request::Labels {
                message: aimed.message,
                account: self.cursor.accounts().first().copied(),
                threads: aimed.threads(),
                stamp,
            }),
            PickerKind::Move => Some(Request::Folders { stamp }),
            PickerKind::Snooze | PickerKind::Remind => None,
        };
        self.pickers.open = Some(Open {
            kind,
            anchor,
            target,
            now,
            aimed,
            typed: String::new(),
            shown: Vec::new(),
            account: None,
            labels: Vec::new(),
            counts: HashMap::new(),
            applied: HashSet::new(),
            closing: false,
            pending: None,
            folders: Vec::new(),
            recent: Vec::new(),
            draft: false,
        });
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Picker, self.policy.caps.stacking);
        if let Some(view) = self.pickers.view(self.bar.keymap()) {
            steps.push(Step::Show(Intent::OpenPicker(view)));
        }
        steps.extend(ask.map(Step::Ask));
        steps
    }

    pub fn in_empty_filter(&self) -> bool {
        self.pickers.open.as_ref().is_some_and(|open| {
            matches!(open.kind, PickerKind::Label | PickerKind::Move) && open.typed.is_empty()
        })
    }

    pub(crate) fn redraw_picker(&mut self) -> Vec<Step> {
        self.pickers
            .view(self.bar.keymap())
            .map(|view| Step::Show(Intent::PickerRows(view)))
            .into_iter()
            .collect()
    }

    pub(crate) fn picker_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let on_top = self.surfaces.top() == Some(SurfaceKind::Picker);
        if !picker_key(id) && !(on_top && id == CommandId::Back) {
            return None;
        }
        if self.pickers.open.is_none() {
            // A picker the frontend reported, not one this opened: Back is
            // the surface stack's to close.
            return picker_key(id).then(Vec::new);
        }
        let steps = match id {
            CommandId::Back => self.close_picker(false),
            CommandId::PickerTypeDate => {
                let dated = self.pickers.open.as_ref().is_some_and(|open| {
                    matches!(open.kind, PickerKind::Snooze | PickerKind::Remind)
                });
                if dated {
                    vec![Step::Show(Intent::PickerField)]
                } else {
                    Vec::new()
                }
            }
            CommandId::PickerToggle => Vec::new(),
            CommandId::PickerConfirm => self.picker_confirm(),
            _ => {
                let number = CHOOSE.iter().position(|choose| *choose == id)?;
                match self.pickers.numbered(number) {
                    Some(token) => self.picker_choose(token),
                    None => Vec::new(),
                }
            }
        };
        Some(steps)
    }

    fn picker_confirm(&mut self) -> Vec<Step> {
        let Some(open) = self.pickers.open.as_ref() else {
            return Vec::new();
        };
        let kind = open.kind;
        match kind {
            PickerKind::Snooze | PickerKind::Remind => match words::typed(&open.typed, open.now) {
                Some(at) => self.picker_act(when(kind, at)),
                None => Vec::new(),
            },
            PickerKind::Label => {
                let names = open.labels.iter().map(|label| label.name.as_str());
                if words::offers_create(names, &open.typed) {
                    let name = open.typed.trim().to_owned();
                    self.picker_create(name, true)
                } else {
                    self.close_picker(false)
                }
            }
            PickerKind::Move => Vec::new(),
        }
    }

    pub(crate) fn picker_choose(&mut self, token: u64) -> Vec<Step> {
        let Some(kind) = self.pickers.open.as_ref().map(|open| open.kind) else {
            return Vec::new();
        };
        match self.pickers.choice(token) {
            Some(Choice::At(at)) => self.picker_act(when(kind, at)),
            Some(Choice::Folder(folder)) => {
                let mut steps = self.picker_act(Command::Move {
                    target: MessageTarget::Selection,
                    to: Some(folder),
                });
                self.pickers.moved.retain(|moved| *moved != folder);
                self.pickers.moved.insert(0, folder);
                self.pickers.moved.truncate(words::RECENT);
                steps.push(Step::Ask(Request::NoteMove(folder)));
                steps
            }
            Some(Choice::Label(label)) => self.picker_label(label),
            Some(Choice::Create(name)) => self.picker_create(name, false),
            None => Vec::new(),
        }
    }

    pub(crate) fn picker_toggle(&mut self, token: u64) -> Vec<Step> {
        match self.pickers.choice(token) {
            Some(Choice::Label(label)) => self.picker_label(label),
            Some(Choice::Create(name)) => self.picker_create(name, false),
            _ => Vec::new(),
        }
    }

    pub(crate) fn picker_typed(&mut self, text: String) -> Vec<Step> {
        let Some(open) = self.pickers.open.as_mut() else {
            return Vec::new();
        };
        if open.typed == text {
            return Vec::new();
        }
        open.typed = text;
        self.redraw_picker()
    }

    fn picker_act(&mut self, command: Command) -> Vec<Step> {
        if self.pickers.open.as_ref().is_some_and(|open| open.draft) {
            let Command::RemindIfNoReply { at: Some(at), .. } = command else {
                return Vec::new();
            };
            let mut steps = self.close_picker(true);
            steps.push(Step::Show(Intent::DraftReminder { at }));
            return steps;
        }
        let Some(aimed) = self.pickers.open.as_ref().map(|open| open.aimed.clone()) else {
            return Vec::new();
        };
        let mut steps = self.close_picker(true);
        steps.push(Step::Ask(Request::Send {
            command,
            aims: aimed.aims,
            everything: aimed.everything,
        }));
        steps
    }

    fn picker_label(&mut self, label: Label) -> Vec<Step> {
        let Some(open) = self.pickers.open.as_mut() else {
            return Vec::new();
        };
        let on = !open.applied.contains(&label.id);
        if on {
            open.applied.insert(label.id);
        } else {
            open.applied.remove(&label.id);
        }
        let send = Step::Ask(Request::Send {
            command: Command::AddLabel {
                target: MessageTarget::Selection,
                label: Some(label.id),
                on: Some(on),
            },
            aims: open.aimed.aims.clone(),
            everything: open.aimed.everything.clone(),
        });
        let mut steps = vec![send];
        steps.extend(self.redraw_picker());
        steps
    }

    fn picker_create(&mut self, name: String, closing: bool) -> Vec<Step> {
        let stamp = self.pickers.stamp;
        let Some(open) = self.pickers.open.as_mut() else {
            return Vec::new();
        };
        let Some(account) = open.account else {
            open.pending = Some((name, closing));
            return Vec::new();
        };
        open.closing = closing;
        vec![Step::Ask(Request::CreateLabel {
            account,
            name,
            stamp,
        })]
    }

    fn close_picker(&mut self, acted: bool) -> Vec<Step> {
        let mut steps = self.picker_gone(acted);
        if self.surfaces.dismiss(SurfaceKind::Picker) {
            steps.insert(0, Step::Show(Intent::CloseSurface(SurfaceKind::Picker)));
            if self.surfaces.top().is_none() {
                steps.insert(1, Step::Show(Intent::KeyboardHome));
            }
        }
        steps
    }

    pub(crate) fn picker_gone(&mut self, acted: bool) -> Vec<Step> {
        self.pickers.stamp += 1;
        let Some(open) = self.pickers.open.take() else {
            return Vec::new();
        };
        if open.aimed.from_list && (acted || open.kind == PickerKind::Label) {
            self.cursor.clear(self.feed.total())
        } else if open.aimed.from_results && (acted || open.kind == PickerKind::Label) {
            self.clear_results_checks()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn picker_reply(&mut self, reply: Reply) -> Vec<Step> {
        let stamp = match &reply {
            Reply::Labels { stamp, .. }
            | Reply::LabelCreated { stamp, .. }
            | Reply::Folders { stamp, .. } => *stamp,
            _ => return Vec::new(),
        };
        if stamp != self.pickers.stamp {
            return Vec::new();
        }
        let Some(open) = self.pickers.open.as_mut() else {
            return Vec::new();
        };
        match reply {
            Reply::Labels { answer, .. } => {
                let read = match answer {
                    Ok(read) => read,
                    Err(error) => {
                        tracing::warn!(%error, "the label picker could not read the labels");
                        return Vec::new();
                    }
                };
                open.applied = words::applied_labels(read.carried, &open.aimed.threads());
                open.account = Some(read.account);
                open.labels = read.labels;
                open.counts = read.counts.into_iter().collect();
                let Some((name, closing)) = open.pending.take() else {
                    return self.redraw_picker();
                };
                let wanted = name.to_lowercase();
                let found = open
                    .labels
                    .iter()
                    .find(|label| label.name.to_lowercase() == wanted)
                    .cloned();
                match found {
                    Some(label) => {
                        let mut steps = Vec::new();
                        if !open.applied.contains(&label.id) {
                            steps.extend(self.picker_label(label));
                        }
                        if closing {
                            steps.extend(self.close_picker(true));
                        } else if let Some(open) = self.pickers.open.as_mut() {
                            open.typed.clear();
                            steps.extend(self.redraw_picker());
                        }
                        steps
                    }
                    None => self.picker_create(name, closing),
                }
            }
            Reply::LabelCreated { answer, .. } => {
                let label = match answer {
                    Ok(label) => label,
                    Err(error) => {
                        tracing::warn!(%error, "Focus could not make a label");
                        return Vec::new();
                    }
                };
                open.applied.insert(label.id);
                let send = Step::Ask(Request::Send {
                    command: Command::AddLabel {
                        target: MessageTarget::Selection,
                        label: Some(label.id),
                        on: Some(true),
                    },
                    aims: open.aimed.aims.clone(),
                    everything: open.aimed.everything.clone(),
                });
                let labels = std::mem::take(&mut open.labels);
                open.labels = words::with_label(labels, label);
                if open.closing {
                    let mut steps = self.close_picker(true);
                    steps.push(send);
                    return steps;
                }
                open.typed.clear();
                let mut steps = vec![send];
                steps.extend(self.redraw_picker());
                steps
            }
            Reply::Folders { answer, .. } => {
                let read = match answer {
                    Ok(read) => read,
                    Err(error) => {
                        tracing::warn!(%error, "the move picker could not read the folders");
                        return Vec::new();
                    }
                };
                let mut folders = read.folders;
                folders.retain(words::is_destination);
                words::order_destinations(&mut folders);
                open.folders = folders;
                let moved = &self.pickers.moved;
                open.recent = moved
                    .iter()
                    .copied()
                    .chain(read.recent.into_iter().filter(|id| !moved.contains(id)))
                    .collect();
                self.redraw_picker()
            }
            _ => Vec::new(),
        }
    }
}

fn when(kind: PickerKind, at: DateTime<Local>) -> Command {
    match kind {
        PickerKind::Remind => Command::RemindIfNoReply {
            target: MessageTarget::Selection,
            at: Some(at.to_utc()),
        },
        _ => Command::Snooze {
            target: MessageTarget::Selection,
            until: Some(at.to_utc()),
        },
    }
}
