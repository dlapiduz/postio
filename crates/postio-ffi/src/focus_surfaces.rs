//! Filtered, the digest window, the rule sheet and capture. The controller (`postio_focus`) decides
//! what each holds and does; the Mac draws each as a whole view and reports clicks, typed text and
//! chosen days. Keys are commands `key` resolves.

use crate::session::{Session, SessionError, blocking};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusHintFfi {
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredViewFfi {
    pub title: String,
    /// "Archived automatically · newest first".
    pub subtitle: String,
    /// Right of the tabs: "Nothing here is deleted automatically · …" (C4).
    pub note: String,
    /// "Sweep the inbox…": a click is `invoke("sweep_inbox")`.
    pub sweep: String,
    pub sweep_key: Option<String>,
    /// The focused row's button: a click is `invoke("restore_filtered")`.
    pub restore: String,
    pub restore_key: Option<String>,
    /// The tabs, `1` to `7`: a click is `invoke("filtered_tab_<n>")`.
    pub tabs: Vec<FilteredTabFfi>,
    pub rows: Vec<FilteredLineFfi>,
    pub focused: Option<u32>,
    /// Whether there may be more: `focus_filtered_more()` at the end.
    pub more: bool,
    pub footer: Vec<FocusHintFfi>,
    pub bar: Option<FilteredBarFfi>,
    pub empty: Option<String>,
    /// The thread open under one of the rows, when one is; `focused` then
    /// counts its children.
    pub expanded: Option<FilteredSpliceFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredSpliceFfi {
    pub anchor: u32,
    /// One per message, oldest first, then the unsent reply.
    pub children: Vec<crate::ChildRowFfi>,
}

/// The bar over Filtered while anything is selected. A verb's press is `invoke(command)`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredBarFfi {
    pub summary: String,
    /// The verbs, the first the one a selection is usually for.
    pub actions: Vec<FilteredBarActionFfi>,
    /// "x toggle", "J/K extend", "Esc clear".
    pub hints: Vec<FocusHintFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredBarActionFfi {
    /// The registry id a press invokes: "restore_filtered".
    pub command: String,
    pub label: String,
    pub key: Option<String>,
    pub primary: bool,
}

impl From<postio_focus::FilteredBar> for FilteredBarFfi {
    fn from(bar: postio_focus::FilteredBar) -> Self {
        FilteredBarFfi {
            summary: bar.summary,
            actions: bar
                .actions
                .into_iter()
                .map(|action| FilteredBarActionFfi {
                    command: action.command.as_str().to_owned(),
                    label: action.label,
                    key: action.key,
                    primary: action.primary,
                })
                .collect(),
            hints: bar.hints.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredTabFfi {
    pub name: String,
    pub count: u32,
    pub key: Option<String>,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteredLineFfi {
    pub message: i64,
    pub sender: String,
    pub subject: String,
    pub preview: Option<String>,
    /// The reason pill: "notification · Forge".
    pub pill: String,
    pub time: String,
    /// The day heading above it, on the first row of each day.
    pub heading: Option<String>,
    pub selected: bool,
    /// How many messages its conversation holds; more than one opens in
    /// place (`invoke("expand_thread")`).
    pub messages: u32,
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
                    selected: row.selected,
                    messages: row.messages,
                })
                .collect(),
            focused: view.focused,
            more: view.more,
            footer: view.footer.into_iter().map(Into::into).collect(),
            bar: view.bar.map(Into::into),
            empty: view.empty,
            expanded: view.expanded.map(|splice| FilteredSpliceFfi {
                anchor: splice.anchor,
                children: splice
                    .children
                    .iter()
                    .map(crate::ChildRowFfi::from)
                    .collect(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DigestPageFfi {
    Summary,
    List,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestViewFfi {
    pub delivery: i64,
    pub page: DigestPageFfi,
    /// "Weekly · Newsletters"; the email's subject on the email page.
    pub title: String,
    /// "14 messages from 6 senders · came due today 16:00"; "Source 2 of
    /// 14" on the email page.
    pub subtitle: String,
    /// "Archive all 14": a click is `invoke("archive")`.
    pub archive: String,
    pub archive_key: Option<String>,
    /// "Weekly, Sunday 09:00 · Edit rule and cadence": a click is
    /// `invoke("digest_rule")`.
    pub rule_line: String,
    pub rule_key: Option<String>,
    pub tabs: bool,
    pub list_tab: String,
    /// The key that switches: a click on a tab is `invoke("toggle_digest_summary")`.
    pub tab_key: Option<String>,
    pub rows: Vec<DigestLineFfi>,
    /// Its row with the keyboard: what `D` and `U` act on.
    pub focused: Option<u32>,
    pub topics: Vec<DigestTopicFfi>,
    /// The focused reference, in reading order (`DigestStatementFfi.index`).
    pub focused_reference: Option<u32>,
    pub card: Option<DigestCardFfi>,
    pub footer: Option<String>,
    /// The email on the email page: draw `reader_document(email.message)`
    /// in the window, the banner over it, `excerpt` highlighted.
    pub email: Option<DigestEmailFfi>,
    /// The email page's "‹ Summary" (or "‹ 3 messages"): a click is
    /// `invoke("back")`.
    pub back: String,
    pub back_key: Option<String>,
    /// Whether what it holds is still being read.
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestLineFfi {
    pub message: i64,
    pub sender: String,
    pub subject: String,
    pub preview: Option<String>,
    pub time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestTopicFfi {
    /// "Rates · 3 statements from 2 messages".
    pub heading: String,
    pub statements: Vec<DigestStatementFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestStatementFfi {
    /// Its place among every statement: what `focus_digest_reference`
    /// takes, and `focused_reference` names.
    pub index: u32,
    /// Plain text, never markup (FR-174).
    pub text: String,
    pub number: u32,
    pub message: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestCardFfi {
    /// "Reference 3 · Ada Moreno · The rate decision".
    pub title: String,
    /// "open the full email".
    pub hint: String,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestEmailFfi {
    pub message: i64,
    /// The reference that cites it, when one does.
    pub number: Option<u32>,
    pub excerpt: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConfirmFfi {
    /// What `focus_confirmed` names it by.
    pub token: u64,
    pub heading: String,
    pub body: String,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RuleViewFfi {
    /// "Digest this sender", "Digest rule · Newsletters".
    pub heading: String,
    pub from_label: String,
    /// The senders, as a plain line; `None` once the query field is up.
    pub from: Option<String>,
    /// The query field's text while it is up: set the field from it only
    /// when it differs.
    pub query: Option<String>,
    pub placeholder: String,
    /// "Match a list or a search instead…": a click is `focus_rule_match_instead()`.
    pub match_instead: Option<String>,
    /// "Digest mail like this": a click is `focus_rule_like_this()`.
    pub like_this: Option<String>,
    pub deliver_label: String,
    pub schedule: RuleScheduleFfi,
    pub cadences: Vec<String>,
    pub weekdays: Vec<String>,
    pub note: String,
    /// "Create" or "Save": a click is `focus_rule_create()`.
    pub create: String,
    pub create_key: Option<String>,
    /// "Would have caught 9 messages in the last 90 days", once read.
    pub preview_heading: Option<String>,
    pub preview: Vec<RulePreviewLineFfi>,
    pub more: Option<String>,
    /// Why the rule cannot be written as it stands.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulePreviewLineFfi {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CaptureModeFfi {
    Task,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CapturePickFfi {
    /// "Today", "Mon", "None".
    pub words: String,
    /// The day, "YYYY-MM-DD"; `None` for no due day.
    pub day: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CaptureProjectFfi {
    /// What `focus_capture_project` takes.
    pub token: u64,
    /// Its name, or "Inbox".
    pub name: String,
    pub note: String,
    /// "3 open".
    pub open: String,
    pub chosen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CaptureViewFfi {
    /// Task or note: the segmented control, whose clicks are
    /// `invoke("capture_task")` and `invoke("capture_note")`.
    pub mode: CaptureModeFfi,
    /// "From Ada · Quarterly review · Mon".
    pub from: String,
    pub field: String,
    /// The text, verbatim: set the field from it only when it differs.
    pub text: String,
    pub subject_key: Option<String>,
    pub has_due: bool,
    pub due: Option<String>,
    /// "Friday, 2 October 2026", "No due date".
    pub due_label: String,
    /// The quick picks: a click is `focus_capture_due(pick.day)`.
    pub picks: Vec<CapturePickFfi>,
    /// "Project · suggested: the subject names Review".
    pub project_title: String,
    pub project: String,
    pub project_note: String,
    /// `⌘P`'s keycap: `invoke("capture_change_project")` opens the list.
    pub project_key: Option<String>,
    pub projects_open: bool,
    pub filter: String,
    /// The projects the filter lets through, then the Inbox.
    pub projects: Vec<CaptureProjectFfi>,
    /// "This exact line will be appended".
    pub preview_title: String,
    pub preview: String,
    pub footnote: String,
    /// "Add task", "Add note": a click is `invoke("capture_write")`.
    pub button: String,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DigestSummaryFfi {
    pub statements: Vec<SummaryStatementFfi>,
    pub messages: u32,
    pub senders: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SummaryStatementFfi {
    pub topic: String,
    pub text: String,
    pub number: u32,
    pub message: i64,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultProjectFfi {
    pub name: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultPictureFfi {
    pub projects: Vec<VaultProjectFfi>,
    pub suggested: Option<VaultProjectFfi>,
    pub tasks_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CapturedFfi {
    pub note: String,
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
    pub fn focus_filtered_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::FilteredPoint(index));
    }

    /// A modified click on Filtered's row at `position`: `range` when Shift was held, a toggle for
    /// the platform's toggle modifier.
    pub fn focus_filtered_pick(&self, position: u32, range: bool) {
        self.focus_driver()
            .input(postio_focus::Input::FilteredPick { position, range });
    }

    pub fn focus_filtered_more(&self) {
        self.focus_driver().input(postio_focus::Input::FilteredMore);
    }

    pub fn focus_digest_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::DigestPoint(index));
    }

    pub fn focus_digest_reference(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::DigestReference(index));
    }

    /// Yes, to the `FocusConfirm` with `token`. A no is nothing.
    pub fn focus_confirmed(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::Confirmed(token));
    }

    pub fn focus_rule_query(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::RuleQuery { text });
    }

    pub fn focus_rule_match_instead(&self) {
        self.focus_driver()
            .input(postio_focus::Input::RuleMatchInstead);
    }

    pub fn focus_rule_like_this(&self) {
        self.focus_driver().input(postio_focus::Input::RuleLikeThis);
    }

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

    pub fn focus_capture_filter(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::CaptureFilter { text });
    }

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

    /// `delivery`'s summary as the host reads it back, or `None` when none is written or left to
    /// show. Blocks: call it off the main actor.
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

    /// What capture needs of the vault for a message with `subject`. Fails with the host's sentence
    /// naming `[focus.vault]` when none is configured. Blocks.
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

    /// Append a task to `project`'s note, or to the tasks note with none: the exact line, the
    /// message's `postio://` link before the due day. `due` is "YYYY-MM-DD". Blocks.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FocusDigestGeometryFfi {
    pub window_width: i32,
    pub column_width: i32,
}

/// The digest's window beside a main window `main_width` wide: the message window's width, so a
/// reference opened inside it never resizes it, and the summary's column, `min(960, w − 80)`.
#[uniffi::export]
pub fn focus_digest_geometry(main_width: i32) -> FocusDigestGeometryFfi {
    let geometry =
        postio_ui::focus_dialog::Geometry::for_platform(postio_config::paths::Platform::Apple);
    let (window_width, _) = geometry.digest_size(main_width, 0);
    FocusDigestGeometryFfi {
        window_width,
        column_width: geometry
            .column_width(window_width, postio_body::treatment::Treatment::AppColours),
    }
}
