//! Capture: a task or a note written to the vault from a message, and the
//! `postio://` links that bring Postio back to it (research R2, slice 12;
//! spec 007 US15, screen 25).
//!
//! Moved from `postio-gtk`'s `capture.rs` (what the sheet holds, its keys,
//! the exact line it previews and writes) and the window's `open_capture`,
//! `capture_source` and `open_link`. The words are `postio_ui::capture`'s
//! and `postio_ui::links`'; the line is `postio_vault`'s own, so the
//! preview is the line written, byte for byte, with the message's link
//! before the date (C21). Capture opens only when a vault is configured,
//! and says how to name one otherwise (C9).

use chrono::NaiveDate;
use postio_client::protocol::VaultPicture;
use postio_core::CommandId;
use postio_model::MessageId;
use postio_ui::capture::{self as words, Mode, Pick, Source};
use postio_ui::hints;
use postio_vault::{NoteEntry, Project, Reason, Task};

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind, ToastKind};

/// The capture window, drawable as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureView {
    /// A task or a note.
    pub mode: Mode,
    /// "From Ada · Quarterly review · Mon".
    pub from: String,
    /// The field's title: "Task", "Note".
    pub field: String,
    /// The field's text: set the field from it only when it differs.
    pub text: String,
    /// `⌥S`'s hint: the key that puts the subject in.
    pub subject_key: Option<String>,
    /// Whether the due row shows: only for a task.
    pub has_due: bool,
    /// The due day chosen.
    pub due: Option<NaiveDate>,
    /// It, written out: "Friday, 2 October 2026", "No due date".
    pub due_label: String,
    /// The quick picks, in day order, then "None".
    pub picks: Vec<Pick>,
    /// "Project · suggested: the subject names Review", or "Project".
    pub project_title: String,
    /// The project chosen: its name, or "Inbox".
    pub project: String,
    /// Its note, relative to the vault.
    pub project_note: String,
    /// The key that opens the project list.
    pub project_key: Option<String>,
    /// Whether the project list is open, with its filter.
    pub projects_open: bool,
    /// The filter's text.
    pub filter: String,
    /// The projects the filter lets through, then the Inbox.
    pub projects: Vec<CaptureProject>,
    /// "This exact line will be appended".
    pub preview_title: String,
    /// The exact line, or lines, written.
    pub preview: String,
    /// Where it goes, and when it is due.
    pub footnote: String,
    /// "Add task", "Add note".
    pub button: String,
    /// Its key.
    pub button_key: Option<String>,
    /// Why the vault could not be read, or the capture not written.
    pub error: Option<String>,
}

/// One project the capture can go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureProject {
    /// What [`crate::Input::CaptureProject`] names it by. Never reused.
    pub token: u64,
    /// Its name, or "Inbox".
    pub name: String,
    /// Its note.
    pub note: String,
    /// "3 open".
    pub open: String,
    /// Whether it is the one chosen.
    pub chosen: bool,
}

/// What the controller keeps of the capture window.
#[derive(Debug)]
pub(crate) struct Capture {
    mode: Mode,
    source: Source,
    text: String,
    due: Option<NaiveDate>,
    picture: Option<VaultPicture>,
    project: Option<Project>,
    /// Whether the project chosen is the vault's suggestion.
    suggested: bool,
    projects_open: bool,
    filter: String,
    /// The project rows on screen, each with its token: `None` the Inbox.
    listed: Vec<(u64, Option<Project>)>,
    error: Option<String>,
    stamp: u64,
}

impl Capture {
    fn task(&self) -> Task {
        Task {
            text: self.text.clone(),
            message: self.source.message,
            due: self.due,
        }
    }

    fn entry(&self) -> NoteEntry {
        NoteEntry {
            text: self.text.clone(),
            message: self.source.message,
            quote: None,
        }
    }

    fn place(&self) -> String {
        self.project
            .as_ref()
            .map_or_else(|| words::INBOX.to_owned(), |project| project.name.clone())
    }

