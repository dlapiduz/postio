//! The capture sheet (spec 007 US15, T158; screen 25): `t` on a message
//! captures a task into the person's Obsidian vault, `n` a note.
//!
//! - **The text** is the action sentence, verbatim, when the message has a
//!   marker that quotes one, and its subject otherwise; `alt+s` puts the
//!   subject in its place.
//! - **The due day** comes from the mail when it names one, with quick
//!   picks beside it.
//! - **The project** is suggested from the vault with its reason, and
//!   `mod+p` changes it.
//! - **The preview** is the exact line `postio-vault` will append, in the
//!   Obsidian Tasks format with the `postio://` link before the date.
//!
//! `mod+Return` writes it, through the host, on this computer. The sheet is
//! a dialog over the window, like every window over the app (FR-092), and
//! its keys are `Context::Capture`'s.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use chrono::{Datelike, NaiveDate, Weekday};
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::VaultPicture;
use postio_core::{CommandId, Keymap};
use postio_model::MessageId;
use postio_ui::hints;
use postio_vault::{NoteEntry, Project, Reason, Task};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};
use postio_widgets::widgets::{Kind, Size};

/// The dialog's widget name, so the window can tell it from another.
pub const DIALOG_NAME: &str = "focus-capture";

/// What is said when `t` or `n` finds no vault to capture into.
pub const NO_VAULT: &str =
    "Name a vault under [focus.vault] in config.toml to capture tasks and notes";

/// The sheet's size: screen 25's.
const WIDTH: i32 = 660;
const HEIGHT: i32 = 600;

/// A task or a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A line in the Obsidian Tasks format.
    Task,
    /// An entry appended to a note.
    Note,
}

/// The message a capture is made from, as the row or the open message says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The message, which the line's link opens.
    pub message: MessageId,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// When it arrived, as a person reads it.
    pub when: String,
    /// The sentence its marker quotes, verbatim, when it has one.
    pub sentence: Option<String>,
    /// The day the mail says it is due, when it says.
    pub due: Option<NaiveDate>,
}

/// What a written capture said, for the window's toast.
type Written = Rc<dyn Fn(String)>;

/// The sheet. See the module.
pub struct CaptureSheet {
    client: Client,
    dialog: adw::Dialog,
    mode: Cell<Mode>,
    source: RefCell<Option<Source>>,
    keymap: RefCell<Keymap>,
    task_toggle: gtk::ToggleButton,
    note_toggle: gtk::ToggleButton,
    cancel: gtk::Button,
    write: gtk::Button,
    from: gtk::Label,
    field_title: gtk::Label,
    entry: gtk::Entry,
    hint: gtk::Box,
    due_box: gtk::Box,
    due_day: gtk::Label,
    picks: gtk::Box,
    due: Cell<Option<NaiveDate>>,
    project_title: gtk::Label,
    project_name: gtk::Label,
    project_note: gtk::Label,
    change: gtk::Button,
    filter: gtk::SearchEntry,
    projects: gtk::ListBox,
    projects_shown: RefCell<Vec<Option<Project>>>,
    preview_title: gtk::Label,
    preview: gtk::Label,
    footnote: gtk::Label,
    picture: RefCell<VaultPicture>,
    project: RefCell<Option<Project>>,
    /// Whether the project on screen is the one the vault suggested.
    suggested: Cell<bool>,
    written: RefCell<Option<Written>>,
    /// Between presenting and closing.
    open: Rc<Cell<bool>>,
    me: RefCell<std::rc::Weak<CaptureSheet>>,
}

impl CaptureSheet {
    /// The commands the sheet has a control for.
    pub fn controls() -> Vec<CommandId> {
        vec![
            CommandId::CaptureTask,
            CommandId::CaptureNote,
            CommandId::CaptureChangeProject,
            CommandId::CaptureUseSubject,
            CommandId::CaptureWrite,
            CommandId::Back,
        ]
    }

    /// A closed sheet writing through `client`, its keys from `keymap`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let cancel = gtk::Button::new();
        postio_widgets::widgets::button::style(&cancel, Kind::Secondary, Size::Small);
        let task_toggle = gtk::ToggleButton::new();
        let note_toggle = gtk::ToggleButton::new();
        note_toggle.set_group(Some(&task_toggle));
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        modes.add_css_class("focus-capture-modes");
        modes.append(&task_toggle);
        modes.append(&note_toggle);
        let write = gtk::Button::new();
        postio_widgets::widgets::button::style(&write, Kind::Primary, Size::Small);
        write.add_css_class("focus-capture-write");
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-capture-header");
        header.set_start_widget(Some(&cancel));
        header.set_center_widget(Some(&modes));
        header.set_end_widget(Some(&write));

