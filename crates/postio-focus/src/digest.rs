//! The digest window and the digest rule dialog (research R2, slice 12;
//! spec 007 US10, US13, US14; screens 22 to 24).
//!
//! Moved from `postio-gtk`'s `digest.rs` (its page, its focused row and
//! reference, the email opened from a reference), `rule_dialog.rs` (what a
//! new or edited rule holds, its preview, Create), and the window's digest
//! handlers (`open_digest`, `digest_key`, `ask_stop_digesting`,
//! `unsubscribe`, `edit_digest_rule`, `new_digest_rule`). The words are
//! `postio_ui::digest`'s and `postio_ui::focus_target`'s; the controller
//! keeps what each surface holds, what each key does to it, and which read
//! is current.
//!
//! The digest opens on its summary when the person has brought a model and
//! one is written, and on its plain list otherwise (C6). A reference's email
//! opens in the digest's own window, and Esc comes back to the same
//! reference. On the Mac a message from the plain list opens there too
//! (M4); Linux's reading dialog stacks over the digest instead.

use chrono::{DateTime, Utc};
use postio_client::protocol::{DigestPreview, DigestRuleDraft, RuleDay};
use postio_config::Due;
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::listing::{Cadence, MessageSummary};
use postio_model::summary::DigestSummary;
use postio_model::{DeliveryId, MessageId};
use postio_ui::digest::{self as words, Page, Schedule};
use postio_ui::focus_list::FocusRow;
use postio_ui::hints;

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::surfaces::{Host, Origin};
use crate::{FocusController, Intent, Reply, Request, SurfaceKind, ToastKind};

/// The digest's window, drawable as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestView {
    /// The delivery on screen.
    pub delivery: DeliveryId,
    /// The page showing.
    pub page: Page,
    /// "Weekly · Newsletters"; the email's subject on the email page.
    pub title: String,
    /// "14 messages from 6 senders · came due today 16:00"; "Source 2 of
    /// 14" on the email page.
    pub subtitle: String,
    /// "Archive all 14".
    pub archive: String,
    /// Its key.
    pub archive_key: Option<String>,
    /// "Weekly, Sunday 09:00 · Edit rule and cadence".
    pub rule_line: String,
    /// Its key.
    pub rule_key: Option<String>,
    /// Whether the Summary and list tabs show: only once a summary exists.
    pub tabs: bool,
    /// The list tab's words: "14 messages".
    pub list_tab: String,
    /// The key that switches between them.
    pub tab_key: Option<String>,
    /// The plain list, one line a message.
    pub rows: Vec<DigestLine>,
    /// The list's row with the keyboard: what `D` and `U` act on.
    pub focused: Option<u32>,
    /// The summary, by topic.
    pub topics: Vec<DigestTopic>,
    /// The focused reference, in reading order.
    pub focused_reference: Option<u32>,
    /// The card under the paragraphs for the focused reference's message.
    pub card: Option<DigestCard>,
    /// The line under the summary.
    pub footer: Option<String>,
    /// The email shown in place, on the email page.
    pub email: Option<DigestEmail>,
    /// The email page's way back: "Summary", "3 messages".
    pub back: String,
    /// Its key.
    pub back_key: Option<String>,
    /// Whether what the delivery holds is still being read.
    pub loading: bool,
}

/// One message of the digest's plain list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestLine {
    /// The message.
    pub message: MessageId,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// Its first line.
    pub preview: Option<String>,
    /// When it came: "16:02", "Tue".
    pub time: String,
}

/// A run of the summary's statements under one topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestTopic {
    /// "Rates · 3 statements from 2 messages".
    pub heading: String,
    /// Its statements, in reading order.
    pub statements: Vec<DigestStatement>,
}

/// One statement of the summary, ending in its numbered reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestStatement {
    /// Where it is among all the statements: what
    /// [`crate::Input::DigestReference`] names.
    pub index: u32,
    /// What it says: plain text, never markup (FR-174).
    pub text: String,
    /// Its reference's number.
    pub number: u32,
    /// The message it cites.
    pub message: MessageId,
}

