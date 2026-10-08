//! Filtered, the digest window, the rule sheet and capture at the boundary
//! (specs/009-focus-macos T111, for the Mac's T113-T116).
//!
//! The surfaces are the controller's (`postio_focus`, slice 12): what each
//! holds, what each key does, which read is current. The Mac draws what it
//! is told -- `FocusFiltered`, `FocusDigest`, `FocusOpenRule`/`FocusRule`,
//! `FocusOpenCapture`/`FocusCapture`, each a whole view -- and reports what
//! the person did that is not a key: a click on a row, text typed, a day or
//! a project chosen. The keys (`1`-`7`, `R`, `]`/`[`, Tab, `D`, `U`, `⇧A`,
//! `d`, `t`/`n`, `⌥S`, `⌘P`, `⌘↩`, Esc) are commands `key` resolves in the
//! surface's context and `invoke` runs.
//!
//! Three reads cross as well, for what Swift may want without a surface: a
//! digest's summary, the vault, and a task written straight to it.

use crate::session::{Session, SessionError, blocking};

/// One key hint: the key as the keymap in force spells it, and its words.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusHintFfi {
    /// "R", "1–7", "Return".
    pub key: String,
    /// "restore + never filter sender".
    pub label: String,
}

impl From<postio_ui::hints::Hint> for FocusHintFfi {
    fn from(hint: postio_ui::hints::Hint) -> Self {
        FocusHintFfi {
            key: hint.key,
            label: hint.label,
        }
    }
}

/// Filtered, drawn whole (screen 21). It replaces the list in the main
/// window; the controller has put it on the stack.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredViewFfi {
    /// "Filtered".
    pub title: String,
    /// "Archived automatically · newest first".
    pub subtitle: String,
    /// Right of the tabs: "Nothing here is deleted automatically · …" (C4).
    pub note: String,
    /// "Sweep the inbox…": a click is `invoke("sweep_inbox")`.
    pub sweep: String,
    /// Its key.
    pub sweep_key: Option<String>,
    /// The focused row's button: a click is `invoke("restore_filtered")`.
    pub restore: String,
    /// Its key.
    pub restore_key: Option<String>,
    /// The tabs, `1` to `7`: a click is `invoke("filtered_tab_<n>")`.
    pub tabs: Vec<FilteredTabFfi>,
    /// The rows read so far, newest first.
    pub rows: Vec<FilteredLineFfi>,
    /// The row with the keyboard.
    pub focused: Option<u32>,
    /// Whether there may be more: `focus_filtered_more()` at the end.
    pub more: bool,
    /// The footer's hints.
    pub footer: Vec<FocusHintFfi>,
}

/// One of Filtered's tabs.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredTabFfi {
    /// "All", "Spam".
    pub name: String,
    /// How many it holds.
    pub count: u32,
    /// The number key that shows it.
    pub key: Option<String>,
    /// Whether it is the one showing.
    pub on: bool,
}

/// One filtered message.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredLineFfi {
    /// The message.
    pub message: i64,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// Its first line.
    pub preview: Option<String>,
    /// The reason pill: "notification · Forge".
    pub pill: String,
    /// When it was filed.
    pub time: String,
    /// The day heading above it, on the first row of each day.
    pub heading: Option<String>,
}

impl From<postio_focus::FilteredView> for FilteredViewFfi {
    fn from(view: postio_focus::FilteredView) -> Self {
        FilteredViewFfi {
            title: view.title,
            subtitle: view.subtitle,
            note: view.note,
            sweep: view.sweep,
            sweep_key: view.sweep_key,
            restore: view.restore,
            restore_key: view.restore_key,
            tabs: view
                .tabs
                .into_iter()
                .map(|tab| FilteredTabFfi {
                    name: tab.name,
                    count: tab.count,
                    key: tab.key,
                    on: tab.on,
                })
                .collect(),
            rows: view
                .rows
                .into_iter()
                .map(|row| FilteredLineFfi {
                    message: row.message.get(),
                    sender: row.sender,
                    subject: row.subject,
                    preview: row.preview,
                    pill: row.pill,
                    time: row.time,
                    heading: row.heading,
                })
                .collect(),
            focused: view.focused,
            more: view.more,
            footer: view.footer.into_iter().map(Into::into).collect(),
        }
    }
}

