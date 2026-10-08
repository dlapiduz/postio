//! The pickers at the row, and the toast's policy (research R2, slice 9).
//!
//! Moved from `postio-gtk`'s window (`open_when`, `open_labels`,
//! `open_moves`, the picker chosen becoming a sent command) and its label
//! and move pickers, so the Mac's pickers behave as GTK's do. `s`, `h`, `l`
//! and `m` open a picker hung from the cursor's row -- or from the open
//! message, aimed at that message alone -- over what a verb would aim at
//! then; what is chosen in it goes there, wherever the cursor has gone
//! since. The words, the times, the rows and what a typed date means are
//! `postio_ui::pickers`' and `postio_ui::schedule`'s; this keeps which
//! picker is up, what it acts on, what each row on screen does, and which
//! reads are current, and numbers the rows the number keys reach. The
//! toolkit draws the picker it is given, owns its arrows and highlight, and
//! hands back the token of a row chosen or toggled.
//!
//! Keys: `1`-`4` choose the numbered rows, `Tab` puts the keyboard in the
//! date field, `Space` puts a label on or takes it off and keeps the
//! picker up, Return confirms -- a typed date, or a label typed that
//! nobody has -- and Back closes. A label picker opened over the list lets
//! the selection go when it closes, as GTK's did; the others let it go once
//! they act.
//!
//! The toast's policy, from `postio-widgets`' toast: a toast stays
//! [`TOAST_SECONDS`](postio_ui::focus_target::TOAST_SECONDS) unless its
//! Undo lasts longer ([`ToastKind::seconds`](crate::ToastKind::seconds)),
//! every toast replaces the one showing, and a queued send's toast carries
//! an Undo of its own -- cancelling the send -- which Undo runs first, until
//! a newer toast replaces it (#1752).

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

/// Which picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    /// "Snooze until" (`s`, screen 11).
    Snooze,
    /// "Remind me if no one replies by" (`h`, screen 12).
    Remind,
    /// "Labels" (`l`, screen 13).
    Label,
    /// "Move to folder" (`m`, screen 14).
    Move,
}

impl PickerKind {
    /// The picker `id` opens, when it opens one.
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

/// What a picker hangs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The list's row at this position: the cursor's.
    Row(u32),
    /// The open message's action row.
    OpenMessage,
}

/// The field a picker holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerField {
    /// A date typed in words, under the rows; `Tab` reaches it.
    Date,
    /// A filter over the rows, above them, holding the keyboard from the
    /// start.
    Filter,
}

/// One row of a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// What [`Input::PickerChoose`](crate::Input::PickerChoose) and
    /// [`Input::PickerToggle`](crate::Input::PickerToggle) hand back. Never
    /// reused: a token from rows since redrawn runs nothing.
    pub token: u64,
    /// The heading of the section this row starts, when it starts one:
    /// "Recent", "All folders".
    pub section: Option<String>,
    /// The name, in bold.
    pub name: String,
    /// On the right: a time, a count, "✓ applied".
    pub detail: String,
    /// The number key that chooses it, as the keymap spells it.
    pub key: Option<String>,
    /// Whether a label's colour dot is drawn before the name.
    pub dot: bool,
    /// The label's stored colour, `#rrggbb`; `None` for one the frontend
    /// picks from the name, as the places popover's dots.
    pub color: Option<String>,
    /// Whether the label is on everything the picker acts on.
    pub applied: bool,
    /// Whether it is "Create label “…”".
    pub create: bool,
}

/// A picker, whole: what [`Intent::OpenPicker`] and [`Intent::PickerRows`]
/// draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    /// Which picker.
    pub kind: PickerKind,
    /// What it hangs from.
    pub anchor: Anchor,
    /// Its title: "Snooze until".
    pub title: String,
    /// What it acts on: "Ada Moreno · Atlas Q3 budget", "3 conversations".
    pub target: String,
    /// The field it holds.
    pub field: PickerField,
    /// What the field says while it is empty.
    pub placeholder: String,
    /// What the field holds.
    pub typed: String,
    /// The date field's line under it: how to start, when the words land,
    /// or what it wants. `None` for a filter.
    pub hint: Option<String>,
    /// The rows, top to bottom. A label or move picker's come once their
    /// read lands, as [`Intent::PickerRows`].
    pub rows: Vec<PickerRow>,
    /// The footnote: what happens next, with its keys.
    pub footnote: String,
}

/// What the label picker reads when it opens, in one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelsRead {
    /// The account whose labels these are: the message's.
    pub account: AccountId,
    /// Its labels.
    pub labels: Vec<Label>,
    /// How many conversations carry each; a label nobody carries may be
    /// left out.
    pub counts: Vec<(LabelId, u32)>,
    /// The labels the conversations aimed at carry, each pair once.
    pub carried: Vec<(ThreadId, Label)>,
}

/// What the move picker reads when it opens, in one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldersRead {
    /// Every enabled account's folders, as they are: the controller keeps
    /// the destinations among them.
    pub folders: Vec<Mailbox>,
    /// The last destinations, most recent first.
    pub recent: Vec<MailboxId>,
}

