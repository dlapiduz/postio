//! Focus's markers (spec 007): what a conversation's row calls out -- an
//! invitation, a question, a to-do -- at most one per message.
//!
//! # Who writes, who reads
//!
//! Focus's body task writes them (T110, T117), and the filing pass will;
//! the row reads them with its page, one batched statement for the whole
//! page (`ThreadRepository::focus_page_at`), so drawing a row reads no body
//! (FR-020). Nothing the classic app or the terminal lists ever reads this
//! table.
//!
//! # A dismissal is for good
//!
//! A marker the person dismissed never comes back on that message (FR-108):
//! [`MarkerRepository::insert`] leaves a message that already has a marker
//! alone, and [`MarkerRepository::replace`], which an invitation update
//! uses, keeps the dismissal as it found it. Only
//! [`MarkerRepository::dismiss`] itself, taking the dismissal back as undo
//! does, lifts it.

use chrono::{DateTime, Utc};
use postio_model::ids::MessageId;
use postio_model::listing::{InviteAnswer, MarkerKind};

use super::{from_millis, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _};
use crate::store::Connection;
use turso::Row;

/// What made a marker (FR-104 to FR-108, SC-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerSource {
    /// A calendar part in the message: an invitation.
    Calendar,
    /// The built-in needs-action detector.
    Detector,
    /// The person's own local model (milestone 2).
    Model,
    /// A reminder that came due with no reply.
    Reminder,
}

/// Where an invitation stands, apart from how the person answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InviteState {
    /// The event is ahead and can be answered.
    Open,
    /// The organiser cancelled it: the marker says so and offers no answer.
    Cancelled,
    /// The event is over: the marker shows it and offers no answer.
    Past,
}

/// An invitation's iTIP identity: what decides whether a later invitation
/// replaces or cancels this one (research R9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteIdentity {
    /// The event's `UID`.
    pub uid: String,
    /// Its `SEQUENCE`.
    pub sequence: i64,
    /// Its `DTSTAMP`, when it carries one.
    pub stamp: Option<DateTime<Utc>>,
}

/// One message's marker, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The message the marker is about.
    pub message: MessageId,
    /// What it calls out.
    pub kind: MarkerKind,
    /// What made it.
    pub source: MarkerSource,
    /// Character offsets into the message's own text: where the sentence is.
    /// `None` for an invitation or a reminder.
    pub span: Option<(u32, u32)>,
    /// The sentence, verbatim and short. `None` for an invitation.
    pub excerpt: Option<String>,
    /// An invitation's event.
    pub starts_at: Option<DateTime<Utc>>,
    /// When the event ends.
    pub ends_at: Option<DateTime<Utc>>,
    /// A to-do's due date; for a reminder, the day it was set.
    pub due_at: Option<DateTime<Utc>>,
    /// The invitation this marker is about, for an invitation.
    pub invite: Option<InviteIdentity>,
    /// Where the invitation stands, for an invitation.
    pub invite_state: Option<InviteState>,
    /// How the person answered the invitation, once they have.
    pub answer: Option<InviteAnswer>,
    /// When the person dismissed it, if they have.
    pub dismissed_at: Option<DateTime<Utc>>,
}

/// Reads and writes [`Marker`]s.
#[derive(Debug)]
pub struct MarkerRepository<'a> {
    connection: &'a Connection,
}

