//! Capture: a task or note written to the vault from a message, and the `postio://` links back.
//! Capture opens only when a vault is configured, and says how to name one otherwise.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureView {
    pub mode: Mode,
    pub from: String,
    pub field: String,
    /// The field's text: set the field from it only when it differs.
    pub text: String,
    pub subject_key: Option<String>,
    pub has_due: bool,
    pub due: Option<NaiveDate>,
    pub due_label: String,
    pub picks: Vec<Pick>,
    pub project_title: String,
    pub project: String,
    pub project_note: String,
    pub project_key: Option<String>,
    pub projects_open: bool,
    pub filter: String,
    pub projects: Vec<CaptureProject>,
    pub preview_title: String,
    pub preview: String,
    pub footnote: String,
    pub button: String,
    pub button_key: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureProject {
    /// What [`crate::Input::CaptureProject`] names it by. Never reused.
    pub token: u64,
    pub name: String,
    pub note: String,
    pub open: String,
    pub chosen: bool,
}

#[derive(Debug)]
pub(crate) struct Capture {
    mode: Mode,
    source: Source,
    text: String,
    due: Option<NaiveDate>,
    picture: Option<VaultPicture>,
    project: Option<Project>,
    suggested: bool,
    projects_open: bool,
    filter: String,
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

    /// While capture is on top it holds the keyboard: what is not its own does nothing, but the key
    /// map and Quit.
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

    fn close_capture(&mut self) -> Vec<Step> {
        self.capture = None;
        self.surfaces.dismiss(SurfaceKind::Capture);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Capture))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

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

    pub(crate) fn open_link(&mut self, uri: &str) -> Vec<Step> {
        match postio_ui::links::message(uri) {
            Some(message) => vec![Step::Ask(Request::FindMessage(message))],
            None => vec![Step::Show(Intent::Toast {
                text: postio_ui::links::UNKNOWN.to_owned(),
                kind: ToastKind::Notice,
            })],
        }
    }

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
