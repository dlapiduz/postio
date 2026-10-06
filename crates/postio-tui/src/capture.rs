//! The capture sheet's state (terminal.md, "Obsidian capture"): `t` or `n`
//! on a message writes a task or a note into the person's vault.
//!
//! The text is the marker's sentence, verbatim, or the subject; the due day
//! has quick picks; the project is suggested from the vault with its reason
//! and `ctrl+p` changes it; and the preview is the exact line
//! `postio-vault` will append. What it reads and writes is
//! `crate::ask::Ask`; the drawing is `view::capture`. The words and rules
//! are `postio_ui::capture`'s.

use chrono::NaiveDate;
use postio_client::protocol::VaultPicture;
use postio_ui::capture::{self, Mode, Pick, Source};
use postio_ui::terminal::SafeText;
use postio_vault::{NoteEntry, Project, Reason, Task};
use tui_input::Input;

/// A control of the sheet the keyboard can be on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The text.
    Text,
    /// The quick picks of the due day, for a task.
    Due,
    /// The project.
    Project,
}

/// The project list `ctrl+p` opens.
#[derive(Debug, Clone)]
pub struct Chooser {
    /// What is typed to narrow the list.
    pub filter: Input,
    /// The row the keyboard is on.
    pub at: usize,
}

/// The sheet. See the module.
#[derive(Debug, Clone)]
pub struct Sheet {
    source: Source,
    mode: Mode,
    text: Input,
    due: Option<NaiveDate>,
    today: NaiveDate,
    picture: Option<VaultPicture>,
    project: Option<Project>,
    suggested: bool,
    field: Field,
    chooser: Option<Chooser>,
    error: Option<String>,
}

impl Sheet {
    /// A sheet for `source`, as a task or a note, on `today`.
    pub fn new(source: Source, mode: Mode, today: NaiveDate) -> Sheet {
        let text = source
            .sentence
            .clone()
            .unwrap_or_else(|| source.subject.clone());
        Sheet {
            due: source.due,
            text: Input::new(SafeText::new(&text).as_str().to_owned()),
            source,
            mode,
            today,
            picture: None,
            project: None,
            suggested: false,
            field: Field::Text,
            chooser: None,
            error: None,
        }
    }