/// The card under the summary for the focused reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestCard {
    /// "Reference 3 · Ada Moreno · The rate decision".
    pub title: String,
    /// "open the full email".
    pub hint: String,
    /// Its key.
    pub key: Option<String>,
}

/// The email shown in the digest's window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestEmail {
    /// The message: what `reader_document` draws.
    pub message: MessageId,
    /// The reference that cites it, when one does.
    pub number: Option<u32>,
    /// The passage to highlight, when a reference cites it.
    pub excerpt: Option<String>,
    /// The banner over it, when a reference cites it.
    pub banner: Option<String>,
}

/// What the controller keeps of the digest's window.
#[derive(Debug)]
pub(crate) struct DigestWindow {
    delivery: DeliveryId,
    rule: String,
    cadence: Option<Cadence>,
    count: u32,
    senders: usize,
    at: DateTime<Utc>,
    rule_when: Option<String>,
    rows: Vec<MessageSummary>,
    summary: Option<DigestSummary>,
    page: Page,
    focused: Option<usize>,
    reference: Option<usize>,
    /// The email on screen, and the page Esc goes back to.
    email: Option<(MessageId, Page)>,
    stamp: u64,
    loading: bool,
}

impl DigestWindow {
    /// The message at `index` of the plain list, as a verb sees it.
    pub(crate) fn facts(&self, index: usize) -> Option<RowFacts> {
        self.rows.get(index).map(|row| RowFacts {
            id: row.id,
            digest: false,
            threads: row.thread.into_iter().collect(),
            writes: false,
        })
    }

    /// Where `message` is in the plain list.
    pub(crate) fn index_of(&self, message: MessageId) -> Option<usize> {
        self.rows.iter().position(|row| row.id == message)
    }

    /// How many messages the plain list holds.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// The subject of the plain list's message at `index`.
    pub(crate) fn subject(&self, index: usize) -> Option<String> {
        self.rows.get(index)?.subject.clone()
    }

    fn statements(&self) -> usize {
        self.summary
            .as_ref()
            .map_or(0, |summary| summary.statements.len())
    }

    fn has_summary(&self) -> bool {
        self.statements() > 0
    }

    /// The reference that cites `message`: its index, number and passage.
    fn cited(&self, message: MessageId) -> Option<(usize, u32, String)> {
        self.summary
            .as_ref()?
            .statements
            .iter()
            .enumerate()
            .find(|(_, statement)| statement.reference.message == message)
            .map(|(index, statement)| {
                (
                    index,
                    statement.reference.number,
                    statement.reference.excerpt.clone(),
                )
            })
    }

    /// The list's focus follows the focused reference: `D` and `U` act on
    /// the message it cites.
    fn follow_reference(&mut self) {
        let message = self.reference.and_then(|index| {
            self.summary
                .as_ref()?
                .statements
                .get(index)
                .map(|statement| statement.reference.message)
        });
        if let Some(at) = message.and_then(|message| self.index_of(message)) {
            self.focused = Some(at);
        }
    }

    fn focused_message(&self) -> Option<&MessageSummary> {
        self.focused.and_then(|at| self.rows.get(at))
    }
}

/// What the controller keeps of the rule dialog.
#[derive(Debug)]
pub(crate) struct RuleDialog {
    heading: String,
    name: String,
    queries: Vec<String>,
    replacing: Option<String>,
    like_this: Option<MessageId>,
    /// The senders' line; `None` once the query entry is up.
    from: Option<String>,
    /// The query entry's text, once it is up.
    query: Option<String>,
    note: String,
    schedule: Schedule,
    preview: Option<DigestPreview>,
    error: Option<String>,
    stamp: u64,
}