/// What the digest's window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DigestPageFfi {
    /// The summary by topic (screen 22).
    Summary,
    /// The plain list of its messages.
    List,
    /// An email, in place (screen 23).
    Email,
}

impl From<postio_focus::DigestPage> for DigestPageFfi {
    fn from(page: postio_focus::DigestPage) -> Self {
        match page {
            postio_focus::DigestPage::Summary => DigestPageFfi::Summary,
            postio_focus::DigestPage::List => DigestPageFfi::List,
            postio_focus::DigestPage::Email => DigestPageFfi::Email,
        }
    }
}

/// The digest's window, drawn whole (screens 22 and 23), at the message
/// window's size with a 560 column (M1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestViewFfi {
    /// The delivery, as `FocusOpenDigest` named it.
    pub delivery: i64,
    /// The page showing.
    pub page: DigestPageFfi,
    /// "Weekly · Newsletters"; the email's subject on the email page.
    pub title: String,
    /// "14 messages from 6 senders · came due today 16:00"; "Source 2 of
    /// 14" on the email page.
    pub subtitle: String,
    /// "Archive all 14": a click is `invoke("archive_thread")`.
    pub archive: String,
    /// Its key.
    pub archive_key: Option<String>,
    /// "Weekly, Sunday 09:00 · Edit rule and cadence": a click is
    /// `invoke("digest_rule")`.
    pub rule_line: String,
    /// Its key.
    pub rule_key: Option<String>,
    /// Whether the Summary and list tabs show.
    pub tabs: bool,
    /// The list tab's words: "14 messages".
    pub list_tab: String,
    /// The key that switches: a click on a tab is
    /// `invoke("toggle_digest_summary")`.
    pub tab_key: Option<String>,
    /// The plain list.
    pub rows: Vec<DigestLineFfi>,
    /// Its row with the keyboard: what `D` and `U` act on.
    pub focused: Option<u32>,
    /// The summary, by topic.
    pub topics: Vec<DigestTopicFfi>,
    /// The focused reference, in reading order (`DigestStatementFfi.index`).
    pub focused_reference: Option<u32>,
    /// The card under the paragraphs.
    pub card: Option<DigestCardFfi>,
    /// The line under the summary.
    pub footer: Option<String>,
    /// The email on the email page: draw `reader_document(email.message)`
    /// in the window, the banner over it, `excerpt` highlighted.
    pub email: Option<DigestEmailFfi>,
    /// The email page's "‹ Summary" (or "‹ 3 messages"): a click is
    /// `invoke("back")`.
    pub back: String,
    /// Its key.
    pub back_key: Option<String>,
    /// Whether what it holds is still being read.
    pub loading: bool,
}

/// One message of the digest's plain list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestLineFfi {
    /// The message.
    pub message: i64,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// Its first line.
    pub preview: Option<String>,
    /// When it came.
    pub time: String,
}

/// A topic of the summary.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestTopicFfi {
    /// "Rates · 3 statements from 2 messages".
    pub heading: String,
    /// Its statements, in reading order.
    pub statements: Vec<DigestStatementFfi>,
}

/// One statement, ending in its numbered reference.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestStatementFfi {
    /// Its place among every statement: what `focus_digest_reference`
    /// takes, and `focused_reference` names.
    pub index: u32,
    /// Plain text, never markup (FR-174).
    pub text: String,
    /// Its reference's number, drawn as a chip after it.
    pub number: u32,
    /// The message it cites.
    pub message: i64,
}

/// The card for the focused reference.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestCardFfi {
    /// "Reference 3 · Ada Moreno · The rate decision".
    pub title: String,
    /// "open the full email".
    pub hint: String,
    /// Its key.
    pub key: Option<String>,
}

/// The email shown in the digest's window.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestEmailFfi {
    /// The message.
    pub message: i64,
    /// The reference that cites it, when one does.
    pub number: Option<u32>,
    /// The passage to highlight.
    pub excerpt: Option<String>,
    /// The banner over it.
    pub banner: Option<String>,
}

