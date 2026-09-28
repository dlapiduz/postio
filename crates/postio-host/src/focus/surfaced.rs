//! The rows Focus's inbox surfaces among its conversations (spec 007,
//! contracts/engine.md "Reads", data-model.md "Surfaced rows"): each digest
//! delivered and not archived, and each reminder that fired and still
//! stands, with when it came due and where it goes.
//!
//! What it reads, whatever the inbox holds: the open deliveries with their
//! counts and kept summaries (one statement), their senders (one), the
//! surfaced reminders on conversations still in an inbox (one), and for
//! each row its place (one) -- and for a reminder its
//! conversation's latest message (one, and its row). No body is read.

use postio_config::FocusConfig;
use postio_model::listing::StoreError;
use postio_model::listing::{Cadence, Surfaced};
use postio_storage::repository::{DigestRepository, ReminderRepository, ThreadRepository};

use crate::Inner;

/// How many senders a digest row names: its line has room for a few, and
/// the count says the rest.
const SENDERS_SHOWN: usize = 6;

/// The surfaced rows, newest first.
pub(crate) async fn surfaced(inner: &Inner) -> Result<Vec<Surfaced>, StoreError> {
    let config = inner.wiring.focus.config();
    let reader = inner.wiring.database.read().await?;
    let threads = ThreadRepository::new(&reader);
    let inboxes = threads.unified_inboxes().await?;
    let digests = DigestRepository::new(&reader);

    let deliveries = digests.open_deliveries().await?;
    let ids: Vec<_> = deliveries.iter().map(|delivery| delivery.id).collect();
    let senders = digests.senders_of(&ids).await?;
    let mut rows = Vec::with_capacity(deliveries.len());
    for delivery in deliveries {
        let named: Vec<_> = senders
            .iter()
            .filter(|(of, _, _)| *of == delivery.id)
            .take(SENDERS_SHOWN)
            .map(|(_, sender, _)| sender.clone())
            .collect();
        rows.push(Surfaced::Digest {
            delivery: delivery.id,
            cadence: config
                .as_ref()
                .and_then(|config| cadence_of(config, &delivery.rule)),
            rule: delivery.rule,
            count: delivery.count,
            senders: named,
            summary_line: super::summary::line_of(delivery.summary.as_deref()),
            at: delivery.due_at,
            position: threads.focus_position(&inboxes, delivery.due_at).await?,
        });
    }

    let mailboxes: Vec<_> = inboxes.iter().map(|(_, inbox)| *inbox).collect();
    for reminder in ReminderRepository::new(&reader)
        .surfaced_in(&mailboxes)
        .await?
    {
        let Some(fired_at) = reminder.fired_at else {
            continue;
        };
        let Some(latest) = threads.latest_member(reminder.thread).await? else {
            continue;
        };
        let Some(representative) = inner
            .wiring
            .store
            .message_rows(vec![latest])
            .await?
            .into_iter()
            .next()
        else {
            continue;
        };
        rows.push(Surfaced::Reminder {
            reminder: reminder.id,
            thread: reminder.thread,
            since: reminder.set_at,
            representative,
            at: fired_at,
            position: threads.focus_position(&inboxes, fired_at).await?,
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.at()));
    Ok(rows)
}

/// How often the rule called `name` comes, as `config` has it, when it is
/// still there and reads.
fn cadence_of(config: &FocusConfig, name: &str) -> Option<Cadence> {
    let rule = config
        .digests
        .iter()
        .find(|rule| rule.name.trim() == name.trim())?;
    Some(match rule.due().ok()? {
        postio_config::Due::Daily { .. } => Cadence::Daily,
        postio_config::Due::Weekly { .. } => Cadence::Weekly,
        postio_config::Due::Monthly { .. } => Cadence::Monthly,
    })
}
