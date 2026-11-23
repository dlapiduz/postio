use std::collections::HashMap;

use chrono::{DateTime, Duration, Local, NaiveTime, Timelike, Utc};
use postio_core::{Command, CommandId};
use postio_model::calendar::{AgendaEntry, EditScope, EventDetail, EventFields, EventTarget};
use postio_ui::calendar::editor::{self as words, EditorField, EditorView};

use crate::FocusController;
use crate::feed::Step;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Edit,
    Move { keep: NaiveTime, length: Duration },
    Delete,
}

#[derive(Debug)]
pub(crate) struct Editor {
    mode: Mode,
    target: Option<EventTarget>,
    recurs: bool,
    texts: HashMap<EditorField, String>,
    opened: Option<(String, DateTime<Utc>, String, DateTime<Utc>)>,
    scope: EditScope,
    error: Option<String>,
}

impl Editor {
    pub(crate) fn new(now: DateTime<Local>) -> Self {
        let start = (now + Duration::hours(1))
            .with_minute(0)
            .and_then(|at| at.with_second(0))
            .and_then(|at| at.with_nanosecond(0))
            .unwrap_or(now)
            .to_utc();
        let mut texts = HashMap::new();
        texts.insert(EditorField::Starts, words::start_text(start, &Local));
        texts.insert(EditorField::Ends, words::length_text(Duration::hours(1)));
        Editor {
            mode: Mode::Edit,
            target: None,
            recurs: false,
            texts,
            opened: None,
            scope: EditScope::This,
            error: None,
        }
    }

    pub(crate) fn of(detail: &EventDetail) -> Self {
        let entry = &detail.entry;
        let starts = words::start_text(entry.start, &Local);
        let ends = words::length_text(entry.end - entry.start);
        let people = detail
            .attendees
            .iter()
            .map(|(address, _)| address.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let texts = HashMap::from([
            (EditorField::Title, entry.title.clone()),
            (EditorField::Starts, starts.clone()),
            (EditorField::Ends, ends.clone()),
            (EditorField::Location, entry.location.clone()),
            (EditorField::People, people),
            (EditorField::Notes, detail.notes.clone()),
        ]);
        Editor {
            mode: Mode::Edit,
            target: Some(EventTarget {
                occurrence: entry.cursor(),
                recurrence_id: entry.recurrence_id.clone(),
            }),
            recurs: detail.repeat.is_some() || entry.recurrence_id.is_some(),
            texts,
            opened: Some((starts, entry.start, ends, entry.end)),
            scope: EditScope::This,
            error: None,
        }
    }

    fn on(entry: &AgendaEntry, mode: Mode) -> Self {
        Editor {
            mode,
            target: Some(EventTarget {
                occurrence: entry.cursor(),
                recurrence_id: entry.recurrence_id.clone(),
            }),
            recurs: entry.recurrence_id.is_some(),
            texts: HashMap::new(),
            opened: None,
            scope: EditScope::This,
            error: None,
        }
    }

    pub(crate) fn moving(entry: &AgendaEntry) -> Self {
        let keep = entry.start.with_timezone(&Local).time();
        Editor::on(
            entry,
            Mode::Move {
                keep,
                length: entry.end - entry.start,
            },
        )
    }

    pub(crate) fn deleting(entry: &AgendaEntry) -> Self {
        Editor::on(entry, Mode::Delete)
    }

    fn moved_to(&self, now: DateTime<Local>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
        let Mode::Move { keep, length } = self.mode else {
            return self.when(now);
        };
        let typed = self.text(EditorField::Starts);
        let start = postio_search::date::parse_move(typed, now, keep)
            .map(|at| at.to_utc())
            .ok_or_else(|| words::unread_start(typed))?;
        Ok((start, start + length))
    }

    fn text(&self, field: EditorField) -> &str {
        self.texts.get(&field).map_or("", String::as_str)
    }

    fn when(&self, now: DateTime<Local>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
        let (starts, ends) = (self.text(EditorField::Starts), self.text(EditorField::Ends));
        let opened = self.opened.as_ref();
        let start = match opened {
            Some((text, start, ..)) if text == starts => *start,
            _ => postio_search::date::parse_when(starts, now)
                .map(|at| at.to_utc())
                .ok_or_else(|| words::unread_start(starts))?,
        };
        let end = match opened {
            Some((_, from, text, end)) if text == ends => start + (*end - *from),
            _ => postio_search::date::parse_end(ends, start.with_timezone(&Local))
                .map(|at| at.to_utc())
                .ok_or_else(|| words::unread_end(ends))?,
        };
        Ok((start, end))
    }

    fn fields(&self, now: DateTime<Local>) -> Result<EventFields, String> {
        let title = self.text(EditorField::Title).trim();
        if title.is_empty() {
            return Err(words::NO_TITLE.to_owned());
        }
        let (start, end) = self.when(now)?;
        Ok(EventFields {
            title: title.to_owned(),
            start,
            end,
            all_day: false,
            location: self.text(EditorField::Location).trim().to_owned(),
            notes: self.text(EditorField::Notes).to_owned(),
            attendees: postio_model::address::parse_list(self.text(EditorField::People)),
            minutes_before: None,
        })
    }

    fn inviting(&self) -> bool {
        !postio_model::address::parse_list(self.text(EditorField::People)).is_empty()
    }
}

impl FocusController {
    pub(crate) fn editor_view(&self) -> Option<EditorView> {
        let editor = self.calendar.as_ref()?.editor.as_ref()?;
        let now = self.now();
        let keymap = self.bar.keymap();
        let (heading, fields, keys): (String, Vec<EditorField>, _) = match editor.mode {
            Mode::Edit => (
                words::heading(editor.target.is_some()).to_owned(),
                EditorField::ALL.to_vec(),
                words::keys(keymap, editor.inviting()),
            ),
            Mode::Move { .. } => (
                postio_core::registry::get(CommandId::MoveEvent)
                    .title
                    .to_owned(),
                vec![EditorField::Starts],
                words::verb_keys(keymap, CommandId::OpenMessage, "move"),
            ),
            Mode::Delete => (
                postio_core::registry::title_in(CommandId::Delete, postio_core::Context::Calendar)
                    .to_owned(),
                Vec::new(),
                words::verb_keys(keymap, CommandId::Delete, "delete"),
            ),
        };
        Some(EditorView {
            heading,
            fields: fields
                .iter()
                .map(|field| (*field, editor.text(*field).to_owned()))
                .collect(),
            when: editor
                .moved_to(now)
                .map(|(start, end)| words::when_line(start, end, &Local)),
            scope: editor.recurs.then_some(editor.scope),
            error: editor.error.clone(),
            keys,
        })
    }