impl From<postio_focus::DigestView> for DigestViewFfi {
    fn from(view: postio_focus::DigestView) -> Self {
        DigestViewFfi {
            delivery: view.delivery.get(),
            page: view.page.into(),
            title: view.title,
            subtitle: view.subtitle,
            archive: view.archive,
            archive_key: view.archive_key,
            rule_line: view.rule_line,
            rule_key: view.rule_key,
            tabs: view.tabs,
            list_tab: view.list_tab,
            tab_key: view.tab_key,
            rows: view
                .rows
                .into_iter()
                .map(|row| DigestLineFfi {
                    message: row.message.get(),
                    sender: row.sender,
                    subject: row.subject,
                    preview: row.preview,
                    time: row.time,
                })
                .collect(),
            focused: view.focused,
            topics: view
                .topics
                .into_iter()
                .map(|topic| DigestTopicFfi {
                    heading: topic.heading,
                    statements: topic
                        .statements
                        .into_iter()
                        .map(|statement| DigestStatementFfi {
                            index: statement.index,
                            text: statement.text,
                            number: statement.number,
                            message: statement.message.get(),
                        })
                        .collect(),
                })
                .collect(),
            focused_reference: view.focused_reference,
            card: view.card.map(|card| DigestCardFfi {
                title: card.title,
                hint: card.hint,
                key: card.key,
            }),
            footer: view.footer,
            email: view.email.map(|email| DigestEmailFfi {
                message: email.message.get(),
                number: email.number,
                excerpt: email.excerpt,
                banner: email.banner,
            }),
            back: view.back,
            back_key: view.back_key,
            loading: view.loading,
        }
    }
}

/// A question to ask before doing something no single undo takes back
/// whole: an alert with Cancel and one button.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConfirmFfi {
    /// What `focus_confirmed` names it by.
    pub token: u64,
    /// The heading.
    pub heading: String,
    /// The sentence under it.
    pub body: String,
    /// The button that says yes.
    pub confirm: String,
    /// Draw the button as destructive rather than as the default.
    pub destructive: bool,
}

impl From<postio_focus::Confirm> for ConfirmFfi {
    fn from(confirm: postio_focus::Confirm) -> Self {
        ConfirmFfi {
            token: confirm.token,
            heading: confirm.heading,
            body: confirm.body,
            confirm: confirm.confirm,
            destructive: confirm.destructive,
        }
    }
}

/// The rule's schedule, as the sheet's controls hold it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleScheduleFfi {
    /// Index into `RuleViewFfi.cadences`.
    pub cadence: u32,
    /// Index into `RuleViewFfi.weekdays`, for a weekly rule.
    pub weekday: u32,
    /// The day of the month from 0, for a monthly rule (0 to 27).
    pub month_day: u32,
    /// The time, 24-hour "HH:MM".
    pub at: String,
}

impl From<postio_focus::RuleSchedule> for RuleScheduleFfi {
    fn from(schedule: postio_focus::RuleSchedule) -> Self {
        RuleScheduleFfi {
            cadence: schedule.cadence as u32,
            weekday: schedule.weekday as u32,
            month_day: schedule.month_day as u32,
            at: schedule.at,
        }
    }
}

impl From<RuleScheduleFfi> for postio_focus::RuleSchedule {
    fn from(schedule: RuleScheduleFfi) -> Self {
        postio_focus::RuleSchedule {
            cadence: schedule.cadence as usize,
            weekday: schedule.weekday as usize,
            month_day: schedule.month_day as usize,
            at: schedule.at,
        }
    }
}

