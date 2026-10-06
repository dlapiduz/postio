//! The digest window's state (terminal.md, "Digests"): one delivery, what it
//! holds, its summary and which page of it is showing.
//!
//! The window is the message frame. It opens on the summary when one is
//! written and on the plain list otherwise; a message from either opens
//! inside it; `Esc` goes back to where that came from. What it reads is
//! `crate::ask::Ask::Digest`; the drawing is `view::digest`. Everything a
//! sender or the model wrote is made [`SafeText`] here, once.

use chrono::{DateTime, Utc};
use postio_model::listing::{Cadence, MessageSummary};
use postio_model::summary::{DigestSummary, SummaryReference, SummaryStatement};
use postio_model::{DeliveryId, MessageId};
use postio_ui::digest::{self, Page};
use postio_ui::terminal::SafeText;

use crate::row::Row;

/// The email on screen, opened from a reference or from the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    /// Which message.
    pub message: MessageId,
    /// The reference that cites it; `0` when the list opened it.
    pub number: u32,
    /// The cited passage, to mark in the body.
    pub excerpt: Option<String>,
    /// The page `Esc` goes back to.
    pub from: Page,
}

/// The question over `D`: stop digesting this sender?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    /// The message whose sender it is.
    pub message: MessageId,
    /// Their address.
    pub sender: SafeText,
}

/// The window. See the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Which delivery.
    pub delivery: DeliveryId,
    /// The rule's name.
    pub rule: SafeText,
    /// How often it comes.
    pub cadence: Option<Cadence>,
    /// How many messages it holds.
    pub count: u32,
    /// How many people sent them.
    pub senders: usize,
    /// When it came due.
    pub at: DateTime<Utc>,
    /// When the rule delivers, as `config.toml` says it.
    pub rule_when: Option<String>,
    /// The messages, newest first, once read.
    rows: Vec<Row>,
    /// The summary, once read and when there is one.
    summary: Option<DigestSummary>,
    /// Whether the delivery has been read.
    read: bool,
    /// The page on screen.
    page: Page,
    /// The statement whose reference is focused.
    reference: Option<usize>,
    /// The message the keyboard is on in the list.
    cursor: usize,
    /// The first line in view.
    top: usize,
    /// The email on screen.
    email: Option<Email>,
    /// The question over stopping a sender, while it is up.
    stopping: Option<Stop>,
}

impl Window {
    /// A window on `delivery`, not yet read.
    pub fn new(
        delivery: DeliveryId,
        rule: &str,
        cadence: Option<Cadence>,
        count: u32,
        senders: usize,
        at: DateTime<Utc>,
        rule_when: Option<String>,
    ) -> Window {
        Window {
            delivery,
            rule: SafeText::new(rule),
            cadence,
            count,
            senders,
            at,
            rule_when,
            rows: Vec::new(),
            summary: None,
            read: false,
            page: Page::List,
            reference: None,
            cursor: 0,
            top: 0,
            email: None,
            stopping: None,
        }
    }

    /// The messages, newest first.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The summary, when there is one worth showing.
    pub fn summary(&self) -> Option<&DigestSummary> {
        self.summary.as_ref()
    }

    /// The page on screen.
    pub fn page(&self) -> Page {
        self.page
    }

    /// The focused statement, from 0.
    pub fn reference(&self) -> Option<usize> {
        self.reference
    }

    /// The message the keyboard is on in the list, by its place.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The first line in view.
    pub fn top(&self) -> usize {
        self.top
    }

    /// The email on screen.
    pub fn email(&self) -> Option<&Email> {
        self.email.as_ref()
    }

    /// The question over stopping a sender.
    pub fn stopping(&self) -> Option<&Stop> {
        self.stopping.as_ref()
    }

    /// Put the question up, or take it down.
    pub fn stop_asked(&mut self, stop: Option<Stop>) {
        self.stopping = stop;
    }

    /// Whether the delivery has been read.
    pub fn is_read(&self) -> bool {
        self.read
    }

    /// The delivery arrived: its messages, and its summary when it has one.
    /// The window opens on the summary only when one exists.
    pub fn read_in(&mut self, messages: Vec<MessageSummary>, summary: Option<DigestSummary>) {
        self.rows = messages.into_iter().map(Row::from).collect();
        self.summary = summary.filter(|summary| !summary.is_empty()).map(safe);
        self.read = true;
        self.page = Page::opening(self.summary.is_some());
        self.reference = self.summary.as_ref().map(|_| 0);
        self.cursor = 0;
        self.top = 0;
        self.sync_cursor();
    }

    /// Make the list's cursor follow the focused reference, so `D` and `U`
    /// act on its message.
    fn sync_cursor(&mut self) {
        let message = self
            .reference
            .and_then(|at| self.summary.as_ref()?.statements.get(at))
            .map(|statement| statement.reference.message);
        if let Some(at) = message.and_then(|message| self.position_of(message)) {
            self.cursor = at;
        }
    }

