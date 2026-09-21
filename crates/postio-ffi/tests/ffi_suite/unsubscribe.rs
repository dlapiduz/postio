//! One-click unsubscribe across the boundary (#971), and the decode caveat
//! that sits beside it in the same notice strip (#901).
//!
//! `PRODUCT.md` lists one-click unsubscribe among Postio's privacy features
//! and CLAUDE.md states the rule: **only on deliberate activation**. macOS
//! had no form of it at all — the boundary did not carry a list identifier,
//! so the reader could not have drawn a banner even if somebody wrote one.
//!
//! The assertions are in two halves, and the second half is the point:
//!
//! 1. **What a message offers** — the `List-Id`, the sender's domain when
//!    there is none, and nothing whatsoever for mail on its way out (#1525).
//! 2. **What cannot happen by accident** — reading a message, rendering its
//!    document and asking what it offers, over and over, records nothing;
//!    and an activation asked for on a message that offers nothing is
//!    refused rather than logged. Between them those say the activation is
//!    reachable only from a frontend that meant it, which is the whole
//!    privacy claim.

use chrono::Utc;
use postio_ffi::{RemoteImagesFfi, Session, SessionOptions};
use postio_model::ids::AccountId;
use postio_model::{Account, DraftState, EmailAddress, Message};
use postio_storage::Store;
use postio_storage::repository::{
    AccountRepository, MessageRepository, StoredBody, UnsubscribeRepository,
};
use postio_storage::test_support;

/// One message to seed. Every field is something the banner's rules read.
#[derive(Default)]
struct Seed {
    /// Which of the store's two accounts owns this message — `0` or `1`.
    ///
    /// The activation log is stamped with an account, and a store holding
    /// only one makes the wrong stamp indistinguishable from the right one:
    /// "the message's account" and "the first account there is" are the same
    /// value, so an assertion on it cannot fail. Anything asserting on the
    /// stamp puts its message under `1`.
    account: usize,
    /// `From`, which is where the domain fallback comes from.
    sender: &'static str,
    /// `List-Id`, when the sender set one.
    list_id: Option<&'static str>,
    /// Whether the stored body says parts of it did not decode.
    encoding_problems: bool,
    /// The `send_state` column, for the Outbox cases.
    send_state: Option<DraftState>,
}