    /// What it is made from.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Task or note.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Switch between a task and a note.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if !mode.has_due() && self.field == Field::Due {
            self.field = Field::Text;
        }
    }

    /// The text, as typed.
    pub fn text(&self) -> &Input {
        &self.text
    }

    /// The text to type into.
    pub fn text_mut(&mut self) -> &mut Input {
        &mut self.text
    }

    /// Put the subject in the text's place.
    pub fn use_subject(&mut self) {
        self.text = Input::new(SafeText::new(&self.source.subject).as_str().to_owned());
    }

    /// The due day chosen.
    pub fn due(&self) -> Option<NaiveDate> {
        self.due
    }

    /// The due day's quick picks.
    pub fn picks(&self) -> Vec<Pick> {
        capture::quick_picks(self.today, self.source.due)
    }

    /// Choose the due day of the pick at `at`.
    pub fn pick(&mut self, at: usize) {
        if let Some(pick) = self.picks().get(at) {
            self.due = pick.day;
        }
    }

    /// Move the chosen pick by `by`, within the picks.
    pub fn pick_by(&mut self, by: isize) {
        let picks = self.picks();
        let at = picks
            .iter()
            .position(|pick| pick.day == self.due)
            .unwrap_or(picks.len() - 1);
        let next = at.saturating_add_signed(by).min(picks.len() - 1);
        self.due = picks[next].day;
    }

    /// The control the keyboard is on.
    pub fn field(&self) -> Field {
        self.field
    }

    /// Move the keyboard to `field` when the sheet has it.
    pub fn focus(&mut self, field: Field) {
        if field != Field::Due || self.mode.has_due() {
            self.field = field;
        }
    }

    /// Walk the controls by `by`.
    pub fn walk(&mut self, by: isize) {
        let fields: Vec<Field> = [Field::Text, Field::Due, Field::Project]
            .into_iter()
            .filter(|field| *field != Field::Due || self.mode.has_due())
            .collect();
        let at = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = (at as isize + by).rem_euclid(fields.len() as isize) as usize;
        self.field = fields[next];
    }

    /// Whether the keyboard is in a field that takes typing.
    pub fn typing(&self) -> bool {
        self.chooser.is_some() || self.field == Field::Text
    }

    /// The vault was read.
    pub fn read(&mut self, picture: VaultPicture) {
        self.suggested = picture.suggestion.is_some();
        self.project = picture
            .suggestion
            .as_ref()
            .map(|suggestion| suggestion.project.clone());
        self.picture = Some(picture);
        self.error = None;
    }

    /// The vault could not be read.
    pub fn say(&mut self, sentence: Option<String>) {
        self.error = sentence;
    }

    /// Why the sheet cannot write, or what went wrong.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the vault has been read.
    pub fn has_vault(&self) -> bool {
        self.picture.is_some()
    }

    /// The project chosen; `None` is the tasks note.
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// The project's name, as the sheet calls it.
    pub fn place(&self) -> String {
        self.project
            .as_ref()
            .map_or_else(|| capture::INBOX.to_owned(), |project| project.name.clone())
    }

    /// The note the project's capture goes to, relative to the vault.
    pub fn note(&self) -> String {
        match (&self.project, &self.picture) {
            (Some(project), _) => project.note.display().to_string(),
            (None, Some(picture)) => capture::inbox_note(&picture.tasks_note.display().to_string()),
            (None, None) => String::new(),
        }
    }

    /// The project's title: why it was suggested while the vault's
    /// suggestion is the one on screen.
    pub fn project_title(&self) -> String {
        let named = self
            .picture
            .as_ref()
            .and_then(|picture| picture.suggestion.as_ref())
            .filter(|_| self.suggested)
            .map(|suggestion| match &suggestion.reason {
                Reason::NamedInSubject(_) => suggestion.project.name.as_str(),
            });
        capture::project_title(named)
    }

    /// The task as it stands.
    pub fn task(&self) -> Task {
        Task {
            text: self.text.value().to_owned(),
            message: self.source.message,
            due: self.due,
        }
    }

    /// The note entry as it stands.
    pub fn entry(&self) -> NoteEntry {
        NoteEntry {
            text: self.text.value().to_owned(),
            message: self.source.message,
            quote: None,
        }
    }

    /// The exact line that will be appended.
    pub fn preview(&self) -> String {
        match self.mode {
            Mode::Task => self.task().line(),
            Mode::Note => self.entry().lines().trim_end().to_owned(),
        }
    }

    /// The note a note entry goes to: the project's, or the tasks note.
    pub fn note_path(&self) -> std::path::PathBuf {
        self.project
            .as_ref()
            .map(|project| project.note.clone())
            .or_else(|| {
                self.picture
                    .as_ref()
                    .map(|picture| picture.tasks_note.clone())
            })
            .unwrap_or_default()
    }

    /// What a written capture says: where it went, and its due day.
    pub fn said(&self) -> String {
        let place = self.place();
        match self.mode {
            Mode::Task => self.due.map_or_else(
                || format!("Task added to {place}"),
                |day| format!("Task added to {place} \u{b7} due {}", day.format("%a")),
            ),
            Mode::Note => format!("Note added to {place}"),
        }
    }

    /// The note a task goes to with no project, relative to the vault.
    pub fn tasks_note(&self) -> String {
        self.picture
            .as_ref()
            .map(|picture| picture.tasks_note.display().to_string())
            .unwrap_or_default()
    }

    /// The project chooser, while it is open.
    pub fn chooser(&self) -> Option<&Chooser> {
        self.chooser.as_ref()
    }

    /// Open the chooser, or put it away.
    pub fn toggle_chooser(&mut self) {
        self.chooser = match self.chooser {
            Some(_) => None,
            None if self.picture.is_some() => Some(Chooser {
                filter: Input::default(),
                at: 0,
            }),
            None => None,
        };
        if self.chooser.is_some() {
            self.field = Field::Project;
        }
    }

    /// The chooser's typing, and its list's rows.
    pub fn chooser_mut(&mut self) -> Option<&mut Chooser> {
        self.chooser.as_mut()
    }

    /// The projects the filter lets through, then the tasks note when it
    /// does: `None` is the Inbox, each with how many of its tasks are open.
    pub fn choices(&self) -> Vec<(Option<Project>, usize)> {
        let (Some(picture), Some(chooser)) = (&self.picture, &self.chooser) else {
            return Vec::new();
        };
        let wanted = chooser.filter.value().to_lowercase();
        let open = |note: &std::path::Path| {
            picture
                .tasks
                .iter()
                .filter(|task| task.note == note && !task.finished)
                .count()
        };
        let mut shown: Vec<(Option<Project>, usize)> = picture
            .projects
            .iter()
            .filter(|project| capture::project_listed(&project.name, &wanted))
            .map(|project| (Some(project.clone()), open(&project.note)))
            .collect();
        if capture::inbox_listed(&wanted) {
            shown.push((None, open(&picture.tasks_note)));
        }
        shown
    }

    /// Move the chooser's keyboard by `by`, within what it lists.
    pub fn choose_by(&mut self, by: isize) {
        let last = self.choices().len().saturating_sub(1);
        if let Some(chooser) = self.chooser.as_mut() {
            chooser.at = chooser.at.saturating_add_signed(by).min(last);
        }
    }

    /// Choose the project listed at `at`, and put the chooser away.
    pub fn choose(&mut self, at: usize) {
        let Some((project, _)) = self.choices().into_iter().nth(at) else {
            return;
        };
        self.suggested = self
            .picture
            .as_ref()
            .and_then(|picture| picture.suggestion.as_ref())
            .is_some_and(|suggestion| Some(&suggestion.project) == project.as_ref());
        self.project = project;
        self.chooser = None;
        self.field = Field::Text;
    }
}
