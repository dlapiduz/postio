//! Focus's due timer: on the engine's tick, every digest whose time has come is delivered.
//! A rule delivers as of the latest due time passed, so several missed deliveries arrive once,
//! never empty; the first tick is at once.

use chrono::{DateTime, TimeZone, Utc};
use postio_config::FocusConfig;
use postio_model::EmailAddress;
use postio_storage::repository::{
    DigestRepository, IdentityRepository, MarkerRepository, MessageRepository, ReminderRepository,
};
use postio_storage::{Store, WritePriority};
use postio_ui::schedule::next_due;

type Failure = postio_storage::Error;

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

/// A reminder due while Focus was closed fires at the first tick, offline too. It is never fired
/// over a reply:
/// a reply can reach the store by a way the filing pass never sees (another app's folder, a first
/// sync), so
/// the timer checks who has written since and cancels instead.
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

/// The person's own reply is filed in Sent, which the filing pass does not read, and another app's
/// folder
/// can bring any reply; so each tick checks the surfaced conversations (one seek each, a few at
/// most).
pub(crate) async fn settle_replied(database: &Store, now: DateTime<Utc>) -> Result<usize, Failure> {
    let surfaced = {
        let reader = database.read().await?;
        ReminderRepository::new(&reader).surfaced().await?
    };
    let mut replied = Vec::new();
    {
        let reader = database.read().await?;
        let reminders = ReminderRepository::new(&reader);
        for reminder in surfaced {
            let Some(fired_at) = reminder.fired_at else {
                continue;
            };
            if !reminders
                .writers_since(reminder.thread, fired_at)
                .await?
                .is_empty()
            {
                replied.push(reminder.id);
            }
        }
    }
    if replied.is_empty() {
        return Ok(0);
    }
    let connection = database.connect_background().await?;
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    let reminders = ReminderRepository::new(&connection);
    let mut settled = 0;
    for reminder in replied {
        if reminders.settle(reminder, now).await? {
            settled += 1;
        }
    }
    Ok(settled)
}

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
