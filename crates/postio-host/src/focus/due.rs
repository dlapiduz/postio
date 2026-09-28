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
use postio_storage::repository::DigestRepository;
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