        let from = gtk::Label::new(None);
        from.set_xalign(0.0);
        from.add_css_class("dim-label");
        from.set_ellipsize(pango::EllipsizeMode::End);

        let field_title = gtk::Label::new(Some("Task"));
        field_title.set_xalign(0.0);
        field_title.add_css_class("caption");
        let entry = gtk::Entry::new();
        entry.add_css_class("focus-capture-text");
        let hint = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        hint.add_css_class("dim-label");
        let field = gtk::Box::new(gtk::Orientation::Vertical, S1);
        field.add_css_class("focus-capture-field");
        field.append(&field_title);
        field.append(&entry);
        field.append(&hint);

        let due_title = gtk::Label::new(Some("Due"));
        due_title.set_xalign(0.0);
        due_title.add_css_class("caption");
        let due_day = gtk::Label::new(None);
        due_day.set_xalign(0.0);
        due_day.add_css_class("heading");
        let due_words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        due_words.set_hexpand(true);
        due_words.append(&due_title);
        due_words.append(&due_day);
        let picks = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        picks.set_valign(gtk::Align::Center);
        let due_box = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        due_box.add_css_class("focus-capture-due");
        due_box.append(&due_words);
        due_box.append(&picks);

        let project_title = gtk::Label::new(None);
        project_title.set_xalign(0.0);
        project_title.add_css_class("caption");
        project_title.set_wrap(true);
        let project_name = gtk::Label::new(None);
        project_name.set_xalign(0.0);
        project_name.add_css_class("heading");
        let project_note = gtk::Label::new(None);
        project_note.set_xalign(0.0);
        project_note.add_css_class("monospace");
        project_note.add_css_class("dim-label");
        let named = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        named.append(&project_name);
        named.append(&project_note);
        let project_words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        project_words.set_hexpand(true);
        project_words.append(&project_title);
        project_words.append(&named);
        let change = gtk::Button::new();
        postio_widgets::widgets::button::style(&change, Kind::Secondary, Size::Small);
        change.set_valign(gtk::Align::Center);
        let project_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        project_row.append(&project_words);
        project_row.append(&change);
        let filter = gtk::SearchEntry::new();
        filter.set_placeholder_text(Some("Filter projects in your vault"));
        filter.set_visible(false);
        let projects = gtk::ListBox::new();
        projects.add_css_class("boxed-list");
        projects.add_css_class("focus-capture-projects");
        projects.set_visible(false);
        let project_box = gtk::Box::new(gtk::Orientation::Vertical, S2);
        project_box.add_css_class("focus-capture-project");
        project_box.append(&project_row);
        project_box.append(&filter);
        project_box.append(&projects);

        let preview_title = gtk::Label::new(None);
        preview_title.set_xalign(0.0);
        preview_title.add_css_class("heading");
        let preview = gtk::Label::new(None);
        preview.set_xalign(0.0);
        preview.set_wrap(true);
        preview.set_wrap_mode(pango::WrapMode::WordChar);
        preview.set_selectable(true);
        preview.add_css_class("monospace");
        preview.add_css_class("focus-capture-preview");
        let footnote = gtk::Label::new(None);
        footnote.set_xalign(0.0);
        footnote.set_wrap(true);
        footnote.add_css_class("dim-label");
        footnote.add_css_class("caption");