/// The rule dialog, drawable as it is (screen 24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleView {
    /// "Digest this sender", "Digest rule · Newsletters".
    pub heading: String,
    /// The first row's label: "From".
    pub from_label: String,
    /// The senders, as a plain line; `None` once the query entry is up.
    pub from: Option<String>,
    /// The query entry's text, while it is up.
    pub query: Option<String>,
    /// The query entry's placeholder.
    pub placeholder: String,
    /// "Match a list or a search instead…", while the senders show.
    pub match_instead: Option<String>,
    /// "Digest mail like this", when a model can be asked for one.
    pub like_this: Option<String>,
    /// The schedule row's label: "Deliver".
    pub deliver_label: String,
    /// The schedule, as its controls hold it.
    pub schedule: Schedule,
    /// The cadences, in menu order: "Daily", "Weekly", "Monthly".
    pub cadences: Vec<String>,
    /// The weekdays, in menu order: "Monday" to "Sunday".
    pub weekdays: Vec<String>,
    /// What still comes straight to the inbox.
    pub note: String,
    /// "Create", or "Save" while editing.
    pub create: String,
    /// Its key.
    pub create_key: Option<String>,
    /// "Would have caught 9 messages in the last 90 days", once read.
    pub preview_heading: Option<String>,
    /// The newest of them.
    pub preview: Vec<RulePreviewLine>,
    /// "and 5 more".
    pub more: Option<String>,
    /// Why the rule cannot be written as it stands.
    pub error: Option<String>,
}

/// One message the rule would have caught.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePreviewLine {
    /// Its subject.
    pub subject: String,
    /// When it came: "Today", "Tue", "12 Sep".
    pub day: String,
}

