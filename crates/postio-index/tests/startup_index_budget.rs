//! Starting up does not re-index the store.
//!
//! `docs/PRODUCT.md` §18 budgets startup to a usable UI at 500ms, and
//! `CLAUDE.md` calls it "enforced by benches in CI". It is not: `bench.yml`
//! compiles the bench targets and deliberately times nothing, because a
//! shared runner cannot defend a millisecond budget. That decision is right,
//! and it leaves the budget as documentation (#100).
//!
//! So count the budget's cause. [`ensure_schema`] runs on every start —
//! `postio_session::ensure_search_index` calls it, documented as "part of
//! opening the store, not part of searching it" — and when a schema version
//! has moved it drops the affected half and regenerates it from `messages`
//! in one pass of SQL. That pass is proportional to the mailbox, which is
//! fine once, on the start that upgrades, and would blow the 500ms budget on
//! a large store if it happened every time.
//!
//! The measure is nested statements — trigger firings. An index build is an
//! `INSERT ... SELECT`, which produces no result rows at all, so the rows a
//! statement returns cannot see it; what it does produce is one trigger
//! invocation per row, which nothing else counts.
//!
//! Nothing tested which of those two an ordinary start is. The difference is
//! invisible in behaviour — search returns the same results either way — and
//! shows up only as a slow start on exactly the mailboxes that are too big to
//! seed in anyone's test.

use chrono::{TimeZone, Utc};
use postio_index::index::ensure_schema;
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted, install};

/// Enough mail that a full re-index is unmistakable next to a no-op.
const MESSAGES: usize = 2_000;

#[test]
fn an_ordinary_start_does_not_reindex_the_mailbox() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    for nth in 0..MESSAGES {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let mut message = Message::new(account.id, mailbox, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        messages.create(&mut message).expect("create message");
    }

    install(&connection);

    // The start that builds the index. This one is allowed to be expensive:
    // it is the upgrade, and it happens once.
    let building = counted(|| ensure_schema(&connection).expect("the first start"));

    // Every start after it.
    let ordinary = counted(|| ensure_schema(&connection).expect("an ordinary start"));

    // The control, and the reason the ceiling below is known to have teeth:
    // the counter demonstrably sees a full pass over the store, because it
    // just measured one. #100 asks that each counted budget fail when the
    // invariant it guards is deliberately broken; this is that failure,
    // measured rather than asserted.
    assert!(
        building.nested > MESSAGES,
        "building the index over {MESSAGES} messages fired only {} nested \
         statements, so it did not do per-row work and the comparison below \
         means nothing",
        building.nested
    );

    assert!(
        ordinary.nested < MESSAGES,
        "an ordinary start fired {} nested statements over a {MESSAGES}-message \
         store, against {} for the start that built the index. Startup work \
         that scales with the mailbox is what §18's 500ms budget cannot \
         survive, and it is invisible in behaviour — search returns the same \
         results either way.",
        ordinary.nested,
        building.nested
    );
}
