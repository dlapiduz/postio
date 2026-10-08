//! Focus's filing rules on a corpus with known answers (spec 007 T122, US9,
//! SC-006): mail arrives through an incremental pass, the built-in
//! classifier decides by structure and headers, and the pass carries the
//! decision out. Every message's fate is written down before the pass runs.

use postio_account::backend::{AppendMessage, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{EmailAddress, Flag, FlagSet, Identity, MailboxRole, MessageId, UidValidity};
use postio_storage::repository::{
    CorrespondentRepository, FilterDecisionRepository, FilterLayer, FilterReason,
    IdentityRepository, MailboxRepository, MessageRepository, ThreadingRepository,
};
use postio_storage::test_support;
use postio_sync::{FocusFiling, Outcome, resync_mailbox_filing, sync_mailbox};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

/// What should become of one message: filed away with a reason, from a
/// layer, naming a source -- or left in the inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Inbox,
    Filtered(FilterReason, FilterLayer, &'static str),
}

/// One message of the corpus.
struct Case {
    subject: &'static str,
    from: &'static str,
    headers: &'static str,
    junk: bool,
    fate: Fate,
    /// A guard covers it: SC-006 counts these.
    guarded: bool,
}

const fn case(
    subject: &'static str,
    from: &'static str,
    headers: &'static str,
    fate: Fate,
) -> Case {
    Case {
        subject,
        from,
        headers,
        junk: false,
        fate,
        guarded: false,
    }
}

const BULK: &str = "List-Unsubscribe: <https://list.example/u/1>\r\nPrecedence: bulk\r\n";
const MACHINE: &str = "Auto-Submitted: auto-generated\r\n";

fn corpus() -> Vec<Case> {
    use Fate::{Filtered, Inbox};
    use FilterLayer::{Header, Senders, Server};
    use FilterReason::{Notification, Promotion, Receipt, Shipping, Spam};
    vec![
        // US9 scenario 1: an automated sender the user never wrote to.
        case(
            "Build 2231 passed",
            "Forge <notifications@forge.example>",
            MACHINE,
            Filtered(Notification, Senders, "Forge"),
        ),
        // US9 scenario 2: a promotion from an address the user wrote to.
        Case {
            guarded: true,
            ..case(
                "Autumn offers",
                "Shop Deals <deals@shop.example>",
                BULK,
                Inbox,
            )
        },
        // US9 scenario 3: a conversation the user took part in, whatever
        // its headers say.
        Case {
            guarded: true,
            ..case(
                "Re: your letter to the editor",
                "Ledger <news@ledger.example>",
                "In-Reply-To: <mine@example.com>\r\nReferences: <mine@example.com>\r\n\
                 List-Unsubscribe: <https://ledger.example/u>\r\nPrecedence: bulk\r\n",
                Inbox,
            )
        },
        // US9 scenario 6: uncertain, so the inbox.
        case("Lunch on Thursday?", "Tove <tove@example.org>", "", Inbox),
        case(
            "[harbour-dev] tide tables",
            "Oren <oren@example.org>",
            "List-Id: Harbour dev <harbour-dev.lists.example.org>\r\n\
             List-Unsubscribe: <mailto:leave@lists.example.org>\r\nPrecedence: list\r\n",
            Inbox,
        ),
        // The other guards: the user's own domain, and a pinned domain.
        Case {
            guarded: true,
            ..case(
                "Nightly backup finished",
                "Builds <builds@firm.example>",
                MACHINE,
                Inbox,
            )
        },
        Case {
            guarded: true,
            ..case(
                "Water level alert",
                "Alerts <alerts@pinned.example>",
                MACHINE,
                Inbox,
            )
        },
        // Each kind of evidence, each kind of reason.
        Case {
            junk: true,
            ..case(
                "You have won",
                "Winner <prize@lottery.example>",
                "",
                Filtered(Spam, Server, "Winner"),
            )
        },
        case(
            "The weekly numbers",
            "Ledger <editor@ledger.example>",
            BULK,
            Filtered(Promotion, Header, "Ledger"),
        ),
        case(
            "Your receipt",
            "Shop <receipts@shop.example>",
            "",
            Filtered(Receipt, Senders, "Shop"),
        ),
        case(
            "Your parcel is on its way",
            "Parcels <tracking@parcels.example>",
            "",
            Filtered(Shipping, Senders, "Parcels"),
        ),
        case(
            "Deploy finished",
            "robot@builds.example",
            MACHINE,
            Filtered(Notification, Header, "builds.example"),
        ),
    ]
}

fn raw(n: usize, case: &Case) -> Vec<u8> {
    format!(
        "From: {}\r\nTo: Test User <test@example.com>\r\n\
         Message-ID: <corpus-{n}@example.com>\r\n{}Subject: {}\r\n\r\nBody.\r\n",
        case.from, case.headers, case.subject
    )
    .into_bytes()
}

#[tokio::test]
async fn the_known_answer_corpus_is_filed_as_written_down_and_no_guarded_mail_is_filtered() {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(MockMessage::new(
                    b"From: Old <old@example.org>\r\nSubject: Old\r\n\r\nOld.\r\n".to_vec(),
                )),
        )
        .build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let sent = test_support::mailbox(&connection, &account, "Sent").await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");

    // What the guards stand on: the user wrote to the shop's deals address,
    // wrote the letter the Ledger replies to, and has an identity at a firm.
    for (to, message_id) in [
        ("deals@shop.example", "<to-deals@example.com>"),
        ("letters@ledger.example", "<mine@example.com>"),
    ] {
        let raw = format!(
            "From: Test User <test@example.com>\r\nTo: {to}\r\nMessage-ID: {message_id}\r\n\
             Subject: From me\r\n\r\nHello.\r\n"
        );
        let mut copy = postio_model::mime::parse(raw.as_bytes()).into_message(
            account.id,
            sent.id,
            chrono::Utc::now(),
        );
        let id = MessageRepository::new(&connection)
            .create(&mut copy)
            .await
            .expect("a sent copy");
        ThreadingRepository::new(&connection, account.id)
            .thread(&copy)
            .await
            .expect("threaded");
        CorrespondentRepository::new(&connection)
            .record_sent(account.id, &[id])
            .await
            .expect("counted");
    }
    let mut work = Identity::new(
        account.id,
        EmailAddress::new(Some("Test User"), "test@firm.example"),
    );
    IdentityRepository::new(&connection)
        .create(&mut work)
        .await
        .expect("an identity");
    // And the user pinned a whole domain.
    let config =
        postio_config::Config::from_toml_str("[focus.filter]\nnever = [\"@pinned.example\"]\n")
            .expect("a config");

    let corpus = corpus();
    for (n, case) in corpus.iter().enumerate() {
        let mut arrival = AppendMessage::new(raw(n, case));
        if case.junk {
            arrival = arrival.with_flags(FlagSet::from_iter([Flag::Junk]));
        }
        backend.append(INBOX, &arrival).await.expect("deliver");
    }

    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&FocusFiling::from_config(&config.focus)),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");
    let Outcome::Incremental { arrived, .. } = outcome else {
        panic!("expected an incremental pass, got {outcome:?}");
    };
    assert_eq!(arrived.len(), corpus.len(), "every message arrived");

    let mailboxes = MailboxRepository::new(&connection);
    let decisions = FilterDecisionRepository::new(&connection);
    let mut wrong = Vec::new();
    for case in &corpus {
        let id: i64 = postio_storage::sql::first(
            &connection,
            "SELECT id FROM messages WHERE subject = ?1",
            [case.subject],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")
        .unwrap_or_else(|| panic!("`{}` was not stored", case.subject));
        let message = MessageRepository::new(&connection)
            .get(MessageId::new(id))
            .await
            .expect("a read")
            .expect("the message");
        let role = mailboxes
            .get(message.mailbox_id)
            .await
            .expect("a read")
            .expect("its folder")
            .role;
        let decision = decisions.get(message.id).await.expect("a read");
        let fate = match (role, &decision) {
            (MailboxRole::Inbox, None) => Some(Fate::Inbox),
            (MailboxRole::Archive, Some(decision)) => {
                let source = decision.source.as_deref().unwrap_or("");
                corpus_source(&corpus, source)
                    .map(|source| Fate::Filtered(decision.reason, decision.layer, source))
            }
            _ => None,
        };
        if fate != Some(case.fate) {
            wrong.push(format!(
                "{:<32} expected {:?}, found {role:?} with {decision:?}",
                case.subject, case.fate
            ));
        }
        if case.guarded {
            // SC-006: zero messages a guard covers are filtered.
            assert_eq!(
                (role, decision.is_none()),
                (MailboxRole::Inbox, true),
                "`{}` is guarded and was filtered",
                case.subject
            );
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    assert_eq!(archive.role, MailboxRole::Archive);
}

/// The corpus's own spelling of `source`, so a `Fate` can hold a
/// `&'static str`.
fn corpus_source(corpus: &[Case], source: &str) -> Option<&'static str> {
    corpus.iter().find_map(|case| match case.fate {
        Fate::Filtered(_, _, named) if named == source => Some(named),
        _ => None,
    })
}

// --- Digests: holding at filing (T133, US10) ------------------------------------------

/// An invitation from the Ledger, in the Ledger's bulk-mail clothes, with
/// the calendar part a server's `BODYSTRUCTURE` would report.
fn invitation() -> MockMessage {
    use postio_account::backend::{BodyStructure, PartNode};
    MockMessage::new(
        b"From: Ledger <news@ledger.example>\r\nTo: Test User <test@example.com>\r\n\
          Message-ID: <reader-evening@ledger.example>\r\nSubject: Readers' evening\r\n\
          List-Unsubscribe: <https://ledger.example/u>\r\nPrecedence: bulk\r\n\
          Content-Type: multipart/alternative; boundary=b\r\n\r\n\
          --b\r\nContent-Type: text/plain\r\n\r\nJoin us.\r\n\
          --b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n\
          --b--\r\n"
            .to_vec(),
    )
    .with_structure(BodyStructure::from_parts(
        "multipart/alternative",
        [
            PartNode::new("1", "text/plain", 9).with_charset("utf-8"),
            PartNode::new("2", "text/calendar", 30),
        ],
    ))
}

#[tokio::test]
async fn a_digest_rule_holds_its_sender_s_mail_and_never_an_invitation() {
    // US10 scenarios 1 and 2, at filing: the rule's sender's mail skips
    // Focus's inbox -- held, not filtered, and still filed in the inbox,
    // where the other apps see it (FR-121) -- and the same sender's
    // invitation comes straight to the inbox.
    const STAGING: &str = "Staging";
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(MockMessage::new(
                    b"From: Old <old@example.org>\r\nSubject: Old\r\n\r\nOld.\r\n".to_vec(),
                )),
        )
        .mailbox(
            MockMailbox::new(STAGING)
                .uid_validity(UidValidity::new(VALIDITY + 1))
                .message(invitation()),
        )
        .build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    test_support::mailbox(&connection, &account, "Archive").await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");
    let threads = postio_storage::repository::ThreadRepository::new(&connection);
    let inboxes = [(account.id, inbox.id)];
    let before = threads.focus_count(&inboxes).await.expect("a count");

    // Wednesday: the newsletter, a letter, and the invitation.
    backend
        .append(
            INBOX,
            &AppendMessage::new(raw(
                0,
                &case(
                    "The weekly numbers",
                    "Ledger <news@ledger.example>",
                    BULK,
                    Fate::Inbox,
                ),
            )),
        )
        .await
        .expect("deliver");
    backend
        .append(
            INBOX,
            &AppendMessage::new(raw(
                1,
                &case(
                    "Lunch on Thursday?",
                    "Tove <tove@example.org>",
                    "",
                    Fate::Inbox,
                ),
            )),
        )
        .await
        .expect("deliver");
    backend
        .copy_messages(
            STAGING,
            &[postio_model::RemoteId::new(format!("{}:1", VALIDITY + 1))],
            INBOX,
        )
        .await
        .expect("the invitation arrives");

    let config = postio_config::Config::from_toml_str(
        r#"[[focus.digests]]
name    = "Newsletters"
match   = ["from:news@ledger.example"]
cadence = "weekly"
day     = "sunday"
at      = "09:00"
"#,
    )
    .expect("a config");
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&FocusFiling::from_config(&config.focus)),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");
    assert!(matches!(outcome, Outcome::Incremental { ref arrived, .. } if arrived.len() == 3));

    let fate = |subject: &'static str| {
        let connection = &connection;
        async move {
            let (id, mailbox): (i64, i64) = postio_storage::sql::first(
                connection,
                "SELECT id, mailbox_id FROM messages WHERE subject = ?1",
                [subject],
                |row| {
                    Ok((
                        postio_storage::sql::RowExt::col(row, 0)?,
                        postio_storage::sql::RowExt::col(row, 1)?,
                    ))
                },
            )
            .await
            .expect("a read")
            .unwrap_or_else(|| panic!("`{subject}` was not stored"));
            let held: Option<String> = postio_storage::sql::first(
                connection,
                "SELECT rule FROM digest_holds WHERE message_id = ?1",
                [id],
                |row| postio_storage::sql::RowExt::col(row, 0),
            )
            .await
            .expect("a read");
            let decided = FilterDecisionRepository::new(connection)
                .get(MessageId::new(id))
                .await
                .expect("a read")
                .is_some();
            (postio_model::MailboxId::new(mailbox), held, decided)
        }
    };

    assert_eq!(
        fate("The weekly numbers").await,
        (inbox.id, Some("Newsletters".to_owned()), false),
        "held under its rule, still filed in the inbox, and not filtered"
    );
    assert_eq!(
        fate("Readers' evening").await,
        (inbox.id, None, false),
        "the rule's sender's invitation is neither held nor filtered"
    );
    assert_eq!(fate("Lunch on Thursday?").await, (inbox.id, None, false));
    assert_eq!(
        threads.focus_count(&inboxes).await.expect("a count"),
        before + 2,
        "Focus's inbox gained the letter and the invitation, not the held newsletter"
    );
}
