//! Digest rules, as the rule dialog and the `g d` list change them (spec 007
//! FR-120, FR-126, FR-127): a rule's preview through the executor, a rule
//! written to `config.toml`, and a rule removed with what it held released.
//!
//! A rule is the person's decision, so it lives in `config.toml` and is
//! written as an editor saves it (`postio_session::focus::FocusSettings`).
//! Nothing here logs a query or a name: they are the person's words.

use chrono::{DateTime, Local, Timelike as _, Utc};
use postio_client::protocol::{DigestPreview, DigestRuleDraft, RuleDay};
use postio_config::{DigestRule, Due};
use postio_core::Event;
use postio_model::listing::{Cadence, StoreError};
use postio_model::{AccountScope, MessageId};
use postio_search::ResultOrder;
use postio_search::facets::Scope;
use postio_storage::repository::{DigestRepository, ThreadRepository};

use crate::Inner;

/// How many of a preview's messages the dialog lists: "then the first four".
const FIRST: usize = 4;

/// What a rule matching `queries` would have caught since `since`: each
/// query run through the executor as search runs it, over every account's
/// mail, newest first -- its count, and the newest four rows (FR-127: a
/// rule means what the same query means in search).
///
/// A message two of the rule's queries both match counts once for each.
/// The dialog writes one `from:` query per sender, which no message meets
/// twice.
pub(crate) async fn preview(
    inner: &Inner,
    queries: &[String],
    since: DateTime<Utc>,
) -> Result<DigestPreview, StoreError> {
    let reader = inner.wiring.database.read().await?;
    let today = Local::now().date_naive();
    let window = since.format("%Y-%m-%d");
    let mut count = 0u64;
    let mut newest: Vec<(DateTime<Utc>, MessageId)> = Vec::new();
    for query in queries {
        let parsed = postio_search::parse(&format!("{query} after:{window}"), today);
        let found = postio_index::search(
            &reader,
            &postio_index::SearchRequest {
                account: AccountScope::Unified,
                query: &parsed,
                scope: Scope::default(),
                limit: FIRST as u32,
                order: ResultOrder::Newest,
            },
            Utc::now(),
        )
        .await
        .map_err(|_| StoreError::new("Postio could not run that rule over your mail"))?;
        count = count.saturating_add(found.total_hits);
        newest.extend(
            found
                .hits
                .iter()
                .map(|hit| (hit.received_at, hit.message_id)),
        );
    }
    newest.sort_by(|a, b| b.cmp(a));
    let mut ids: Vec<MessageId> = Vec::with_capacity(FIRST);
    for (_, id) in newest {
        if !ids.contains(&id) && ids.len() < FIRST {
            ids.push(id);
        }
    }
    let first = inner.wiring.store.message_rows(ids).await?;
    Ok(DigestPreview {
        count: u32::try_from(count).unwrap_or(u32::MAX),
        first,
    })
}

/// Write `draft` to `config.toml`'s `[[focus.digests]]`: a new rule, or the
/// rule called `replacing` rewritten where it stands. Refused, with a
/// sentence for the dialog, when the rule would not apply.
pub(crate) async fn save(
    inner: &Inner,
    replacing: Option<String>,
    draft: DigestRuleDraft,
) -> Result<(), StoreError> {
    let due = due_of(&draft)?;
    let rule = DigestRule::from_due(draft.name.trim(), draft.queries, due);
    if rule.name.is_empty() {
        return Err(StoreError::new("A digest rule needs a name"));
    }
    if rule.queries.is_empty() {
        return Err(StoreError::new(
            "A digest rule needs a sender or a query to hold",
        ));
    }
    if !postio_ui::digest::unreadable_queries(&rule, Local::now().date_naive()).is_empty() {
        return Err(StoreError::new(
            "Postio cannot read one of that rule's queries",
        ));
    }
    let taken = inner.wiring.focus.config().is_some_and(|config| {
        config.digests.iter().any(|other| {
            other.name.trim() == rule.name
                && replacing.as_deref().map(str::trim) != Some(&rule.name)
        })
    });
    if taken {
        return Err(StoreError::new(
            "There is a digest rule with that name already",
        ));
    }
    inner
        .wiring
        .focus
        .write(move |text| {
            postio_config::focus_edit::put_digest_rule(text, replacing.as_deref(), &rule, None)
                .map(Some)
                .map_err(|_| "Postio could not write that rule to config.toml".to_owned())
        })
        .await
        .map(|_| ())
        .map_err(StoreError::new)
}

/// Take the rule called `name` out of `config.toml` and release what it held
/// and had not delivered into the inbox (FR-126); answers how many messages
/// it released. What a delivery already holds stays in that digest, on
/// screen.
pub(crate) async fn delete(inner: &Inner, name: String) -> Result<u32, StoreError> {
    let removing = name.clone();
    inner
        .wiring
        .focus
        .write(move |text| {
            postio_config::focus_edit::remove_digest_rule(text, &removing)
                .map(|removed| removed.map(|(text, _, _)| text))
                .map_err(|_| "Postio could not take that rule out of config.toml".to_owned())
        })
        .await
        .map_err(StoreError::new)?;
    let connection = inner.wiring.database.connect().await?;
    let released = DigestRepository::new(&connection)
        .release_rule(name.trim())
        .await?;
    if released > 0 {
        // What was held rejoins Focus's inbox, which is every inbox.
        let inboxes = ThreadRepository::new(&connection).unified_inboxes().await?;
        for (account, mailbox) in inboxes {
            inner
                .hub
                .emit(Event::MessageListChanged { account, mailbox });
        }
    }
    Ok(u32::try_from(released).unwrap_or(u32::MAX))
}

/// When `draft` says it comes: its cadence, day and time.
fn due_of(draft: &DigestRuleDraft) -> Result<Due, StoreError> {
    let at = draft
        .at
        .with_second(0)
        .and_then(|at| at.with_nanosecond(0))
        .unwrap_or(draft.at);
    Ok(match (draft.cadence, draft.day) {
        (Cadence::Daily, None) => Due::Daily { at },
        (Cadence::Weekly, Some(RuleDay::Weekday(day))) => Due::Weekly { day, at },
        (Cadence::Monthly, Some(RuleDay::OfMonth(day))) if (1..=28).contains(&day) => {
            Due::Monthly { day, at }
        }
        (Cadence::Daily, Some(_)) => {
            return Err(StoreError::new(
                "A daily digest comes every day, so it takes no day",
            ));
        }
        (Cadence::Weekly, _) => return Err(StoreError::new("A weekly digest needs a weekday")),
        (Cadence::Monthly, _) => {
            return Err(StoreError::new("A monthly digest needs a day from 1 to 28"));
        }
    })
}