impl<'a> MarkerRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Writes `marker` for its message, unless the message has one already,
    /// and answers whether it wrote.
    ///
    /// A message's marker is decided once: classifying it again, or a
    /// newer classifier catching up, does not write over what is there --
    /// least of all over a dismissal (FR-108). An invitation update is
    /// [`Self::replace`].
    pub async fn insert(&self, marker: &Marker) -> Result<bool> {
        let written = sql::execute(
            self.connection,
            &format!(
                "INSERT OR IGNORE INTO markers ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
            ),
            values(marker),
        )
        .await?;
        Ok(written > 0)
    }

    /// Replaces the marker on `marker.message` with `marker`, and answers
    /// whether there was one to replace.
    ///
    /// For an invitation update: a later `SEQUENCE` moves the event or
    /// cancels it, and the marker the older invitation left takes the newer
    /// one's time, identity, state and answer. Whether it *is* newer is the
    /// calendar adapter's to say (`postio_calendar::supersedes`), not this.
    /// A dismissal stands: `marker.dismissed_at` is not written, so a
    /// marker the person dismissed stays dismissed through any update.
    pub async fn replace(&self, marker: &Marker) -> Result<bool> {
        let replaced = sql::execute(
            self.connection,
            "UPDATE markers
                SET kind = ?2, source = ?3, span_start = ?4, span_end = ?5, excerpt = ?6,
                    starts_at = ?7, ends_at = ?8, due_at = ?9, invite_uid = ?10,
                    invite_sequence = ?11, invite_stamp = ?12, invite_state = ?13, answer = ?14
              WHERE message_id = ?1",
            values(marker)[..14].to_vec(),
        )
        .await?;
        Ok(replaced > 0)
    }

    /// Dismisses the marker on `message` at `at`, or with `None` takes the
    /// dismissal back as undo does, and answers whether the message has a
    /// marker at all.
    pub async fn dismiss(&self, message: MessageId, at: Option<DateTime<Utc>>) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE markers SET dismissed_at = ?2 WHERE message_id = ?1",
            vec![
                turso::Value::Integer(message.get()),
                at.map(to_millis)
                    .map_or(turso::Value::Null, turso::Value::Integer),
            ],
        )
        .await?;
        Ok(changed > 0)
    }

    /// Records how the person answered the invitation on `message`, and,
    /// while the reply waits out its window, when the window closes
    /// (research R9); `None` for both takes an answer back, as undo within
    /// the window does. Answers whether the message has a marker.
    pub async fn answer(
        &self,
        message: MessageId,
        answer: Option<InviteAnswer>,
        until: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        use turso::Value::{Integer, Null, Text};
        let changed = sql::execute(
            self.connection,
            "UPDATE markers SET answer = ?2, answer_until = ?3 WHERE message_id = ?1",
            vec![
                Integer(message.get()),
                answer.map_or(Null, |answer| Text(answer_name(answer).to_owned())),
                until.map_or(Null, |until| Integer(to_millis(until))),
            ],
        )
        .await?;
        Ok(changed > 0)
    }

    /// The messages whose answer's window has closed by `now`: what Focus's
    /// due timer makes final each tick. One statement, a seek on
    /// `idx_markers_answer_until`.
    pub async fn answers_due(&self, now: DateTime<Utc>) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            Self::explain_answers_due(),
            [to_millis(now)],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::answers_due`] runs.
    pub fn explain_answers_due() -> &'static str {
        "SELECT message_id FROM markers WHERE answer_until <= ?1 ORDER BY answer_until LIMIT 256"
    }

    /// Makes the answer on `message` final, its window closed: `accepting`
    /// becomes `accepted` and `declining` `declined`. Answers whether there
    /// was one waiting.
    pub async fn settle_answer(&self, message: MessageId) -> Result<bool> {
        let settled = sql::execute(
            self.connection,
            "UPDATE markers
                SET answer = CASE answer WHEN 'accepting' THEN 'accepted'
                                         WHEN 'declining' THEN 'declined'
                                         ELSE answer END,
                    answer_until = NULL
              WHERE message_id = ?1 AND answer_until IS NOT NULL",
            [message.get()],
        )
        .await?;
        Ok(settled > 0)
    }

    /// The marker on `message`, dismissed or not: what the open message's
    /// marker card reads.
    pub async fn get(&self, message: MessageId) -> Result<Option<Marker>> {
        sql::first(
            self.connection,
            &format!("SELECT {COLUMNS} FROM markers WHERE message_id = ?1"),
            [message.get()],
            read_marker,
        )
        .await
    }

    /// Every marker about the invitation `uid`, in message order: the ones a
    /// later `REQUEST` or `CANCEL` for the same event may replace. One seek
    /// on `idx_markers_invite`.
    pub async fn invitations(&self, uid: &str) -> Result<Vec<Marker>> {
        sql::all(
            self.connection,
            &format!("SELECT {COLUMNS} FROM markers WHERE invite_uid = ?1 ORDER BY message_id"),
            [uid],
            read_marker,
        )
        .await
    }
}

/// Every column, in the order [`values`] binds them and [`read_marker`]
/// reads them.
const COLUMNS: &str = "message_id, kind, source, span_start, span_end, excerpt, starts_at, \
                       ends_at, due_at, invite_uid, invite_sequence, invite_stamp, \
                       invite_state, answer, dismissed_at";

/// `marker`'s columns as parameters, `?1` the message.
fn values(marker: &Marker) -> Vec<turso::Value> {
    use turso::Value::{Integer, Null, Text};
    let integer = |value: Option<i64>| value.map_or(Null, Integer);
    let time = |value: Option<DateTime<Utc>>| integer(value.map(to_millis));
    let text = |value: Option<&str>| value.map_or(Null, |value| Text(value.to_owned()));
    let invite = marker.invite.as_ref();
    vec![
        Integer(marker.message.get()),
        Text(kind_name(marker.kind).to_owned()),
        Text(source_name(marker.source).to_owned()),
        integer(marker.span.map(|(start, _)| i64::from(start))),
        integer(marker.span.map(|(_, end)| i64::from(end))),
        text(marker.excerpt.as_deref()),
        time(marker.starts_at),
        time(marker.ends_at),
        time(marker.due_at),
        text(invite.map(|invite| invite.uid.as_str())),
        integer(invite.map(|invite| invite.sequence)),
        time(invite.and_then(|invite| invite.stamp)),
        text(marker.invite_state.map(state_name)),
        text(marker.answer.map(answer_name)),
        time(marker.dismissed_at),
    ]
}

fn read_marker(row: &Row) -> Result<Marker> {
    let time = |index: usize| -> Result<Option<DateTime<Utc>>> {
        Ok(row.col::<Option<i64>>(index)?.map(from_millis))
    };
    let span = match (row.col::<Option<i64>>(3)?, row.col::<Option<i64>>(4)?) {
        (Some(start), Some(end)) => Some((offset(start)?, offset(end)?)),
        _ => None,
    };
    let invite = match row.col::<Option<String>>(9)? {
        Some(uid) => Some(InviteIdentity {
            uid,
            sequence: row.col::<Option<i64>>(10)?.unwrap_or_default(),
            stamp: time(11)?,
        }),
        None => None,
    };
    Ok(Marker {
        message: MessageId::new(row.col(0)?),
        kind: kind_of(&row.col::<String>(1)?)?,
        source: source_of(&row.col::<String>(2)?)?,
        span,
        excerpt: row.col(5)?,
        starts_at: time(6)?,
        ends_at: time(7)?,
        due_at: time(8)?,
        invite,
        invite_state: row
            .col::<Option<String>>(12)?
            .as_deref()
            .map(state_of)
            .transpose()?,
        answer: row
            .col::<Option<String>>(13)?
            .as_deref()
            .map(answer_of)
            .transpose()?,
        dismissed_at: time(14)?,
    })
}

/// A stored character offset, which a negative number cannot be.
fn offset(value: i64) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::ColumnType {
        column: "markers.span_start".to_owned(),
        reason: format!("a character offset, and {value} is out of range"),
    })
}

pub(crate) fn kind_name(kind: MarkerKind) -> &'static str {
    match kind {
        MarkerKind::Invite => "invite",
        MarkerKind::Question => "question",
        MarkerKind::Todo => "todo",
        MarkerKind::NoReply => "no_reply",
    }
}

pub(crate) fn kind_of(name: &str) -> Result<MarkerKind> {
    Ok(match name {
        "invite" => MarkerKind::Invite,
        "question" => MarkerKind::Question,
        "todo" => MarkerKind::Todo,
        "no_reply" => MarkerKind::NoReply,
        other => return Err(unknown_enum("markers.kind", other)),
    })
}

fn source_name(source: MarkerSource) -> &'static str {
    match source {
        MarkerSource::Calendar => "calendar",
        MarkerSource::Detector => "detector",
        MarkerSource::Model => "model",
        MarkerSource::Reminder => "reminder",
    }
}

fn source_of(name: &str) -> Result<MarkerSource> {
    Ok(match name {
        "calendar" => MarkerSource::Calendar,
        "detector" => MarkerSource::Detector,
        "model" => MarkerSource::Model,
        "reminder" => MarkerSource::Reminder,
        other => return Err(unknown_enum("markers.source", other)),
    })
}

fn state_name(state: InviteState) -> &'static str {
    match state {
        InviteState::Open => "open",
        InviteState::Cancelled => "cancelled",
        InviteState::Past => "past",
    }
}

pub(crate) fn state_of(name: &str) -> Result<InviteState> {
    Ok(match name {
        "open" => InviteState::Open,
        "cancelled" => InviteState::Cancelled,
        "past" => InviteState::Past,
        other => return Err(unknown_enum("markers.invite_state", other)),
    })
}

fn answer_name(answer: InviteAnswer) -> &'static str {
    match answer {
        InviteAnswer::Accepting => "accepting",
        InviteAnswer::Accepted => "accepted",
        InviteAnswer::Declining => "declining",
        InviteAnswer::Declined => "declined",
    }
}

pub(crate) fn answer_of(name: &str) -> Result<InviteAnswer> {
    Ok(match name {
        "accepting" => InviteAnswer::Accepting,
        "accepted" => InviteAnswer::Accepted,
        "declining" => InviteAnswer::Declining,
        "declined" => InviteAnswer::Declined,
        other => return Err(unknown_enum("markers.answer", other)),
    })
}