/// The digest-this-sender sheet, drawn whole (screen 24).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleViewFfi {
    /// "Digest this sender", "Digest rule · Newsletters".
    pub heading: String,
    /// "From".
    pub from_label: String,
    /// The senders, as a plain line; `None` once the query field is up.
    pub from: Option<String>,
    /// The query field's text while it is up: set the field from it only
    /// when it differs.
    pub query: Option<String>,
    /// The query field's placeholder.
    pub placeholder: String,
    /// "Match a list or a search instead…": a click is
    /// `focus_rule_match_instead()`.
    pub match_instead: Option<String>,
    /// "Digest mail like this": a click is `focus_rule_like_this()`.
    pub like_this: Option<String>,
    /// "Deliver".
    pub deliver_label: String,
    /// The schedule.
    pub schedule: RuleScheduleFfi,
    /// "Daily", "Weekly", "Monthly".
    pub cadences: Vec<String>,
    /// "Monday" to "Sunday".
    pub weekdays: Vec<String>,
    /// What still comes straight to the inbox.
    pub note: String,
    /// "Create" or "Save": a click is `focus_rule_create()`.
    pub create: String,
    /// Its key.
    pub create_key: Option<String>,
    /// "Would have caught 9 messages in the last 90 days", once read.
    pub preview_heading: Option<String>,
    /// The newest of them.
    pub preview: Vec<RulePreviewLineFfi>,
    /// "and 5 more".
    pub more: Option<String>,
    /// Why the rule cannot be written as it stands.
    pub error: Option<String>,
}

/// One message the rule would have caught.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulePreviewLineFfi {
    /// Its subject.
    pub subject: String,
    /// "Today", "Tue", "12 Sep".
    pub day: String,
}

impl From<postio_focus::RuleView> for RuleViewFfi {
    fn from(view: postio_focus::RuleView) -> Self {
        RuleViewFfi {
            heading: view.heading,
            from_label: view.from_label,
            from: view.from,
            query: view.query,
            placeholder: view.placeholder,
            match_instead: view.match_instead,
            like_this: view.like_this,
            deliver_label: view.deliver_label,
            schedule: view.schedule.into(),
            cadences: view.cadences,
            weekdays: view.weekdays,
            note: view.note,
            create: view.create,
            create_key: view.create_key,
            preview_heading: view.preview_heading,
            preview: view
                .preview
                .into_iter()
                .map(|line| RulePreviewLineFfi {
                    subject: line.subject,
                    day: line.day,
                })
                .collect(),
            more: view.more,
            error: view.error,
        }
    }
}

/// A task or a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CaptureModeFfi {
    /// A line in the Obsidian Tasks format.
    Task,
    /// An entry appended to a note.
    Note,
}

/// One quick pick for the due day.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CapturePickFfi {
    /// "Today", "Mon", "None".
    pub words: String,
    /// The day, "YYYY-MM-DD"; `None` for no due day.
    pub day: Option<String>,
}

/// One project the capture can go to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CaptureProjectFfi {
    /// What `focus_capture_project` takes.
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

/// The capture window, drawn whole (screen 25).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CaptureViewFfi {
    /// Task or note: the segmented control, whose clicks are
    /// `invoke("capture_task")` and `invoke("capture_note")`.
    pub mode: CaptureModeFfi,
    /// "From Ada · Quarterly review · Mon".
    pub from: String,
    /// "Task", "Note".
    pub field: String,
    /// The text, verbatim: set the field from it only when it differs.
    pub text: String,
    /// `⌥S`'s keycap.
    pub subject_key: Option<String>,
    /// Whether the due row shows.
    pub has_due: bool,
    /// The due day, "YYYY-MM-DD".
    pub due: Option<String>,
    /// "Friday, 2 October 2026", "No due date".
    pub due_label: String,
    /// The quick picks: a click is `focus_capture_due(pick.day)`.
    pub picks: Vec<CapturePickFfi>,
    /// "Project · suggested: the subject names Review".
    pub project_title: String,
    /// The project chosen, or "Inbox".
    pub project: String,
    /// Its note.
    pub project_note: String,
    /// `⌘P`'s keycap: `invoke("capture_change_project")` opens the list.
    pub project_key: Option<String>,
    /// Whether the project list is open.
    pub projects_open: bool,
    /// Its filter's text.
    pub filter: String,
    /// The projects the filter lets through, then the Inbox.
    pub projects: Vec<CaptureProjectFfi>,
    /// "This exact line will be appended".
    pub preview_title: String,
    /// The exact line written.
    pub preview: String,
    /// Where it goes and when it is due.
    pub footnote: String,
    /// "Add task", "Add note": a click is `invoke("capture_write")`.
    pub button: String,
    /// Its key.
    pub button_key: Option<String>,
    /// Why the vault could not be read or written.
    pub error: Option<String>,
}