    fn tasks_note(&self) -> std::path::PathBuf {
        self.picture
            .as_ref()
            .map(|picture| picture.tasks_note.clone())
            .unwrap_or_default()
    }
}

impl FocusController {
    /// `t` or `n` on the list or the open message: capture, as a task or a
    /// note, from the message aimed at; or, with no vault, how to name one.
    pub(crate) fn open_capture(&mut self, mode: Mode, rows: &dyn Rows) -> Vec<Step> {
        if self.states.config.vault.is_none() {
            return vec![Step::Show(Intent::Toast {
                text: words::NO_VAULT.to_owned(),
                kind: ToastKind::Notice,
            })];
        }
        let Some(source) = self.capture_source(rows) else {
            return Vec::new();
        };
        let stamp = self.stamp();
        let subject = source.subject.clone();
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Capture, self.policy.caps.stacking);
        self.capture = Some(Capture {
            mode,
            text: source
                .sentence
                .clone()
                .unwrap_or_else(|| source.subject.clone()),
            due: source.due,
            source,
            picture: None,
            project: None,
            suggested: false,
            projects_open: false,
            filter: String::new(),
            listed: Vec::new(),
            error: None,
            stamp,
        });
        if let Some(view) = self.capture_view() {
            steps.push(Step::Show(Intent::OpenCapture(Box::new(view))));
        }
        steps.push(Step::Ask(Request::Vault { subject, stamp }));
        steps
    }

    /// What a capture is made from: the message open over the list when one
    /// is, the cursor's row otherwise, with its marker's sentence and day.
    fn capture_source(&self, rows: &dyn Rows) -> Option<Source> {
        let reading = self.surfaces.reading();
        let message = match reading {
            Some(message) => message,
            None => {
                let at = self.cursor.position()?;
                rows.facts(at).filter(|row| !row.digest)?.id
            }
        };
        let row = rows.position_of(message).and_then(|at| rows.row(at));
        let title = self.reading_subject(message).unwrap_or_default();
        words::source(message, row.as_ref(), &title, self.now())
    }

    /// The subject of `message` where the controller holds it: a filtered
    /// message, a digest's.
    fn reading_subject(&self, message: MessageId) -> Option<String> {
        if let Some(filtered) = &self.filtered
            && let Some(at) = filtered.index_of(message)
        {
            return filtered.subject(at);
        }
        if let Some(window) = &self.digest
            && let Some(at) = window.index_of(message)
        {
            return window.subject(at);
        }
        None
    }

    /// Whether `id` is capture's own while it is on top.
    pub(crate) fn capture_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::CaptureTask
                | CommandId::CaptureNote
                | CommandId::CaptureUseSubject
                | CommandId::CaptureChangeProject
                | CommandId::CaptureWrite
        )
    }

    /// A command while capture is on top. It holds the keyboard: what is not
    /// its own does nothing, but the key map and Quit.
    pub(crate) fn capture_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let capture = self.capture.as_mut()?;
        match id {
            CommandId::Back => return Some(self.close_capture()),
            CommandId::CaptureTask => capture.mode = Mode::Task,
            CommandId::CaptureNote => capture.mode = Mode::Note,
            CommandId::CaptureUseSubject => capture.text = capture.source.subject.clone(),
            CommandId::CaptureChangeProject => capture.projects_open = !capture.projects_open,
            CommandId::CaptureWrite => return Some(self.write_capture()),
            CommandId::CheatSheet => return None,
            _ => return Some(Vec::new()),
        }
        Some(self.draw_capture())
    }

    /// Capture goes, writing nothing more.
    fn close_capture(&mut self) -> Vec<Step> {
        self.capture = None;
        self.surfaces.dismiss(SurfaceKind::Capture);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Capture))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// `mod+Return`: append what the window shows to the vault.
    fn write_capture(&mut self) -> Vec<Step> {
        let Some(capture) = self.capture.as_ref() else {
            return Vec::new();
        };
        let stamp = capture.stamp;
        let request = match capture.mode {
            Mode::Task => Request::CaptureTask {
                project: capture.project.clone(),
                task: capture.task(),
                stamp,
            },
            Mode::Note => Request::CaptureNote {
                note: capture
                    .project
                    .as_ref()
                    .map_or_else(|| capture.tasks_note(), |project| project.note.clone()),
                entry: capture.entry(),
                stamp,
            },
        };
        vec![Step::Ask(request)]
    }

    /// Something the person did in the capture window.
    pub(crate) fn capture_input(&mut self, input: crate::Input) -> Vec<Step> {
        use crate::Input;
        let Some(capture) = self.capture.as_mut() else {
            return Vec::new();
        };
        match input {
            Input::CaptureTyped { text } => {
                if capture.text == text {
                    return Vec::new();
                }
                capture.text = text;
            }
            Input::CaptureDue(due) => capture.due = due,
            Input::CaptureFilter { text } => {
                if capture.filter == text {
                    return Vec::new();
                }
                capture.filter = text;
            }
            Input::CaptureProject(token) => {
                let Some((_, project)) = capture
                    .listed
                    .iter()
                    .find(|(listed, _)| *listed == token)
                    .cloned()
                else {
                    return Vec::new();
                };
                capture.suggested = capture
                    .picture
                    .as_ref()
                    .and_then(|picture| picture.suggestion.as_ref())
                    .is_some_and(|suggestion| Some(&suggestion.project) == project.as_ref());
                capture.project = project;
                capture.projects_open = false;
            }
            _ => return Vec::new(),
        }
        self.draw_capture()
    }

    /// An answer for the capture window.
    pub(crate) fn capture_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::Vault { stamp, answer } => {
                let Some(capture) = self.capture.as_mut().filter(|it| it.stamp == stamp) else {
                    return Vec::new();
                };
                match answer {
                    Ok(picture) => {
                        let suggested = picture
                            .suggestion
                            .as_ref()
                            .map(|suggestion| suggestion.project.clone());
                        capture.suggested = suggested.is_some();
                        capture.project = suggested;
                        capture.picture = Some(picture);
                    }
                    Err(error) => capture.error = Some(error),
                }
                self.draw_capture()
            }
            Reply::Captured { stamp, answer } => {
                let Some(capture) = self.capture.as_mut().filter(|it| it.stamp == stamp) else {
                    return Vec::new();
                };
                match answer {
                    Ok(()) => {
                        let said = words::added(
                            capture.mode,
                            &capture.place(),
                            match capture.mode {
                                Mode::Task => capture.due,
                                Mode::Note => None,
                            },
                        );
                        let mut steps = self.close_capture();
                        steps.push(Step::Show(Intent::Toast {
                            text: said,
                            kind: ToastKind::Notice,
                        }));
                        steps
                    }
                    Err(error) => {
                        capture.error = Some(error);
                        self.draw_capture()
                    }
                }
            }
            _ => Vec::new(),
        }
    }

    fn draw_capture(&mut self) -> Vec<Step> {
        self.capture_view()
            .map(|view| vec![Step::Show(Intent::Capture(Box::new(view)))])
            .unwrap_or_default()
    }

    /// The capture window in words, as it stands; the project rows on
    /// screen get their tokens here.
    pub(crate) fn capture_view(&mut self) -> Option<CaptureView> {
        let today = self.now().date_naive();
        let keymap = self.keymap().clone();
        let mut next = self.stamps;
        let capture = self.capture.as_mut()?;
        let mode = capture.mode;
        let tasks_note = capture.tasks_note();
        let named = capture
            .picture
            .as_ref()
            .and_then(|picture| picture.suggestion.as_ref())
            .filter(|_| capture.suggested)
            .map(|suggestion| match &suggestion.reason {
                // The project's own name says it as written; the subject's
                // word was folded.
                Reason::NamedInSubject(_) => suggestion.project.name.clone(),
            });
        let (project, project_note) = match &capture.project {
            Some(project) => (project.name.clone(), project.note.display().to_string()),
            None => (
                words::INBOX.to_owned(),
                words::inbox_note(&tasks_note.display().to_string()),
            ),
        };
        // The projects the filter lets through, and the Inbox, each with a
        // token never used before.
        let mut listed: Vec<(u64, Option<Project>)> = Vec::new();
        let mut projects = Vec::new();
        if let Some(picture) = &capture.picture {
            let open = |note: &std::path::Path| {
                picture
                    .tasks
                    .iter()
                    .filter(|task| task.note == note && !task.finished)
                    .count()
            };
            let shown = picture
                .projects
                .iter()
                .filter(|project| words::project_listed(&project.name, &capture.filter))
                .cloned()
                .map(Some)
                .chain(words::inbox_listed(&capture.filter).then_some(None));
            for project in shown {
                next += 1;
                let (name, note, count) = match &project {
                    Some(project) => (
                        project.name.clone(),
                        project.note.display().to_string(),
                        open(&project.note),
                    ),
                    None => (
                        words::INBOX.to_owned(),
                        words::inbox_note(&picture.tasks_note.display().to_string()),
                        open(&picture.tasks_note),
                    ),
                };
                projects.push(CaptureProject {
                    token: next,
                    name,
                    note,
                    open: words::open_count(count),
                    chosen: project == capture.project,
                });
                listed.push((next, project));
            }
        }
        capture.listed = listed;
        let preview = match mode {
            Mode::Task => capture.task().line(),
            Mode::Note => capture.entry().lines().trim_end().to_owned(),
        };
        let view = CaptureView {
            mode,
            from: words::from_line(&capture.source),
            field: mode.field().to_owned(),
            text: capture.text.clone(),
            subject_key: hints::key(&keymap, CommandId::CaptureUseSubject),
            has_due: mode.has_due(),
            due: capture.due,
            due_label: words::due_label(capture.due),
            picks: words::quick_picks(today, capture.due),
            project_title: words::project_title(named.as_deref()),
            project,
            project_note,
            project_key: hints::key(&keymap, CommandId::CaptureChangeProject),
            projects_open: capture.projects_open,
            filter: capture.filter.clone(),
            projects,
            preview_title: mode.preview_title().to_owned(),
            preview,
            footnote: words::footnote(mode, &capture.place(), capture.due),
            button: mode.button().to_owned(),
            button_key: hints::key(&keymap, CommandId::CaptureWrite),
            error: capture.error.clone(),
        };
        self.stamps = next;
        Some(view)
    }

    /// A `postio://` link: look its message up, or say why not.
    pub(crate) fn open_link(&mut self, uri: &str) -> Vec<Step> {
        match postio_ui::links::message(uri) {
            Some(message) => vec![Step::Ask(Request::FindMessage(message))],
            None => vec![Step::Show(Intent::Toast {
                text: postio_ui::links::UNKNOWN.to_owned(),
                kind: ToastKind::Notice,
            })],
        }
    }

    /// The linked message, looked up: opened as the open message, or the
    /// sentence saying it is not on this computer.
    pub(crate) fn link_found(
        &mut self,
        message: MessageId,
        row: Option<postio_model::listing::MessageSummary>,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let Some(row) = row else {
            return vec![Step::Show(Intent::Toast {
                text: postio_ui::links::GONE.to_owned(),
                kind: ToastKind::Notice,
            })];
        };
        let (index, total) = match rows.position_of(message) {
            Some(at) => (at, rows.len()),
            None => (0, 1),
        };
        let facts = RowFacts {
            id: message,
            digest: false,
            threads: row.thread.into_iter().collect(),
            writes: false,
        };
        self.surfaces
            .open(&facts, index, total, self.policy.caps.stacking)
    }
}