/// What a picker acts on, captured as it opened.
#[derive(Debug, Clone)]
struct Aimed {
    aims: Vec<MessageTarget>,
    everything: Option<Everything>,
    /// The message it opened on: whose account the labels are.
    message: Option<MessageId>,
    /// Whether it opened over the list, whose selection goes once it acts.
    from_list: bool,
}

impl Aimed {
    /// The conversations it acts on: what a label is applied on.
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

/// What a row on screen does.
#[derive(Debug, Clone)]
enum Choice {
    /// A preset's time.
    At(DateTime<Local>),
    /// A label, on or off.
    Label(Label),
    /// Make this label.
    Create(String),
    /// Move there.
    Folder(MailboxId),
}

/// A row on screen.
#[derive(Debug, Clone)]
struct Shown {
    token: u64,
    choice: Choice,
    numbered: bool,
}

/// The picker that is up.
#[derive(Debug)]
struct Open {
    kind: PickerKind,
    anchor: Anchor,
    target: String,
    /// The clock when it opened: what the presets and a typed date count
    /// from.
    now: DateTime<Local>,
    aimed: Aimed,
    typed: String,
    shown: Vec<Shown>,
    /// The account the labels are, once read.
    account: Option<AccountId>,
    labels: Vec<Label>,
    counts: HashMap<LabelId, u32>,
    applied: HashSet<LabelId>,
    /// A label being made closes the picker once it is on.
    closing: bool,
    /// A label asked for before the labels landed -- its name, and whether
    /// the picker closes once it is on -- made or put on when they do.
    pending: Option<(String, bool)>,
    folders: Vec<Mailbox>,
    recent: Vec<MailboxId>,
}

/// The pickers' state.
#[derive(Debug, Default)]
pub(crate) struct Pickers {
    /// The clock, when stopped.
    pub(crate) clock: Option<DateTime<Local>>,
    open: Option<Open>,
    /// Moves with every opening and closing: a read asked under an older
    /// one is for a picker no longer up.
    stamp: u64,
    next_token: u64,
}

/// The number keys, in order: the first four numbered rows answer to them.
const CHOOSE: [CommandId; 4] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
];

/// The pickers' own keys: the controller's always, and nothing with no
/// picker up (`Context::Picker` resolves them only while one is).
const KEYS: [CommandId; 7] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
    CommandId::PickerTypeDate,
    CommandId::PickerToggle,
    CommandId::PickerConfirm,
];

/// Whether `id` is one of the pickers' own keys.
pub(crate) fn picker_key(id: CommandId) -> bool {
    KEYS.contains(&id)
}

/// Whether `id` opens a picker.
pub(crate) fn opens_picker(id: CommandId) -> bool {
    PickerKind::of(id).is_some()
}