        let body = gtk::Box::new(gtk::Orientation::Vertical, S3);
        body.add_css_class("focus-capture-body");
        body.set_margin_start(S3);
        body.set_margin_end(S3);
        body.set_margin_top(S3);
        body.set_margin_bottom(S3);
        body.append(&from);
        body.append(&field);
        body.append(&due_box);
        body.append(&project_box);
        body.append(&preview_title);
        body.append(&preview);
        body.append(&footnote);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&body)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-capture");
        content.append(&header);
        content.append(&scrolled);
        let dialog = adw::Dialog::builder()
            .title("Capture")
            .content_width(WIDTH)
            .content_height(HEIGHT)
            .child(&content)
            .build();
        dialog.set_widget_name(DIALOG_NAME);

        let sheet = Rc::new(CaptureSheet {
            client,
            dialog,
            mode: Cell::new(Mode::Task),
            source: RefCell::default(),
            keymap: RefCell::new(keymap.clone()),
            task_toggle,
            note_toggle,
            cancel,
            write,
            from,
            field_title,
            entry,
            hint,
            due_box,
            due_day,
            picks,
            due: Cell::default(),
            project_title,
            project_name,
            project_note,
            change,
            filter,
            projects,
            projects_shown: RefCell::default(),
            preview_title,
            preview,
            footnote,
            picture: RefCell::default(),
            project: RefCell::default(),
            suggested: Cell::new(false),
            written: RefCell::default(),
            open: Rc::default(),
            me: RefCell::default(),
        });
        sheet.me.replace(Rc::downgrade(&sheet));
        sheet.dialog.connect_closed({
            let open = Rc::clone(&sheet.open);
            move |_| open.set(false)
        });
        sheet.wire();
        sheet.set_keymap(keymap);
        sheet
    }

    fn weak(&self) -> std::rc::Weak<CaptureSheet> {
        self.me.borrow().clone()
    }

    fn wire(&self) {
        let weak = self.weak();
        let each = move |command: CommandId| {
            let weak = weak.clone();
            move || {
                if let Some(sheet) = weak.upgrade() {
                    sheet.run(command);
                }
            }
        };
        let back = each(CommandId::Back);
        self.cancel.connect_clicked(move |_| back());
        let write = each(CommandId::CaptureWrite);
        self.write.connect_clicked(move |_| write());
        let change = each(CommandId::CaptureChangeProject);
        self.change.connect_clicked(move |_| change());
        for (toggle, mode) in [
            (&self.task_toggle, Mode::Task),
            (&self.note_toggle, Mode::Note),
        ] {
            let weak = self.weak();
            toggle.connect_toggled(move |toggle| {
                if toggle.is_active()
                    && let Some(sheet) = weak.upgrade()
                    && sheet.mode.get() != mode
                {
                    sheet.set_mode(mode);
                }
            });
        }
        let weak = self.weak();
        self.entry.connect_changed(move |_| {
            if let Some(sheet) = weak.upgrade() {
                sheet.show_preview();
            }
        });
        let weak = self.weak();
        self.filter.connect_search_changed(move |_| {
            if let Some(sheet) = weak.upgrade() {
                sheet.list_projects();
            }
        });
        let weak = self.weak();
        self.projects.connect_row_activated(move |_, row| {
            if let Some(sheet) = weak.upgrade() {
                sheet.choose_project_at(row.index());
            }
        });
        // Enter in the filter takes the first project it shows.
        let weak = self.weak();
        self.filter.connect_activate(move |_| {
            if let Some(sheet) = weak.upgrade() {
                sheet.choose_project_at(0);
            }
        });
    }

    /// Run `handler` with what a written capture says.
    pub fn connect_written(&self, handler: impl Fn(String) + 'static) {
        self.written.replace(Some(Rc::new(handler)));
    }

    /// Read every key the sheet shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        let key = |command| hints::key(keymap, command);
        self.cancel.set_child(Some(&keyhint::labelled(
            "Cancel",
            key(CommandId::Back).as_deref(),
        )));
        self.task_toggle.set_child(Some(&keyhint::labelled(
            "Task",
            key(CommandId::CaptureTask).as_deref(),
        )));
        self.note_toggle.set_child(Some(&keyhint::labelled(
            "Note",
            key(CommandId::CaptureNote).as_deref(),
        )));
        self.change.set_child(Some(&keyhint::labelled(
            "Change",
            key(CommandId::CaptureChangeProject).as_deref(),
        )));
        while let Some(child) = self.hint.first_child() {
            self.hint.remove(&child);
        }
        self.hint.append(&gtk::Label::new(Some(
            "The sentence from the mail, as written \u{b7}",
        )));
        // A control, as every key is (constitution II): what `alt+s` does.
        let use_subject = gtk::Button::new();
        postio_widgets::widgets::button::style(&use_subject, Kind::Ghost, Size::Small);
        use_subject.set_child(Some(&keyhint::labelled(
            "use the subject instead",
            key(CommandId::CaptureUseSubject).as_deref(),
        )));
        let weak = self.weak();
        use_subject.connect_clicked(move |_| {
            if let Some(sheet) = weak.upgrade() {
                sheet.run(CommandId::CaptureUseSubject);
            }
        });
        self.hint.append(&use_subject);
        self.show_mode();
    }

    /// Open over `parent` for `source`, as a task or a note, and read the
    /// vault's projects for it.
    pub fn open(&self, parent: &impl IsA<gtk::Widget>, source: Source, mode: Mode) {
        let text = source
            .sentence
            .clone()
            .unwrap_or_else(|| source.subject.clone());
        self.from.set_text(&format!(
            "From {} \u{b7} {} \u{b7} {}",
            source.sender, source.subject, source.when
        ));
        self.due.set(source.due);
        self.project.replace(None);
        self.suggested.set(false);
        self.picture.replace(VaultPicture::default());
        self.filter.set_text("");
        self.filter.set_visible(false);
        self.projects.set_visible(false);
        let subject = source.subject.clone();
        self.source.replace(Some(source));
        self.entry.set_text(&text);
        self.set_mode(mode);
        self.show_project();
        self.open.set(true);
        self.dialog.present(Some(parent));
        // The keyboard in the text, the text left as it is: selecting it
        // all would let the first key typed replace the sentence.
        self.entry.grab_focus_without_selecting();
        self.read_vault(subject);
    }

    /// Read the vault's projects, the suggestion and the open tasks.
    fn read_vault(&self, subject: String) {
        let client = self.client.clone();
        let weak = self.weak();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // reads the vault on a thread of its own (ADR 0041).
            let read = client.vault(&subject).await;
            let Some(sheet) = weak.upgrade() else {
                return;
            };
            match read {
                Ok(picture) => {
                    let suggested = picture
                        .suggestion
                        .as_ref()
                        .map(|suggestion| suggestion.project.clone());
                    sheet.suggested.set(suggested.is_some());
                    sheet.project.replace(suggested);
                    sheet.picture.replace(picture);
                    sheet.show_project();
                    sheet.list_projects();
                }
                Err(error) => sheet.project_title.set_text(&error.to_string()),
            }
        });
    }

    /// Run `command`, as its key or its control asked.
    pub fn run(&self, command: CommandId) {
        match command {
            CommandId::Back => self.close(),
            CommandId::CaptureTask => self.set_mode(Mode::Task),
            CommandId::CaptureNote => self.set_mode(Mode::Note),
            CommandId::CaptureUseSubject => {
                if let Some(source) = self.source.borrow().clone() {
                    self.entry.set_text(&source.subject);
                }
            }
            CommandId::CaptureChangeProject => {
                let shown = !self.projects.is_visible();
                self.filter.set_visible(shown);
                self.projects.set_visible(shown);
                if shown {
                    self.list_projects();
                    self.filter.grab_focus();
                } else {
                    self.entry.grab_focus();
                }
            }
            CommandId::CaptureWrite => self.write(),
            _ => {}
        }
    }

    fn set_mode(&self, mode: Mode) {
        self.mode.set(mode);
        match mode {
            Mode::Task => self.task_toggle.set_active(true),
            Mode::Note => self.note_toggle.set_active(true),
        }
        self.show_mode();
    }

    /// What depends on task or note: the heading, the due row, the button,
    /// and the preview.
    fn show_mode(&self) {
        let keymap = self.keymap.borrow().clone();
        let (field, button, preview) = match self.mode.get() {
            Mode::Task => ("Task", "Add task", "This exact line will be appended"),
            Mode::Note => ("Note", "Add note", "This exact entry will be appended"),
        };
        self.field_title.set_text(field);
        self.write.set_child(Some(&keyhint::labelled(
            button,
            hints::key(&keymap, CommandId::CaptureWrite).as_deref(),
        )));
        self.preview_title.set_text(preview);
        self.due_box.set_visible(self.mode.get() == Mode::Task);
        self.show_due();
        self.show_preview();
    }

    /// The due day and its quick picks: today, the coming Monday,
    /// Wednesday and Friday, the mail's own day among them, and none.
    fn show_due(&self) {
        while let Some(child) = self.picks.first_child() {
            self.picks.remove(&child);
        }
        let today = postio_ui::clock::now().date_naive();
        let mut days = vec![today];
        for weekday in [Weekday::Mon, Weekday::Wed, Weekday::Fri] {
            days.push(next(today, weekday));
        }
        if let Some(due) = self.due.get()
            && !days.contains(&due)
        {
            days.push(due);
        }
        days.sort();
        let chosen = self.due.get();
        let mut picks: Vec<(String, Option<NaiveDate>)> = days
            .into_iter()
            .map(|day| {
                let words = if day == today {
                    "Today".to_owned()
                } else {
                    day.format("%a").to_string()
                };
                (words, Some(day))
            })
            .collect();
        picks.push(("None".to_owned(), None));
        for (words, day) in picks {
            let pick = gtk::ToggleButton::with_label(&words);
            // The day chosen is ringed, the rest are quiet (screen 25).
            let kind = if day == chosen {
                Kind::Secondary
            } else {
                Kind::Ghost
            };
            postio_widgets::widgets::button::style(&pick, kind, Size::Small);
            pick.add_css_class("focus-capture-pick");
            pick.set_active(day == chosen);
            let weak = self.weak();
            pick.connect_clicked(move |_| {
                if let Some(sheet) = weak.upgrade() {
                    sheet.due.set(day);
                    sheet.show_due();
                    sheet.show_preview();
                }
            });
            self.picks.append(&pick);
        }
        self.due_day.set_text(&match chosen {
            Some(day) => day.format("%A, %-d %B %Y").to_string(),
            None => "No due date".to_owned(),
        });
    }

    /// The project chosen, why, and where its note is.
    fn show_project(&self) {
        let picture = self.picture.borrow();
        let project = self.project.borrow().clone();
        let why = picture
            .suggestion
            .as_ref()
            .filter(|_| self.suggested.get())
            .map(|suggestion| match &suggestion.reason {
                // The word as the subject has it, which the vault read
                // folded: the project's own name says it as written.
                Reason::NamedInSubject(_) => format!(
                    "Project \u{b7} suggested: the subject names {}",
                    suggestion.project.name
                ),
            });
        self.project_title
            .set_text(why.as_deref().unwrap_or("Project"));
        match project {
            Some(project) => {
                self.project_name.set_text(&project.name);
                self.project_note
                    .set_text(&project.note.display().to_string());
            }
            None => {
                self.project_name.set_text("Inbox");
                self.project_note
                    .set_text(&format!("{} (no project)", picture.tasks_note.display()));
            }
        }
        drop(picture);
        self.show_preview();
    }

    /// The projects the filter lets through, and the tasks note.
    fn list_projects(&self) {
        while let Some(row) = self.projects.first_child() {
            self.projects.remove(&row);
        }
        let picture = self.picture.borrow();
        let wanted = self.filter.text().to_lowercase();
        let open = |note: &std::path::Path| {
            picture
                .tasks
                .iter()
                .filter(|task| task.note == note && !task.finished)
                .count()
        };
        let mut shown: Vec<Option<Project>> = picture
            .projects
            .iter()
            .filter(|project| project.name.to_lowercase().contains(&wanted))
            .cloned()
            .map(Some)
            .collect();
        if "inbox".contains(&wanted) {
            shown.push(None);
        }
        let chosen = self.project.borrow().clone();
        for project in &shown {
            let (name, note) = match project {
                Some(project) => (project.name.clone(), project.note.display().to_string()),
                None => (
                    "Inbox".to_owned(),
                    format!("{} (no project)", picture.tasks_note.display()),
                ),
            };
            let count = open(
                project
                    .as_ref()
                    .map_or(picture.tasks_note.as_path(), |project| {
                        project.note.as_path()
                    }),
            );
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&name))
                .subtitle(glib::markup_escape_text(&note))
                .activatable(true)
                .build();
            let count = gtk::Label::new(Some(&format!("{count} open")));
            count.add_css_class("dim-label");
            row.add_suffix(&count);
            if *project == chosen {
                row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            self.projects.append(&row);
        }
        drop(picture);
        self.projects_shown.replace(shown);
    }

    /// Choose the project listed at `index`, and put the list away.
    fn choose_project_at(&self, index: i32) {
        let chosen = usize::try_from(index)
            .ok()
            .and_then(|index| self.projects_shown.borrow().get(index).cloned());
        let Some(project) = chosen else {
            return;
        };
        let suggested = self
            .picture
            .borrow()
            .suggestion
            .as_ref()
            .is_some_and(|suggestion| Some(&suggestion.project) == project.as_ref());
        self.suggested.set(suggested);
        self.project.replace(project);
        self.filter.set_visible(false);
        self.projects.set_visible(false);
        self.show_project();
        self.entry.grab_focus();
    }

    /// The note a note entry goes to: the project's, or the tasks note.
    fn note_path(&self) -> std::path::PathBuf {
        self.project
            .borrow()
            .as_ref()
            .map(|project| project.note.clone())
            .unwrap_or_else(|| self.picture.borrow().tasks_note.clone())
    }

    /// The task as it stands.
    fn task(&self) -> Option<Task> {
        let source = self.source.borrow();
        let source = source.as_ref()?;
        Some(Task {
            text: self.entry.text().to_string(),
            message: source.message,
            due: self.due.get(),
        })
    }

    /// The note entry as it stands.
    fn entry(&self) -> Option<NoteEntry> {
        let source = self.source.borrow();
        let source = source.as_ref()?;
        Some(NoteEntry {
            text: self.entry.text().to_string(),
            message: source.message,
            quote: None,
        })
    }

    /// The exact line that will be appended.
    fn show_preview(&self) {
        let line = match self.mode.get() {
            Mode::Task => self.task().map(|task| task.line()),
            Mode::Note => self
                .entry()
                .map(|entry| entry.lines().trim_end().to_owned()),
        };
        self.preview.set_text(line.as_deref().unwrap_or_default());
        let place = self
            .project
            .borrow()
            .as_ref()
            .map_or_else(|| "Inbox".to_owned(), |project| project.name.clone());
        self.footnote.set_text(&match self.mode.get() {
            Mode::Task => format!(
                "Plain markdown, Obsidian Tasks format, written on this computer. It goes in {place}{}.",
                self.due
                    .get()
                    .map(|day| format!(", due {}", day.format("%a")))
                    .unwrap_or_default()
            ),
            Mode::Note => {
                format!("Plain markdown, written on this computer. It goes in {place}'s note.")
            }
        });
    }

    /// Append what the sheet shows to the vault, through the host; close
    /// and say so once it is written.
    fn write(&self) {
        let project = self.project.borrow().clone();
        let place = project
            .as_ref()
            .map_or_else(|| "Inbox".to_owned(), |project| project.name.clone());
        let client = self.client.clone();
        let weak = self.weak();
        match self.mode.get() {
            Mode::Task => {
                let Some(task) = self.task() else {
                    return;
                };
                let due = task.due;
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    let written = client.capture_task(project, task).await;
                    if let Some(sheet) = weak.upgrade() {
                        let said = due.map_or_else(
                            || format!("Task added to {place}"),
                            |day| format!("Task added to {place} \u{b7} due {}", day.format("%a")),
                        );
                        sheet.done(written.map(|_| said));
                    }
                });
            }
            Mode::Note => {
                let Some(entry) = self.entry() else {
                    return;
                };
                let note = self.note_path();
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    let written = client.capture_note(note, entry).await;
                    if let Some(sheet) = weak.upgrade() {
                        sheet.done(written.map(|_| format!("Note added to {place}")));
                    }
                });
            }
        }
    }

    fn done(&self, written: Result<String, postio_model::listing::StoreError>) {
        let said = match written {
            Ok(said) => {
                self.close();
                said
            }
            Err(error) => error.to_string(),
        };
        let handler = self.written.borrow().clone();
        if let Some(handler) = handler {
            handler(said);
        }
    }

    /// Close, writing nothing.
    pub fn close(&self) {
        self.dialog.close();
    }

    /// Whether the sheet is over the window.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Whether the sheet is up and drawn.
    pub fn is_shown(&self) -> bool {
        self.dialog
            .child()
            .is_some_and(|child| child.is_mapped() && child.width() > 0)
    }

    /// The text to capture, as it stands.
    pub fn text(&self) -> String {
        self.entry.text().to_string()
    }

    /// The exact line the preview shows.
    pub fn preview(&self) -> String {
        self.preview.text().to_string()
    }

    /// Every label the sheet shows, in order.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.dialog.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                let text = label.text().to_string();
                if !text.is_empty() {
                    said.push(text);
                }
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        said
    }

    /// The dialog.
    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}

/// The first `weekday` after `today`.
fn next(today: NaiveDate, weekday: Weekday) -> NaiveDate {
    let ahead = (7 + weekday.num_days_from_monday() as i64
        - today.weekday().num_days_from_monday() as i64)
        % 7;
    let ahead = if ahead == 0 { 7 } else { ahead };
    today + chrono::Duration::days(ahead)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quick_picks_name_the_coming_weekday_never_today() {
        let saturday = NaiveDate::from_ymd_opt(2026, 9, 26).expect("a day");
        assert_eq!(
            next(saturday, Weekday::Wed),
            NaiveDate::from_ymd_opt(2026, 9, 30).expect("a day")
        );
        assert_eq!(
            next(saturday, Weekday::Sat),
            NaiveDate::from_ymd_opt(2026, 10, 3).expect("a day"),
            "a week on, not today"
        );
    }
}
