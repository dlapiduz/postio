//! One-click unsubscribe across the boundary (#971), and the decode caveat beside it (#901).
//! Reading, rendering and asking what a message offers record nothing, and an activation on a
//! message that offers nothing is refused.

use chrono::Utc;
use postio_ffi::{RemoteImagesFfi, Session, SessionOptions};
use postio_model::ids::AccountId;
use postio_model::{Account, DraftState, EmailAddress, Message};
use postio_storage::Store;
use postio_storage::repository::{
    AccountRepository, MessageRepository, StoredBody, UnsubscribeRepository,
};
use postio_storage::test_support;

#[derive(Default)]
struct Seed {
    /// The activation log is stamped with an account; with one account a wrong stamp equals the
    /// right one, so anything asserting on the stamp puts its message under `1`.
    account: usize,
    sender: &'static str,
    list_id: Option<&'static str>,
    encoding_problems: bool,
    send_state: Option<DraftState>,
    list_unsubscribe: bool,
}

/// Always two accounts: `activateUnsubscribe` must stamp the message's account and
/// `unsubscribeActivations` must merge every account's log,
/// which a one-account fixture cannot tell apart. The store is returned so the stamp can be read
/// from the log directly.
async fn a_store_with(
    seeds: &[Seed],
) -> (std::sync::Arc<Session>, Vec<i64>, Store, Vec<AccountId>) {
    a_store_configured("", seeds).await
}

async fn a_store_configured(
    config: &str,
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
            message.promoted = Some(postio_model::promoted::PromotedHeaders {
                unsubscribe_offered: seed.list_unsubscribe,
                automation: 0,
            });
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
            // Written directly: the only writer of this column is private to `postio-storage`,
            // behind saving a draft.
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
        SessionOptions::in_memory_with(database.clone())
            .with_blobs_for_test(blobs, scratch)
            .with_config_for_test(config),
    )
    .expect("a session over the store");
    (session, ids, database, accounts)
}

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
    // Against the shared sentence, not a copy typed here.
    assert_eq!(
        offer.summary,
        postio_ui::unsubscribe::summary("news.example.org")
    );
    assert_eq!(offer.action, postio_ui::unsubscribe::ACTION);
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_mail_with_no_list_header_names_the_senders_domain() {
    // Almost no commercial mail sets `List-Id`; a banner only for mail that did would never be
    // seen.
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
    // #1525: the fallback is the sender's domain, which for outgoing mail is the user's own.
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
    // Reading, rendering and asking what a message offers, repeatedly, leaves the log empty:
    // unsubscribing is something a person does.
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
            .reader_document(ids[0], RemoteImagesFfi::Blocked, true)
            .await;
        session
            .reader_document(ids[0], RemoteImagesFfi::Allowed, false)
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
    // The message is the second account's: with one account a wrong stamp and the right one are the
    // same number.
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
    // The action re-derives the offer instead of trusting that a banner was shown, so a frontend
    // bug cannot record the user leaving their own domain.
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
    // #901's flag was computed and read by nothing on this platform.
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
    // Rows are written straight into the log at four known instants, alternating accounts: pressing
    // the button stamps `Utc::now()` and two presses tie.
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

fn focus_banner(session: &Session, message: i64) -> Option<postio_ffi::UnsubscribeOfferFfi> {
    session
        .focus_reader_document(message, RemoteImagesFfi::Blocked, None, 1440)
        .unsubscribe
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_draws_the_banner_over_list_mail_with_the_shared_words() {
    let (session, ids, _database, _accounts) = a_store_with(&[
        Seed {
            sender: "weekly@mail.example.org",
            list_id: Some("news.example.org"),
            ..Seed::default()
        },
        Seed {
            sender: "deals@shop.example.org",
            list_unsubscribe: true,
            ..Seed::default()
        },
    ])
    .await;

    let listed = focus_banner(&session, ids[0]).expect("a List-Id is list mail");
    assert_eq!(listed.list_identifier, "news.example.org");
    assert_eq!(
        listed.summary,
        postio_ui::unsubscribe::summary("news.example.org")
    );
    assert_eq!(listed.action, postio_ui::unsubscribe::ACTION);

    let bulk = focus_banner(&session, ids[1]).expect("a List-Unsubscribe is list mail too");
    assert_eq!(bulk.list_identifier, "shop.example.org");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_draws_no_banner_over_personal_mail_or_the_outbox() {
    let (session, ids, _database, _accounts) = a_store_with(&[
        Seed {
            sender: "grace@example.org",
            ..Seed::default()
        },
        Seed {
            sender: "ada@example.com",
            list_id: Some("news.example.org"),
            send_state: Some(DraftState::Queued),
            ..Seed::default()
        },
        Seed {
            sender: "weekly@mail.example.org",
            list_id: Some("news.example.org"),
            ..Seed::default()
        },
    ])
    .await;

    assert_eq!(focus_banner(&session, ids[0]), None, "personal mail");
    assert_eq!(focus_banner(&session, ids[1]), None, "the Outbox");
    assert!(focus_banner(&session, ids[2]).is_some());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_says_when_a_body_lost_a_part() {
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

    let lost = session.focus_reader_document(ids[0], RemoteImagesFfi::Blocked, None, 1440);
    assert!(lost.caveat.is_some(), "a body that lost a part says so");
    let whole = session.focus_reader_document(ids[1], RemoteImagesFfi::Blocked, None, 1440);
    assert_eq!(whole.caveat, None);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_offer_after_leaving_counts_the_lists_mail_and_only_that() {
    let (session, ids, _database, _) = a_store_with(&[
        Seed {
            sender: "news@mail.example.org",
            list_id: Some("weekly.example.org"),
            list_unsubscribe: true,
            ..Seed::default()
        },
        Seed {
            sender: "news@mail.example.org",
            list_id: Some("weekly.example.org"),
            ..Seed::default()
        },
        Seed {
            sender: "other@mail.example.org",
            list_id: Some("other.example.org"),
            ..Seed::default()
        },
        Seed {
            sender: "ada@example.net",
            ..Seed::default()
        },
    ])
    .await;

    let offer = session
        .list_mail_offer(ids[0])
        .await
        .expect("two messages from the list are still here");
    assert_eq!(offer.count, 2);
    assert_eq!(offer.button, "Archive 2", "archive is the default action");
    assert!(!offer.delete);
    assert!(
        offer.summary.contains("weekly.example.org"),
        "{}",
        offer.summary
    );

    let offer = session
        .list_mail_offer(ids[3])
        .await
        .expect("their own mail");
    assert_eq!(offer.count, 1);
    assert!(
        offer.summary.contains("ada@example.net"),
        "{}",
        offer.summary
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_notice_offers_the_configured_action() {
    let seeds = [
        Seed {
            sender: "news@mail.example.org",
            list_id: Some("weekly.example.org"),
            ..Seed::default()
        },
        Seed {
            sender: "news@mail.example.org",
            list_id: Some("weekly.example.org"),
            ..Seed::default()
        },
    ];
    let (session, ids, _database, _) =
        a_store_configured("[focus]\nafter_unsubscribe = \"delete\"\n", &seeds).await;
    let offer = session
        .list_mail_offer(ids[0])
        .await
        .expect("two messages are still here");
    assert_eq!(offer.button, "Delete 2");
    assert!(offer.delete, "the boundary says which verb the button is");
    assert!(
        offer
            .summary
            .starts_with("Unsubscribed from weekly.example.org")
    );
}