/// A session over a store holding one message per seed, the store itself,
/// and both of its accounts.
///
/// The store is handed back because the activation log is *stamped with an
/// account* and the boundary's own record deliberately does not carry one —
/// the Privacy pane draws every account's activations in one list, the same
/// shape the remote-image grants have. Reading the log directly is how that
/// stamp gets asserted at all.
///
/// # Why there are always two accounts
///
/// Both of the things this file is here to pin are invisible in a store with
/// one. `activateUnsubscribe` has to stamp the row with *the message's*
/// account, and `unsubscribeActivations` has to read *every* account's log
/// and interleave them — and a single-account fixture makes "the message's
/// account", "the first account" and "the only account" one value, so an
/// implementation that reached for any of them would pass. The second
/// account exists whether or not a given test names it, which also means
/// every read in here goes through the merge rather than around it.
async fn a_store_with(
    seeds: &[Seed],
) -> (std::sync::Arc<Session>, Vec<i64>, Store, Vec<AccountId>) {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let (ids, accounts) = {
        let connection = database.connect().await.expect("a connection");
        let (first, first_inbox) = test_support::account_with_inbox(&connection).await;
        let mut second = Account::new(
            "Second",
            EmailAddress::new(None::<String>, "grace@example.org"),
        );
        AccountRepository::new(&connection)
            .create(&mut second)
            .await
            .expect("a second account");
        let second_inbox = test_support::mailbox(&connection, &second, "INBOX")
            .await
            .id;
        let homes = [(first.id, first_inbox), (second.id, second_inbox)];

        let repository = MessageRepository::new(&connection);
        let mut ids = Vec::new();
        for seed in seeds {
            let (account_id, inbox) = homes[seed.account];
            let mut message = Message::new(account_id, inbox, Utc::now());
            message.from = vec![EmailAddress::new(Some("Weekly Digest"), seed.sender)];
            message.list_id = seed.list_id.map(str::to_owned);
            let id = repository.create(&mut message).await.expect("a message");
            repository
                .set_body(
                    id,
                    &StoredBody {
                        text: Some("Nothing much happened".to_owned()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: seed.encoding_problems,
                    },
                    postio_model::message::BodyState::Full,
                )
                .await
                .expect("the body is stored");
            // Written here rather than through a draft, because the only
            // writer of this column is private to `postio-storage` and lives
            // behind saving a draft — three tables and a queue row to reach
            // one flag the reader reads back out. What is under test is what
            // the boundary does with the column, not how it came to hold a
            // value.
            if let Some(state) = seed.send_state {
                connection
                    .execute(
                        "UPDATE messages SET send_state = ?2 WHERE id = ?1",
                        (id.get(), state.as_str()),
                    )
                    .await
                    .expect("the send state is written");
            }
            ids.push(id.get());
        }
        (ids, vec![first.id, second.id])
    };

    let session = Session::open(
        SessionOptions::in_memory_with(database.clone()).with_blobs_for_test(blobs, scratch),
    )
    .expect("a session over the store");
    (session, ids, database, accounts)
}

/// Everything the log holds for `account`, straight from storage.
async fn logged(database: &Store, account: AccountId) -> Vec<postio_model::UnsubscribeActivation> {
    let connection = database.connect().await.expect("a connection");
    UnsubscribeRepository::new(&connection)
        .for_account(account)
        .await
        .expect("the activation log reads")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_from_a_list_names_the_list_it_says_it_is_from() {
    let (session, ids, _database, _accounts) = a_store_with(&[Seed {
        sender: "weekly@mail.example.org",
        list_id: Some("news.example.org"),
        ..Seed::default()
    }])
    .await;

    let offer = session
        .unsubscribe_offer(ids[0])
        .await
        .expect("a message with a List-Id is offered an unsubscribe");
    assert_eq!(offer.list_identifier, "news.example.org");
    // Against the shared sentence rather than a copy typed here: a second
    // copy of the words is how the two readers come to say different things
    // about the same message.
    assert_eq!(
        offer.summary,
        postio_ui::unsubscribe::summary("news.example.org")
    );
    assert_eq!(offer.action, postio_ui::unsubscribe::ACTION);
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_mail_with_no_list_header_names_the_senders_domain() {
    // Almost no commercial mail sets `List-Id`. A banner that only appeared
    // for the mail that did would be a privacy feature nobody ever saw.
    let (session, ids, _database, _accounts) = a_store_with(&[Seed {
        sender: "weekly@news.example.org",
        ..Seed::default()
    }])
    .await;

    let offer = session
        .unsubscribe_offer(ids[0])
        .await
        .expect("the sender's domain is the fallback");
    assert_eq!(offer.list_identifier, "news.example.org");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_on_its_way_out_is_offered_nothing() {
    // #1525: the fallback is the sender's domain, and the sender of an
    // outgoing message is the user — so a banner over the Outbox offers to
    // unsubscribe somebody from their own account.
    let (session, ids, _database, _accounts) = a_store_with(&[
        Seed {
            sender: "ada@example.com",
            send_state: Some(DraftState::Queued),
            ..Seed::default()
        },
        Seed {
            sender: "ada@example.com",
            send_state: Some(DraftState::Sending),
            ..Seed::default()
        },
        Seed {
            sender: "ada@example.com",
            send_state: Some(DraftState::Failed),
            ..Seed::default()
        },
    ])
    .await;

    for id in ids {
        assert_eq!(
            session.unsubscribe_offer(id).await,
            None,
            "a message on its way out was offered an unsubscribe from the user's own domain"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_the_store_does_not_hold_offers_nothing() {
    let (session, _ids, _database, _accounts) = a_store_with(&[Seed {
        sender: "weekly@news.example.org",
        ..Seed::default()
    }])
    .await;
    assert_eq!(session.unsubscribe_offer(9_999).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_a_message_never_records_an_activation() {
    // The privacy claim, asserted rather than asserted *about*. Everything a
    // reader does when a message opens — render the document, ask what it
    // offers, render it again because the reader turned a page — happens
    // here, repeatedly, and the log stays empty. Unsubscribing is something
    // a person does, not something a message causes.
    let (session, ids, database, accounts) = a_store_with(&[Seed {
        sender: "weekly@news.example.org",
        list_id: Some("news.example.org"),
        ..Seed::default()
    }])
    .await;

    for _ in 0..3 {
        let offer = session.unsubscribe_offer(ids[0]).await;
        assert!(offer.is_some(), "the offer is what is being drawn");
        session
            .reader_document(ids[0], RemoteImagesFfi::Blocked, false)
            .await;
        session
            .reader_document(ids[0], RemoteImagesFfi::Allowed, true)
            .await;
        session.decode_caveat(ids[0]).await;
        assert!(session.unsubscribe_activations().await.is_empty());
    }

    for account in &accounts {
        assert!(
            logged(&database, *account).await.is_empty(),
            "rendering a message recorded an unsubscribe activation"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn activating_records_one_activation_against_the_messages_account() {
    // The message is deliberately the *second* account's. The log is scoped
    // per account, so a row stamped with the wrong one is a row the owner
    // never sees in their pane and somebody else does — and with one account
    // in the store the wrong stamp and the right one are the same number.
    let (session, ids, database, accounts) = a_store_with(&[Seed {
        account: 1,
        sender: "weekly@mail.example.org",
        list_id: Some("news.example.org"),
        ..Seed::default()
    }])
    .await;

    assert_eq!(
        session.activate_unsubscribe(ids[0]).await,
        None,
        "the activation reported a failure"
    );

    let activations = logged(&database, accounts[1]).await;
    assert_eq!(activations.len(), 1, "one press, one row");
    assert_eq!(activations[0].list_identifier, "news.example.org");
    assert_eq!(
        activations[0].account_id, accounts[1],
        "the row has to name the account whose message it was, not whichever \
         account the store happens to list first"
    );
    assert!(
        logged(&database, accounts[0]).await.is_empty(),
        "an account that owns none of this mail was given an activation"
    );

    // And it is what the Privacy pane will draw — which reads every account,
    // so the row shows up there whichever one it belongs to.
    let listed = session.unsubscribe_activations().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].list_identifier, "news.example.org");
    assert_eq!(
        listed[0].when,
        postio_ui::unsubscribe::activated_on(activations[0].activated_at)
    );
    assert_eq!(
        listed[0].label,
        postio_ui::unsubscribe::activation_label("news.example.org", activations[0].activated_at)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn activating_a_message_that_offers_nothing_is_refused() {
    // The second half of "only on deliberate activation": the action does
    // not trust the caller's word that a banner was on screen. It re-derives
    // the offer, and refuses when there is none — so a frontend bug cannot
    // record the user leaving their own account's domain.
    let (session, ids, database, accounts) = a_store_with(&[Seed {
        sender: "ada@example.com",
        send_state: Some(DraftState::Queued),
        ..Seed::default()
    }])
    .await;

    let complaint = session
        .activate_unsubscribe(ids[0])
        .await
        .expect("a message that offers nothing must refuse, not log");
    assert!(
        complaint.contains("list"),
        "the refusal should say what was missing: {complaint}"
    );
    for account in &accounts {
        assert!(logged(&database, *account).await.is_empty());
    }
    assert!(session.unsubscribe_activations().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_privacy_pane_lists_activations_newest_first() {
    let (session, ids, _database, _accounts) = a_store_with(&[
        Seed {
            sender: "weekly@old-news.example.org",
            ..Seed::default()
        },
        Seed {
            sender: "daily@new-news.example.org",
            ..Seed::default()
        },
    ])
    .await;

    assert_eq!(session.activate_unsubscribe(ids[0]).await, None);
    assert_eq!(session.activate_unsubscribe(ids[1]).await, None);

    let listed = session.unsubscribe_activations().await;
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[0].list_identifier, "new-news.example.org",
        "the pane reads newest first, the way the log itself is ordered"
    );
    assert_eq!(listed[1].list_identifier, "old-news.example.org");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_lost_a_part_says_so_and_a_clean_one_stays_quiet() {
    // #901's flag, which was computed and read by nothing on this platform:
    // the macOS reader renders a document and never said the words in it may
    // not be the words that were sent.
    let (session, ids, _database, _accounts) = a_store_with(&[
        Seed {
            sender: "weekly@news.example.org",
            encoding_problems: true,
            ..Seed::default()
        },
        Seed {
            sender: "weekly@news.example.org",
            ..Seed::default()
        },
    ])
    .await;

    assert_eq!(
        session.decode_caveat(ids[0]).await.as_deref(),
        Some(postio_ui::reader::document::DECODE_CAVEAT),
        "a body that did not fully decode has to say so"
    );
    assert_eq!(
        session.decode_caveat(ids[1]).await,
        None,
        "a notice with nothing to report teaches people to dismiss the one that matters"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_privacy_pane_interleaves_every_accounts_log() {
    // What `unsubscribeActivations` claims, asserted: *every* account, in one
    // list, in time order. The rows are written straight into the log rather
    // than by pressing the button, because pressing it stamps `Utc::now()`
    // and two presses in a test land in the same millisecond — which would
    // make this assert on a tie-break instead of on the merge. Four rows at
    // four known instants, alternating between the two accounts, is the only
    // arrangement where reading one account, or reading both and not
    // interleaving them, both come out wrong.
    let (session, _ids, database, accounts) = a_store_with(&[]).await;
    let connection = database.connect().await.expect("a connection");
    let log = UnsubscribeRepository::new(&connection);
    let at = |seconds: i64| chrono::DateTime::from_timestamp(seconds, 0).expect("an instant");
    for (account, list, when) in [
        (accounts[0], "first.early.example.org", at(1_000)),
        (accounts[1], "second.middling.example.org", at(2_000)),
        (accounts[0], "first.later.example.org", at(3_000)),
        (accounts[1], "second.latest.example.org", at(4_000)),
    ] {
        let mut activation = postio_model::UnsubscribeActivation::new(account, list, when);
        log.record(&mut activation).await.expect("a logged row");
    }
    drop(connection);

    let listed = session.unsubscribe_activations().await;
    let names: Vec<&str> = listed
        .iter()
        .map(|row| row.list_identifier.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "second.latest.example.org",
            "first.later.example.org",
            "second.middling.example.org",
            "first.early.example.org",
        ],
        "the pane is one list over every account, newest first"
    );
}