/// A day, as it crosses: "YYYY-MM-DD".
fn day(day: chrono::NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

impl From<postio_focus::CaptureView> for CaptureViewFfi {
    fn from(view: postio_focus::CaptureView) -> Self {
        CaptureViewFfi {
            mode: match view.mode {
                postio_focus::CaptureMode::Task => CaptureModeFfi::Task,
                postio_focus::CaptureMode::Note => CaptureModeFfi::Note,
            },
            from: view.from,
            field: view.field,
            text: view.text,
            subject_key: view.subject_key,
            has_due: view.has_due,
            due: view.due.map(day),
            due_label: view.due_label,
            picks: view
                .picks
                .into_iter()
                .map(|pick| CapturePickFfi {
                    words: pick.words,
                    day: pick.day.map(day),
                })
                .collect(),
            project_title: view.project_title,
            project: view.project,
            project_note: view.project_note,
            project_key: view.project_key,
            projects_open: view.projects_open,
            filter: view.filter,
            projects: view
                .projects
                .into_iter()
                .map(|project| CaptureProjectFfi {
                    token: project.token,
                    name: project.name,
                    note: project.note,
                    open: project.open,
                    chosen: project.chosen,
                })
                .collect(),
            preview_title: view.preview_title,
            preview: view.preview,
            footnote: view.footnote,
            button: view.button,
            button_key: view.button_key,
            error: view.error,
        }
    }
}

/// A digest's summary, as the host reads it back: every statement whose
/// passage is still in its message's own text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestSummaryFfi {
    /// The statements, in reading order.
    pub statements: Vec<SummaryStatementFfi>,
    /// How many messages it was written from.
    pub messages: u32,
    /// From how many senders.
    pub senders: u32,
}

/// One statement of a summary.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SummaryStatementFfi {
    /// Its topic.
    pub topic: String,
    /// What it says.
    pub text: String,
    /// Its reference's number.
    pub number: u32,
    /// The message it cites.
    pub message: i64,
    /// The passage it cites, verbatim.
    pub excerpt: String,
}

/// A project note in the vault.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultProjectFfi {
    /// Its name.
    pub name: String,
    /// Its note, relative to the vault.
    pub note: String,
}

/// What capture needs of the vault for a message.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultPictureFfi {
    /// The projects.
    pub projects: Vec<VaultProjectFfi>,
    /// The one the subject suggests.
    pub suggested: Option<VaultProjectFfi>,
    /// The note a task goes to with no project.
    pub tasks_note: String,
}

/// What a capture appended.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CapturedFfi {
    /// The note, relative to the vault.
    pub note: String,
    /// The line appended.
    pub line: String,
}

fn project_ffi(project: postio_vault::Project) -> VaultProjectFfi {
    VaultProjectFfi {
        name: project.name,
        note: project.note.display().to_string(),
    }
}

fn unavailable(message: impl ToString) -> SessionError {
    SessionError::StoreUnavailable {
        message: message.to_string(),
    }
}