/// What a picker names as its target: everything, how many, or the one
/// conversation by its sender and subject.
fn target(selection: &Selection, said: Option<(String, String)>) -> String {
    let selected = match selection {
        Selection::These(picked) => picked.len(),
        Selection::Everything { .. } => return "Every conversation".to_owned(),
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

    /// The picker up, as it is now, its rows given fresh tokens.
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

    /// `open`'s rows for what is typed, each with a new token, the
    /// numbered ones keyed `1`-`4`.
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
        // The number keys reach the numbered rows, in order.
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

    /// What the row `token` on screen does.
    fn choice(&self, token: u64) -> Option<Choice> {
        let open = self.open.as_ref()?;
        open.shown
            .iter()
            .find(|row| row.token == token)
            .map(|row| row.choice.clone())
    }

    /// The token of the `number`th numbered row on screen.
    fn numbered(&self, number: usize) -> Option<u64> {
        let open = self.open.as_ref()?;
        open.shown
            .iter()
            .filter(|row| row.numbered)
            .nth(number)
            .map(|row| row.token)
    }
}

/// A row with nothing in it yet.
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
    /// `s`, `h`, `l` or `m` on the list: the picker at the cursor's row,
    /// over what a verb would aim at now; `None` when `id` opens none.
    pub(crate) fn picker_on_list(
        &mut self,
        id: postio_core::CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        let kind = PickerKind::of(id)?;
        let selection = self.cursor.selection();
        let at = self.cursor.row(rows).map(|row| AimRow {
            id: row.id,
            threads: row.threads,
            digest: row.digest,
        });
        let (aims, everything) =
            match postio_ui::focus_target::aim_by(&selection, self.cursor.reach(), at.as_ref()) {
                Aim::Everything { except } => (
                    Vec::new(),
                    Some(Everything {
                        accounts: self.cursor.accounts().to_vec(),
                        except,
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
        };
        let anchor = Anchor::Row(position.unwrap_or(0));
        Some(self.open_picker(kind, anchor, target(&selection, said), aimed))
    }

    /// `s`, `h`, `l` or `m` in the open message: the picker at its action
    /// row, aimed at the message alone -- not the selection, which its
    /// window does not show. `None` when `id` opens none.
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
        };
        let Aim::Targets(aims) = postio_ui::focus_target::aim_by(
            &Selection::These(Vec::new()),
            &Default::default(),
            Some(&at),
        ) else {
            return Some(Vec::new());
        };
        if aims.is_empty() {
            return Some(Vec::new());
        }
        let said = position.and_then(|position| rows.said(position));
        let aimed = Aimed {
            aims,
            everything: None,
            message: Some(at.id),
            from_list: false,
        };
        let target = target(&Selection::These(Vec::new()), said);
        Some(self.open_picker(kind, Anchor::OpenMessage, target, aimed))
    }

    /// Put the picker up, and ask for what it lists.
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

    /// Whether a picker with a filter is up and its filter holds nothing:
    /// where a bare digit or space is the picker's key rather than typing
    /// (`postio_ui::pickers::is_typing`).
    pub fn in_empty_filter(&self) -> bool {
        self.pickers.open.as_ref().is_some_and(|open| {
            matches!(open.kind, PickerKind::Label | PickerKind::Move) && open.typed.is_empty()
        })
    }

    /// Redraw the picker up.
    pub(crate) fn redraw_picker(&mut self) -> Vec<Step> {
        self.pickers
            .view(self.bar.keymap())
            .map(|view| Step::Show(Intent::PickerRows(view)))
            .into_iter()
            .collect()
    }

    /// A picker's key: the pickers' own always (nothing with no picker up),
    /// and Back while one is on top. `None` for the rest.
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
            // `Space` is on the row the toolkit highlights, which it names
            // with `PickerToggle(token)`.
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

    /// Return with no row highlighted: the date typed, or a label typed
    /// that nobody has; a label picker closes either way.
    fn picker_confirm(&mut self) -> Vec<Step> {
        let Some(open) = self.pickers.open.as_ref() else {
            return Vec::new();
        };
        let kind = open.kind;
        match kind {
            PickerKind::Snooze | PickerKind::Remind => {
                match words::typed(&open.typed, open.now) {
                    Some(at) => self.picker_act(when(kind, at)),
                    // The hint under the field already says what it wants.
                    None => Vec::new(),
                }
            }
            PickerKind::Label => {
                let names = open.labels.iter().map(|label| label.name.as_str());
                if words::offers_create(names, &open.typed) {
                    let name = open.typed.trim().to_owned();
                    self.picker_create(name, true)
                } else {
                    self.close_picker(false)
                }
            }
            // A folder is chosen by its row, highlighted or numbered.
            PickerKind::Move => Vec::new(),
        }
    }

    /// The row `token` chosen: a click, a number key, or Return on it.
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
                steps.push(Step::Ask(Request::NoteMove(folder)));
                steps
            }
            // A label chosen is a label toggled: the picker stays up.
            Some(Choice::Label(label)) => self.picker_label(label),
            Some(Choice::Create(name)) => self.picker_create(name, false),
            None => Vec::new(),
        }
    }

    /// `Space` on the row `token`: a label on or off, or one made.
    pub(crate) fn picker_toggle(&mut self, token: u64) -> Vec<Step> {
        match self.pickers.choice(token) {
            Some(Choice::Label(label)) => self.picker_label(label),
            Some(Choice::Create(name)) => self.picker_create(name, false),
            _ => Vec::new(),
        }
    }

    /// The picker's field holds `text` now.
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

    /// Send `command` where the picker aimed, and close it.
    fn picker_act(&mut self, command: Command) -> Vec<Step> {
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

    /// Put `label` on what the picker acts on, or take it off when it is
    /// on all of it; the picker stays up.
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

    /// Make the label `name` in the labels' account, to put on once made;
    /// the picker closes then when `closing`.
    fn picker_create(&mut self, name: String, closing: bool) -> Vec<Step> {
        let stamp = self.pickers.stamp;
        let Some(open) = self.pickers.open.as_mut() else {
            return Vec::new();
        };
        // The labels are an account's: until they are read, the name waits
        // for them -- it may be one of them.
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

    /// Close the picker up, `acted` when it sent something. The selection
    /// it acted on from the list goes then -- and a label picker's however
    /// it closes, as GTK's did.
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

    /// The picker is gone, however it went: what it showed is forgotten,
    /// an answer still on its way is for nothing, and the selection goes
    /// as [`close_picker`](Self::close_picker) says.
    pub(crate) fn picker_gone(&mut self, acted: bool) -> Vec<Step> {
        self.pickers.stamp += 1;
        let Some(open) = self.pickers.open.take() else {
            return Vec::new();
        };
        if open.aimed.from_list && (acted || open.kind == PickerKind::Label) {
            self.cursor.clear(self.feed.total())
        } else {
            Vec::new()
        }
    }

    /// A reply a picker asked for: drawn when it is for the picker up.
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
                // Asked for before the labels landed: put on when it is one
                // of them, in any case, and made when it is not.
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
                open.recent = read.recent;
                self.redraw_picker()
            }
            _ => Vec::new(),
        }
    }
}

/// What choosing `at` sends from a snooze or remind picker.
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