    fn position_of(&self, message: MessageId) -> Option<usize> {
        self.rows.iter().position(|row| row.id == message)
    }

    /// The row of `message`, when the delivery holds it.
    pub fn row_of(&self, message: MessageId) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == message)
    }

    /// The message `D` and `U` act on: the email on screen, else the focused
    /// reference's, else the list's cursor.
    pub fn focused(&self) -> Option<&Row> {
        if let Some(email) = &self.email {
            return self.row_of(email.message);
        }
        self.rows.get(self.cursor)
    }

    /// `Tab`: the other of the summary and the list, only while there is a
    /// summary.
    pub fn toggle(&mut self) -> bool {
        match self.page.toggled(self.summary.is_some()) {
            Some(page) => {
                self.page = page;
                self.top = 0;
                true
            }
            None => false,
        }
    }

    /// Show `page` when it can be shown.
    pub fn show(&mut self, page: Page) {
        let allowed = match page {
            Page::Summary => self.summary.is_some(),
            Page::List => true,
            Page::Email => self.email.is_some(),
        };
        if allowed && self.page != page {
            self.page = page;
            self.top = 0;
        }
    }

    /// `]` and `[`: the focused reference after a step.
    pub fn step_reference(&mut self, by: i32) {
        let len = self.summary.as_ref().map_or(0, |s| s.statements.len());
        if let Some(next) = digest::step_reference(self.reference, by, len) {
            self.reference = Some(next);
            self.sync_cursor();
        }
    }

    /// Focus the reference of statement `at`.
    pub fn focus_reference(&mut self, at: usize) {
        let len = self.summary.as_ref().map_or(0, |s| s.statements.len());
        if at < len {
            self.reference = Some(at);
            self.sync_cursor();
        }
    }

    /// The focused statement's reference.
    pub fn focused_reference(&self) -> Option<&SummaryReference> {
        self.summary
            .as_ref()?
            .statements
            .get(self.reference?)
            .map(|statement| &statement.reference)
    }

    /// Move the list's cursor by `by`, kept to the messages.
    pub fn step(&mut self, by: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Move the list's cursor to `at`.
    pub fn go_to(&mut self, at: usize) {
        if at < self.rows.len() {
            self.cursor = at;
        }
    }

    /// Open `message` in the window, from `from` the page `Esc` goes back
    /// to, citing the reference that names it when one does.
    pub fn open_email(&mut self, message: MessageId, from: Page) {
        let cited = self.summary.as_ref().and_then(|summary| {
            summary
                .statements
                .iter()
                .find(|statement| statement.reference.message == message)
                .map(|statement| &statement.reference)
        });
        let (number, excerpt) = match cited {
            Some(reference) if !reference.excerpt.is_empty() => {
                (reference.number, Some(reference.excerpt.clone()))
            }
            Some(reference) => (reference.number, None),
            None => (0, None),
        };
        if let Some(at) = self.position_of(message) {
            self.cursor = at;
        }
        self.email = Some(Email {
            message,
            number,
            excerpt,
            from,
        });
        self.page = Page::Email;
        self.top = 0;
    }

    /// Step to the previous or next source while an email is open.
    pub fn source_by(&mut self, by: isize) -> Option<MessageId> {
        let from = self.email.as_ref()?.from;
        let at = self.position_of(self.email.as_ref()?.message)?;
        let next = at
            .checked_add_signed(by)
            .filter(|next| *next < self.rows.len())?;
        let message = self.rows[next].id;
        self.open_email(message, from);
        Some(message)
    }

    /// `Esc` over an email: back to where it was opened from. Answers
    /// whether there was an email to leave.
    pub fn back(&mut self) -> bool {
        let Some(email) = self.email.take() else {
            return false;
        };
        self.page = match email.from {
            Page::Summary if self.summary.is_some() => Page::Summary,
            _ => Page::List,
        };
        self.top = 0;
        true
    }

    /// Scroll to `top`.
    pub fn scroll_to(&mut self, top: usize) {
        self.top = top;
    }
}

/// `summary`'s words, made safe to draw: they are the model's, from mail.
fn safe(summary: DigestSummary) -> DigestSummary {
    let clean = |text: &str| SafeText::new(text).as_str().to_owned();
    DigestSummary {
        statements: summary
            .statements
            .into_iter()
            .map(|statement| SummaryStatement {
                topic: clean(&statement.topic),
                text: clean(&statement.text),
                reference: SummaryReference {
                    excerpt: clean(&statement.reference.excerpt),
                    ..statement.reference
                },
            })
            .collect(),
        ..summary
    }
}