#[uniffi::export]
impl Session {
    /// A click on Filtered's row at `index`: the keyboard goes to it.
    pub fn focus_filtered_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::FilteredPoint(index));
    }

    /// Filtered was scrolled to its end: its next page, when `more`.
    pub fn focus_filtered_more(&self) {
        self.focus_driver().input(postio_focus::Input::FilteredMore);
    }

    /// A click on the digest's list row at `index`.
    pub fn focus_digest_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::DigestPoint(index));
    }

    /// A click on the summary's statement at `index` (its `index`).
    pub fn focus_digest_reference(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::DigestReference(index));
    }

    /// Yes, to the `FocusConfirm` with `token`. A no is nothing.
    pub fn focus_confirmed(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::Confirmed(token));
    }

    /// The rule sheet's query field holds `text` now.
    pub fn focus_rule_query(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::RuleQuery { text });
    }

    /// "Match a list or a search instead…".
    pub fn focus_rule_match_instead(&self) {
        self.focus_driver()
            .input(postio_focus::Input::RuleMatchInstead);
    }

    /// "Digest mail like this".
    pub fn focus_rule_like_this(&self) {
        self.focus_driver().input(postio_focus::Input::RuleLikeThis);
    }

    /// The rule sheet's cadence, day or time changed.
    pub fn focus_rule_schedule(&self, schedule: RuleScheduleFfi) {
        self.focus_driver()
            .input(postio_focus::Input::RuleSchedule(schedule.into()));
    }

    /// Create (or Save): write the rule. The sheet closes on
    /// `FocusCloseSurface { kind: Dialog }` once it is written, or redraws
    /// with its `error`.
    pub fn focus_rule_create(&self) {
        self.focus_driver().input(postio_focus::Input::RuleCreate);
    }

    /// The capture field holds `text` now.
    pub fn focus_capture_typed(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::CaptureTyped { text });
    }

    /// A quick pick: the due day, "YYYY-MM-DD", or `None` for none. A day
    /// that does not read is none.
    pub fn focus_capture_due(&self, day: Option<String>) {
        let due = day.and_then(|day| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok());
        self.focus_driver()
            .input(postio_focus::Input::CaptureDue(due));
    }

    /// The project list's filter holds `text` now.
    pub fn focus_capture_filter(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::CaptureFilter { text });
    }

    /// The project row with `token` was chosen.
    pub fn focus_capture_project(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::CaptureProject(token));
    }

    /// Open the message a `postio://` link names, as `FocusOpenMessage`;
    /// or a `FocusToast` saying `link_unknown()` or `link_gone()`.
    pub fn focus_open_link(&self, uri: String) {
        self.focus_driver()
            .input(postio_focus::Input::OpenLink(uri));
    }

    /// `delivery`'s summary as the host reads it back, or `None` when none
    /// is written or none is left to show. Blocks: call it off the main
    /// actor. The digest window does not need it -- `FocusDigest` carries
    /// the summary -- and reads it only when a model is configured (C6).
    pub fn digest_summary(&self, delivery: i64) -> Result<Option<DigestSummaryFfi>, SessionError> {
        let client = self
            .client()
            .ok_or_else(|| unavailable("The store is closed."))?;
        let summary = blocking(client.digest_summary(postio_model::DeliveryId::new(delivery)))
            .map_err(unavailable)?;
        Ok(summary.map(|summary| DigestSummaryFfi {
            statements: summary
                .statements
                .into_iter()
                .map(|statement| SummaryStatementFfi {
                    topic: statement.topic,
                    text: statement.text,
                    number: statement.reference.number,
                    message: statement.reference.message.get(),
                    excerpt: statement.reference.excerpt,
                })
                .collect(),
            messages: summary.messages,
            senders: summary.senders,
        }))
    }

    /// What capture needs of the vault for a message with `subject`. Fails
    /// with the host's sentence naming `[focus.vault]` when none is
    /// configured (C9). Blocks.
    pub fn vault(&self, subject: String) -> Result<VaultPictureFfi, SessionError> {
        let client = self
            .client()
            .ok_or_else(|| unavailable("The store is closed."))?;
        let picture = blocking(client.vault(&subject)).map_err(unavailable)?;
        Ok(VaultPictureFfi {
            projects: picture.projects.into_iter().map(project_ffi).collect(),
            suggested: picture
                .suggestion
                .map(|suggestion| project_ffi(suggestion.project)),
            tasks_note: picture.tasks_note.display().to_string(),
        })
    }

    /// Append a task to `project`'s note in the vault, or to the tasks note
    /// with none: the exact line, the message's `postio://` link before the
    /// due day (C21). `due` is "YYYY-MM-DD". Blocks.
    pub fn capture_task(
        &self,
        project: Option<VaultProjectFfi>,
        text: String,
        message: i64,
        due: Option<String>,
    ) -> Result<CapturedFfi, SessionError> {
        let client = self
            .client()
            .ok_or_else(|| unavailable("The store is closed."))?;
        let task = postio_vault::Task {
            text,
            message: postio_model::MessageId::new(message),
            due: due.and_then(|day| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok()),
        };
        let project = project.map(|project| postio_vault::Project {
            name: project.name,
            note: project.note.into(),
        });
        let captured = blocking(client.capture_task(project, task)).map_err(unavailable)?;
        Ok(CapturedFfi {
            note: captured.note.display().to_string(),
            line: captured.line,
        })
    }
}