    pub(crate) fn editor_typed(&mut self, field: EditorField, text: String) -> Vec<Step> {
        let Some(editor) = self
            .calendar
            .as_mut()
            .and_then(|calendar| calendar.editor.as_mut())
        else {
            return Vec::new();
        };
        editor.texts.insert(field, text);
        editor.error = None;
        self.draw_calendar()
    }

    pub(crate) fn editor_scope(&mut self, scope: EditScope) -> Vec<Step> {
        let Some(editor) = self
            .calendar
            .as_mut()
            .and_then(|calendar| calendar.editor.as_mut())
        else {
            return Vec::new();
        };
        editor.scope = scope;
        self.draw_calendar()
    }

    pub(crate) fn editor_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let now = self.now();
        let calendar = self.calendar.as_mut()?;
        let editor = calendar.editor.as_mut()?;
        if editor.mode != Mode::Edit {
            let confirm = matches!(
                id,
                CommandId::SaveEvent | CommandId::OpenMessage | CommandId::Delete
            );
            if id == CommandId::Back {
                calendar.editor = None;
                return Some(self.draw_calendar());
            }
            if !confirm {
                return (id != CommandId::CheatSheet).then(Vec::new);
            }
            let event = editor.target.clone()?;
            let scope = editor.recurs.then_some(editor.scope);
            let command = match editor.mode {
                Mode::Move { .. } => match editor.moved_to(now) {
                    Ok((start, _)) => {
                        calendar.follow = Some(start);
                        Command::MoveEventTo {
                            event,
                            start,
                            scope,
                        }
                    }
                    Err(error) => {
                        editor.error = Some(error);
                        return Some(self.draw_calendar());
                    }
                },
                _ => {
                    calendar.undo_follows = Some(event.occurrence.start);
                    Command::DeleteEvent { event, scope }
                }
            };
            calendar.editor = None;
            calendar.open = None;
            let mut steps = self.verbs.post(command);
            steps.extend(self.draw_calendar());
            return Some(steps);
        }
        let send = match id {
            CommandId::Back => {
                calendar.editor = None;
                return Some(self.draw_calendar());
            }
            CommandId::SaveEvent => false,
            CommandId::SaveAndSendEvent if editor.inviting() => true,
            CommandId::CheatSheet => return None,
            _ => return Some(Vec::new()),
        };
        let fields = match editor.fields(now) {
            Ok(fields) => fields,
            Err(error) => {
                editor.error = Some(error);
                return Some(self.draw_calendar());
            }
        };
        let event = editor.target.clone();
        let scope = editor.recurs.then_some(editor.scope);
        calendar.editor = None;
        calendar.open = None;
        calendar.follow = Some(fields.start);
        let command = if send {
            Command::SaveAndSendEvent {
                event,
                calendar: None,
                fields: Some(fields),
                scope,
            }
        } else {
            Command::SaveEvent {
                event,
                calendar: None,
                fields: Some(fields),
                scope,
            }
        };
        let mut steps = self.verbs.post(command);
        steps.extend(self.draw_calendar());
        Some(steps)
    }
}
