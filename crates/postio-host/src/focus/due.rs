//! Focus's due timer (spec 007 T135, `contracts/engine.md`, "Focus mode in
//! the host"): on the engine's tick, every digest whose time has come is
//! delivered.
//!
//! A rule's next delivery is the first time its cadence names after the
//! oldest mail it holds that no delivery has taken
//! (`postio_ui::schedule::next_due`, on calendar days, safe across a change
//! of the clocks). When that time has passed, everything the rule holds is
//! delivered as one digest (FR-123):
//!
//! - **as of the latest due time that has passed**, so a digest that came
//!   due several times while Focus was closed arrives once, holding
//!   everything since the last delivery, and nothing is lost or doubled;
//! - **never empty**: a rule holding nothing makes no row (US10 scenario 6);
//! - **at start**: the timer's first tick is at once, so what came due while
//!   Focus was closed is there when it opens (US10 scenario 3).

use chrono::{DateTime, TimeZone, Utc};
use postio_config::FocusConfig;
use postio_model::EmailAddress;
use postio_storage::repository::{
    DigestRepository, IdentityRepository, MarkerRepository, MessageRepository, ReminderRepository,
};
use postio_storage::{Store, WritePriority};
use postio_ui::schedule::next_due;

/// Why the timer could not read or deliver.
type Failure = postio_storage::Error;

/// Deliver every applicable rule in `config` that has come due by `now`, in
/// `now`'s zone, the one a rule's time is read in. Answers how many digests
/// it delivered.
pub(crate) async fn deliver_due<Tz>(
    database: &Store,
    config: &FocusConfig,
    now: &DateTime<Tz>,
) -> Result<usize, Failure>
where
    Tz: TimeZone,
{
    let rules = config.applicable_digests();
    if rules.is_empty() {
        return Ok(0);
    }
    let connection = database.connect_background().await?;
    let zone = now.timezone();
    let mut delivered = 0;
    for (rule, due) in rules {
        let name = rule.name.trim();
        let Some(since) = DigestRepository::new(&connection)
            .waiting_since(name)
            .await?
        else {
            continue;
        };
        let Some(mut due_at) = next_due(&due, &since.with_timezone(&zone)) else {
            continue;
        };
        if due_at > *now {
            continue;
        }
        // The latest time it came due: one digest for all of them.
        while let Some(next) = next_due(&due, &due_at).filter(|next| next <= now) {
            due_at = next;
        }
        let _permit = connection
            .write_gate()
            .acquire(WritePriority::Background)
            .await;
        let made = DigestRepository::new(&connection)
            .deliver(name, due_at.with_timezone(&Utc), now.with_timezone(&Utc))
            .await?;
        delivered += usize::from(made.is_some());
    }
    Ok(delivered)
}

/// Fire every reminder that has come due by `now` with no reply (spec 007
/// US5): each surfaces at the top of Focus's inbox, marked "No reply
/// since" the day it was set. Answers how many fired.
///
/// - **At start too**: the timer's first tick is at once, so a reminder that
///   came due while Focus was closed surfaces as it opens (FR-045). It needs
///   nothing but the store, so it fires offline as well.
/// - **Never over a reply** (FR-044): Focus's filing pass cancels a reminder
///   as a reply from somebody else is filed, but a reply can reach the store
///   by a way that pass never sees -- a folder another app synced, a first
///   sync. So before firing, the timer asks who has written in the
///   conversation since the reminder was set, and one who is not the person
///   cancels it instead.
pub(crate) async fn fire_reminders(database: &Store, now: DateTime<Utc>) -> Result<usize, Failure> {
    let due = {
        let reader = database.read().await?;
        ReminderRepository::new(&reader).due(now).await?
    };
    if due.is_empty() {
        return Ok(0);
    }
    let connection = database.connect_background().await?;
    let own: Vec<String> = IdentityRepository::new(&connection)
        .own_addresses()
        .await?
        .iter()
        .map(EmailAddress::normalized)
        .collect();
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    let reminders = ReminderRepository::new(&connection);
    let mut fired = 0;
    for reminder in due {
        let replied = reminders
            .writers_since(reminder.thread, reminder.set_at)
            .await?
            .iter()
            .any(|writer| !own.contains(writer));
        if replied {
            reminders.cancel(reminder.id, now).await?;
        } else if reminders.fire(reminder.id, now).await? {
            fired += 1;
        }
    }
    Ok(fired)
}

/// Make final every answer whose window has closed by `now` (spec 007
/// research R9): `accepting` becomes `accepted`, `declining` `declined`.
/// Answers each message it settled, with its account.
pub(crate) async fn settle_answers(
    database: &Store,
    now: DateTime<Utc>,
) -> Result<Vec<(postio_model::AccountId, postio_model::MessageId)>, Failure> {
    let due = {
        let reader = database.read().await?;
        MarkerRepository::new(&reader).answers_due(now).await?
    };
    if due.is_empty() {
        return Ok(Vec::new());
    }
    let connection = database.connect_background().await?;
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    let mut settled = Vec::with_capacity(due.len());
    for message in due {
        if !MarkerRepository::new(&connection)
            .settle_answer(message)
            .await?
        {
            continue;
        }
        if let Some(row) = MessageRepository::new(&connection).get(message).await? {
            settled.push((row.account_id, message));
        }
    }
    Ok(settled)
}