impl FocusController {
    /// Open the digest whose row is at `position`, as `row` names it.
    pub(crate) fn open_digest(
        &mut self,
        row: MessageId,
        position: u32,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let digest = match rows.row(position) {
            Some(FocusRow::Digest(digest)) => Some(digest),
            _ => self.feed.digest(row),
        };
        let Some(digest) = digest else {
            return vec![Step::Show(Intent::OpenDigest { row })];
        };
        let stamp = self.stamp();
        let config = &self.states.config;
        let rule_when = config
            .digests
            .iter()
            .find(|rule| rule.name.trim() == digest.rule.trim())
            .and_then(words::rule_when);
        // C6: a summary is read only when the person has brought a model.
        let summary = config
            .model_for(postio_config::model::ModelFeature::DigestSummary)
            .is_some();
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Digest, self.policy.caps.stacking);
        self.digest = Some(DigestWindow {
            delivery: digest.delivery,
            rule: digest.rule.clone(),
            cadence: digest.cadence,
            count: digest.count,
            senders: digest.senders.len(),
            at: digest.at,
            rule_when,
            rows: Vec::new(),
            summary: None,
            page: Page::List,
            focused: None,
            reference: None,
            email: None,
            stamp,
            loading: true,
        });
        steps.push(Step::Show(Intent::OpenDigest { row }));
        steps.extend(self.draw_digest());
        steps.push(Step::Ask(Request::DigestRead {
            delivery: digest.delivery,
            summary,
            stamp,
        }));
        steps
    }

    /// Whether `id` is the digest's own while it is on top.
    pub(crate) fn digest_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::NextReference
                | CommandId::PrevReference
                | CommandId::ToggleDigestSummary
                | CommandId::StopDigestingSender
                | CommandId::Unsubscribe
                | CommandId::DigestRule
                | CommandId::ArchiveThread
        )
    }

    /// The email open in the digest's window, on its email page.
    pub(crate) fn digest_email(&self) -> Option<MessageId> {
        self.digest.as_ref()?.email.map(|(message, _)| message)
    }

    /// A command while the digest is on top, or `None` when the app's rules
    /// apply to it (Undo, the key map, the bar).
    pub(crate) fn digest_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let window = self.digest.as_mut()?;
        let steps = match id {
            CommandId::Back => {
                if let Some((_, from)) = window.email.take() {
                    window.page = from;
                    self.draw_digest()
                } else {
                    self.close_digest()
                }
            }
            CommandId::GoToInbox => self.close_digest(),
            CommandId::ArchiveThread => {
                let delivery = window.delivery;
                let mut steps = vec![Step::Ask(Request::Post(Command::ArchiveDigest {
                    delivery,
                    archived: true,
                }))];
                steps.extend(self.close_digest());
                steps
            }
            CommandId::DigestRule => {
                let rule = window.rule.clone();
                self.edit_rule(&rule)
            }
            CommandId::NextReference | CommandId::PrevReference => {
                let by = if id == CommandId::NextReference {
                    1
                } else {
                    -1
                };
                self.step_reference(by)
            }
            CommandId::ToggleDigestSummary => match window.page.toggled(window.has_summary()) {
                Some(page) => {
                    window.page = page;
                    self.draw_digest()
                }
                None => Vec::new(),
            },
            CommandId::StopDigestingSender => {
                let Some(message) = window.focused_message() else {
                    return Some(Vec::new());
                };
                let id = message.id;
                let sender = message
                    .from
                    .as_ref()
                    .map(|from| from.address.clone())
                    .unwrap_or_default();
                vec![self.ask_first(
                    postio_ui::focus_target::stop_digesting_title(&sender),
                    postio_ui::focus_target::STOP_DIGESTING_BODY.to_owned(),
                    postio_ui::focus_target::STOP_DIGESTING.to_owned(),
                    false,
                    Command::StopDigestingSender {
                        target: MessageTarget::Messages(vec![id]),
                        stopped: true,
                        kept: None,
                    },
                )]
            }
            CommandId::Unsubscribe => match window.focused_message() {
                Some(message) => vec![Step::Ask(Request::Unsubscribe(message.id))],
                None => Vec::new(),
            },
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                match window.page {
                    Page::Summary => self.step_reference(by),
                    Page::List => self.step_digest_row(by),
                    Page::Email => {
                        let mut steps = self.step_digest_row(by);
                        if !steps.is_empty() {
                            steps = self.digest_email_at_focus();
                        }
                        steps
                    }
                }
            }
            CommandId::OpenMessage => match window.page {
                Page::Summary => self.open_reference(),
                Page::List => self.open_digest_row(rows),
                Page::Email => Vec::new(),
            },
            CommandId::Undo
            | CommandId::CheatSheet
            | CommandId::Search
            | CommandId::CommandPalette => return None,
            // A verb meant for the list behind: nobody can see its rows.
            _ => Vec::new(),
        };
        Some(steps)
    }

    /// The digest goes, however it was asked to.
    fn close_digest(&mut self) -> Vec<Step> {
        self.digest = None;
        self.surfaces.dismiss(SurfaceKind::Digest);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Digest))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// `]`/`[`: the focused reference, clamped to the summary's ends.
    fn step_reference(&mut self, by: i32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(next) = words::step_reference(window.reference, by, window.statements()) else {
            return Vec::new();
        };
        window.reference = Some(next);
        window.follow_reference();
        self.draw_digest()
    }

    /// `j`/`k` over the plain list, or the email opened from it.
    fn step_digest_row(&mut self, by: i32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if window.rows.is_empty() {
            return Vec::new();
        }
        let last = window.rows.len() as i64 - 1;
        let next = window
            .focused
            .map_or(0, |at| (at as i64 + i64::from(by)).clamp(0, last)) as usize;
        if window.focused == Some(next) {
            return Vec::new();
        }
        window.focused = Some(next);
        self.draw_digest()
    }

    /// Return over the summary: the focused reference's email, in place.
    fn open_reference(&mut self) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(message) = window.reference.and_then(|index| {
            window
                .summary
                .as_ref()?
                .statements
                .get(index)
                .map(|statement| statement.reference.message)
        }) else {
            return Vec::new();
        };
        window.email = Some((message, Page::Summary));
        window.page = Page::Email;
        self.show_digest_email(message)
    }

    /// Return over the plain list: on the Mac, the message in the digest's
    /// window (M4); on Linux, the reading dialog over the digest.
    fn open_digest_row(&mut self, _rows: &dyn Rows) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(at) = window.focused else {
            return Vec::new();
        };
        if self.policy.caps.stacking {
            return self.open_digest_message(at);
        }
        let Some(message) = window.rows.get(at).map(|row| row.id) else {
            return Vec::new();
        };
        window.email = Some((message, Page::List));
        window.page = Page::Email;
        self.show_digest_email(message)
    }

    /// Over the email page: show the message the list's focus is on now.
    fn digest_email_at_focus(&mut self) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(message) = window.focused_message().map(|row| row.id) else {
            return Vec::new();
        };
        let from = window.email.map_or(Page::List, |(_, from)| from);
        window.email = Some((message, from));
        self.show_digest_email(message)
    }

    /// The email page on `message`: shown in the digest's window, citing its
    /// reference when one names it.
    fn show_digest_email(&mut self, message: MessageId) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if let Some(at) = window.index_of(message) {
            window.focused = Some(at);
        }
        let index = window.index_of(message).unwrap_or(0) as u32;
        let total = window.rows.len() as u32;
        let mut steps = vec![Step::Show(Intent::OpenMessage {
            message,
            index,
            total,
            host: Host::Digest,
        })];
        steps.extend(self.draw_digest());
        steps
    }

    /// Linux: the digest's message at `index` in the reading dialog, over
    /// the digest.
    pub(crate) fn open_digest_message(&mut self, index: usize) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(row) = window.facts(index) else {
            return Vec::new();
        };
        window.focused = Some(index);
        let total = window.rows.len() as u32;
        let mut steps = self
            .surfaces
            .open(&row, index as u32, total, self.policy.caps.stacking);
        self.surfaces.set_origin(Origin::Digest);
        steps.extend(self.draw_digest());
        steps
    }

    /// A click on the plain list's row at `index`.
    pub(crate) fn digest_point(&mut self, index: u32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if index as usize >= window.rows.len() {
            return Vec::new();
        }
        window.focused = Some(index as usize);
        self.draw_digest()
    }

    /// A click on the summary's reference at `index`.
    pub(crate) fn digest_reference(&mut self, index: u32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if index as usize >= window.statements() {
            return Vec::new();
        }
        window.reference = Some(index as usize);
        window.follow_reference();
        self.draw_digest()
    }

    /// An answer for the digest's window.
    pub(crate) fn digest_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::DigestRead {
                stamp,
                rows,
                summary,
            } => {
                let Some(window) = self.digest.as_mut().filter(|it| it.stamp == stamp) else {
                    return Vec::new();
                };
                window.loading = false;
                match rows {
                    Ok(rows) => window.rows = rows,
                    Err(error) => tracing::warn!(%error, "Focus could not read a digest"),
                }
                window.summary = summary.filter(|summary| !summary.is_empty());
                let has_summary = window.has_summary();
                window.page = Page::opening(has_summary);
                window.reference = has_summary.then_some(0);
                window.focused = (!window.rows.is_empty()).then_some(0);
                window.follow_reference();
                self.draw_digest()
            }
            Reply::Unsubscribed(said) => vec![Step::Show(Intent::Toast {
                text: match said {
                    Ok(list) => postio_ui::focus_target::unsubscribed(&list),
                    Err(error) => error,
                },
                kind: ToastKind::Notice,
            })],
            _ => Vec::new(),
        }
    }

    /// Redraw the digest's window.
    fn draw_digest(&self) -> Vec<Step> {
        self.digest_view()
            .map(|view| vec![Step::Show(Intent::Digest(Box::new(view)))])
            .unwrap_or_default()
    }

    /// The digest's window in words, as it stands.
    pub(crate) fn digest_view(&self) -> Option<DigestView> {
        let window = self.digest.as_ref()?;
        let keymap = self.keymap();
        let now = self.now();
        let mut title = words::window_title(window.cadence, &window.rule);
        let mut subtitle = words::window_subtitle(
            window.count,
            window.senders,
            window.at.with_timezone(&chrono::Local),
            now,
        );
        let email = window.email.map(|(message, _)| {
            let cited = window.cited(message);
            if let Some(row) = window.rows.iter().find(|row| row.id == message) {
                title = row.subject.clone().unwrap_or_default();
            }
            subtitle = words::source_line(window.index_of(message), window.rows.len());
            DigestEmail {
                message,
                number: cited.as_ref().map(|(_, number, _)| *number),
                excerpt: cited.as_ref().map(|(_, _, excerpt)| excerpt.clone()),
                banner: cited
                    .as_ref()
                    .map(|(_, number, _)| words::cited_banner(*number)),
            }
        });
        let statements = window
            .summary
            .as_ref()
            .map_or(&[][..], |summary| &summary.statements[..]);
        let card = window.reference.and_then(|index| {
            let statement = statements.get(index)?;
            let row = window
                .rows
                .iter()
                .find(|row| row.id == statement.reference.message)?;
            let sender = row.from.as_ref().map(|from| from.display().to_owned());
            Some(DigestCard {
                title: words::reference_title(
                    statement.reference.number,
                    sender.as_deref(),
                    row.subject.as_deref(),
                ),
                hint: words::OPEN_FULL_EMAIL.to_owned(),
                key: hints::key(keymap, CommandId::OpenMessage),
            })
        });
        let list_tab = words::list_tab(window.rows.len());
        Some(DigestView {
            delivery: window.delivery,
            page: window.page,
            title,
            subtitle,
            archive: words::archive_all(window.count),
            archive_key: hints::key(keymap, CommandId::ArchiveThread),
            rule_line: words::rule_line(window.rule_when.as_deref()),
            rule_key: hints::key(keymap, CommandId::DigestRule),
            tabs: window.has_summary(),
            list_tab: list_tab.clone(),
            tab_key: hints::key(keymap, CommandId::ToggleDigestSummary),
            rows: window
                .rows
                .iter()
                .map(|row| DigestLine {
                    message: row.id,
                    sender: row
                        .from
                        .as_ref()
                        .map(|from| from.display().to_owned())
                        .unwrap_or_default(),
                    subject: row.subject.clone().unwrap_or_default(),
                    preview: row.preview.clone(),
                    time: postio_ui::row::timestamp(row.received_at, now),
                })
                .collect(),
            focused: window.focused.map(|at| at as u32),
            topics: words::topics(statements)
                .iter()
                .map(|topic| DigestTopic {
                    heading: words::topic_heading(topic),
                    statements: topic
                        .statements
                        .iter()
                        .enumerate()
                        .map(|(at, statement)| DigestStatement {
                            index: (topic.start + at) as u32,
                            text: statement.text.clone(),
                            number: statement.reference.number,
                            message: statement.reference.message,
                        })
                        .collect(),
                })
                .collect(),
            focused_reference: window.reference.map(|at| at as u32),
            card,
            footer: window
                .summary
                .as_ref()
                .map(|summary| words::summary_footer(summary.messages)),
            email,
            back: match window.email {
                Some((_, Page::List)) => list_tab,
                _ => "Summary".to_owned(),
            },
            back_key: hints::key(keymap, CommandId::Back),
            loading: window.loading,
        })
    }

    /// `d` on the list, or on the open message: a new rule for the senders
    /// of what a verb would aim at.
    pub(crate) fn new_rule(&mut self, rows: &dyn Rows) -> Vec<Step> {
        use postio_core::state::Selection;
        let selection = self.cursor.selection();
        let cursor = self.cursor.position().and_then(|at| rows.row(at));
        let resident: Vec<FocusRow> = match &selection {
            Selection::These(picked) => picked
                .iter()
                .filter_map(|id| rows.position_of(*id).and_then(|at| rows.row(at)))
                .collect(),
            Selection::Everything { .. } => Vec::new(),
        };
        let aimed = postio_ui::focus_target::aimed_rows(&selection, cursor.clone(), resident);
        let senders = postio_ui::focus_target::senders(&aimed);
        if senders.is_empty() {
            return Vec::new();
        }
        let like_this = postio_ui::focus_target::like_this_message(
            &selection,
            self.states
                .config
                .model_for(postio_config::model::ModelFeature::LikeThis)
                .is_some(),
            cursor.as_ref(),
        );
        let names: Vec<String> = senders
            .iter()
            .map(|sender| sender.display().to_owned())
            .collect();
        let stamp = self.stamp();
        self.rule = Some(RuleDialog {
            heading: words::new_rule_heading(senders.len()).to_owned(),
            name: words::rule_name(&names),
            queries: words::sender_queries(&senders),
            replacing: None,
            like_this,
            from: Some(words::sender_line(&senders)),
            query: None,
            note: words::rule_note(senders.len()).to_owned(),
            schedule: Schedule::new_rule(),
            preview: None,
            error: None,
            stamp,
        });
        self.open_rule()
    }

    /// `d` in a digest: edit the rule called `name`, or say it is gone.
    fn edit_rule(&mut self, name: &str) -> Vec<Step> {
        let rule = self
            .states
            .config
            .digests
            .iter()
            .find(|rule| rule.name.trim() == name.trim())
            .cloned();
        let Some(rule) = rule else {
            return vec![Step::Show(Intent::Toast {
                text: postio_ui::focus_target::RULE_MISSING.to_owned(),
                kind: ToastKind::Notice,
            })];
        };
        let senders = words::is_sender_rule(&rule.queries);
        let text = rule.queries.join(", ");
        let stamp = self.stamp();
        self.rule = Some(RuleDialog {
            heading: words::edit_rule_heading(&rule.name),
            name: rule.name.clone(),
            queries: rule.queries.clone(),
            replacing: Some(rule.name.clone()),
            // Never while editing (US14).
            like_this: None,
            from: senders.then(|| text.clone()),
            query: (!senders).then_some(text),
            note: words::rule_note(rule.queries.len()).to_owned(),
            schedule: rule
                .due()
                .map_or_else(|_| Schedule::new_rule(), |due| Schedule::of(&due)),
            preview: None,
            error: None,
            stamp,
        });
        self.open_rule()
    }

    /// Show the rule dialog on what it holds, and read its preview.
    fn open_rule(&mut self) -> Vec<Step> {
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Dialog, self.policy.caps.stacking);
        if let Some(view) = self.rule_view() {
            steps.push(Step::Show(Intent::OpenRule(Box::new(view))));
        }
        steps.extend(self.read_preview());
        steps
    }

    /// What the rule would have caught over the last 90 days.
    fn read_preview(&mut self) -> Vec<Step> {
        let since = self.now().to_utc() - chrono::Duration::days(words::PREVIEW_DAYS);
        let stamp = self.stamp();
        let Some(rule) = self.rule.as_mut() else {
            return Vec::new();
        };
        rule.stamp = stamp;
        if rule.queries.is_empty() {
            rule.preview = None;
            return Vec::new();
        }
        vec![Step::Ask(Request::DigestPreview {
            queries: rule.queries.clone(),
            since,
            stamp,
        })]
    }

    /// Something the person did in the rule dialog.
    pub(crate) fn rule_input(&mut self, input: crate::Input) -> Vec<Step> {
        use crate::Input;
        let Some(rule) = self.rule.as_mut() else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        match input {
            Input::RuleQuery { text } => {
                if rule.query.as_deref() == Some(text.as_str()) {
                    return Vec::new();
                }
                set_query(rule, &text);
                steps.extend(self.read_preview());
            }
            Input::RuleMatchInstead => {
                set_query(rule, "");
                steps.extend(self.read_preview());
            }
            Input::RuleLikeThis => {
                let Some(message) = rule.like_this else {
                    return Vec::new();
                };
                let stamp = rule.stamp;
                return vec![Step::Ask(Request::DigestLikeThis { message, stamp })];
            }
            Input::RuleSchedule(schedule) => {
                rule.schedule = schedule;
                rule.error = None;
            }
            Input::RuleCreate => match draft(rule) {
                Ok(draft) => {
                    rule.error = None;
                    return vec![Step::Ask(Request::SaveDigestRule {
                        replacing: rule.replacing.clone(),
                        draft,
                        stamp: rule.stamp,
                    })];
                }
                Err(sentence) => rule.error = Some(sentence),
            },
            _ => return Vec::new(),
        }
        let mut drawn = self.draw_rule();
        drawn.extend(steps);
        drawn
    }

    /// An answer for the rule dialog.
    pub(crate) fn rule_reply(&mut self, reply: Reply) -> Vec<Step> {
        let stamp = match &reply {
            Reply::DigestPreview { stamp, .. }
            | Reply::LikeThis { stamp, .. }
            | Reply::RuleSaved { stamp, .. } => *stamp,
            _ => return Vec::new(),
        };
        let Some(rule) = self.rule.as_mut().filter(|rule| rule.stamp == stamp) else {
            return Vec::new();
        };
        match reply {
            Reply::DigestPreview { answer, .. } => match answer {
                Ok(preview) => {
                    rule.preview = Some(preview);
                    rule.error = None;
                }
                Err(error) => rule.error = Some(error),
            },
            Reply::LikeThis { answer, .. } => match answer {
                Ok(Some(queries)) => {
                    set_query(rule, &queries.join(", "));
                    let mut steps = self.draw_rule();
                    steps.extend(self.read_preview());
                    return steps;
                }
                Ok(None) => rule.error = Some(words::NOTHING_ALIKE.to_owned()),
                Err(error) => rule.error = Some(error),
            },
            Reply::RuleSaved { answer, .. } => match answer {
                Ok(()) => {
                    let name = rule.name.clone();
                    self.rule = None;
                    self.surfaces.dismiss(SurfaceKind::Dialog);
                    let mut steps = vec![
                        Step::Show(Intent::CloseSurface(SurfaceKind::Dialog)),
                        Step::Show(Intent::Toast {
                            text: postio_ui::focus_target::rule_saved(&name),
                            kind: ToastKind::Notice,
                        }),
                    ];
                    if !self.has_surface() {
                        steps.push(Step::Show(Intent::KeyboardHome));
                    }
                    return steps;
                }
                Err(error) => rule.error = Some(error),
            },
            _ => {}
        }
        self.draw_rule()
    }

    fn draw_rule(&self) -> Vec<Step> {
        self.rule_view()
            .map(|view| vec![Step::Show(Intent::Rule(Box::new(view)))])
            .unwrap_or_default()
    }

    /// The rule dialog in words, as it stands.
    pub(crate) fn rule_view(&self) -> Option<RuleView> {
        let rule = self.rule.as_ref()?;
        let now = self.now();
        let senders = rule.query.is_none();
        Some(RuleView {
            heading: rule.heading.clone(),
            from_label: words::FROM.to_owned(),
            from: rule.from.clone().filter(|_| senders),
            query: rule.query.clone(),
            placeholder: words::QUERY_PLACEHOLDER.to_owned(),
            match_instead: senders.then(|| words::MATCH_INSTEAD.to_owned()),
            like_this: (senders && rule.like_this.is_some()).then(|| words::LIKE_THIS.to_owned()),
            deliver_label: words::DELIVER.to_owned(),
            schedule: rule.schedule.clone(),
            cadences: words::CADENCES
                .iter()
                .map(|(_, name)| (*name).to_owned())
                .collect(),
            weekdays: words::WEEKDAYS
                .iter()
                .map(|day| {
                    chrono::NaiveDate::from_isoywd_opt(2026, 1, *day)
                        .map(|date| date.format("%A").to_string())
                        .unwrap_or_default()
                })
                .collect(),
            note: rule.note.clone(),
            create: words::create_words(rule.replacing.is_some()).to_owned(),
            create_key: hints::key(self.keymap(), CommandId::PickerConfirm),
            preview_heading: rule
                .preview
                .as_ref()
                .map(|preview| words::preview_heading(preview.count)),
            preview: rule
                .preview
                .as_ref()
                .map(|preview| {
                    preview
                        .first
                        .iter()
                        .map(|row| RulePreviewLine {
                            subject: row.subject.clone().unwrap_or_default(),
                            day: words::preview_day(
                                row.received_at.with_timezone(&chrono::Local),
                                now,
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            more: rule
                .preview
                .as_ref()
                .and_then(|preview| words::preview_more(preview.count, preview.first.len())),
            error: rule.error.clone(),
        })
    }
}

/// The query entry up, holding `text`: one query per comma-separated piece
/// becomes the rule's `match`, and its name (US14 scenario 2).
fn set_query(rule: &mut RuleDialog, text: &str) {
    let queries = words::split_queries(text);
    rule.name = queries.join(", ");
    rule.queries = queries;
    rule.query = Some(text.to_owned());
    rule.error = None;
}

/// The rule as the dialog says it, or why it cannot be one.
fn draft(rule: &RuleDialog) -> Result<DigestRuleDraft, String> {
    let (cadence, day, at) = match rule.schedule.due()? {
        Due::Daily { at } => (Cadence::Daily, None, at),
        Due::Weekly { day, at } => (Cadence::Weekly, Some(RuleDay::Weekday(day)), at),
        Due::Monthly { day, at } => (Cadence::Monthly, Some(RuleDay::OfMonth(day)), at),
    };
    Ok(DigestRuleDraft {
        name: rule.name.clone(),
        queries: rule.queries.clone(),
        cadence,
        day,
        at,
    })
}
