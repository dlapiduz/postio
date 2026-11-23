use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, Generation, Mailbox, MessageId, Uid, UidValidity};
use postio_storage::BlobStore;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support::{self, counting};
use postio_sync::backfill::{Backfill, BackfillPolicy, BodyRequest, Want, fetch_body, seed};
use postio_sync::{resync_mailbox, sync_mailbox};

const VALIDITY: u32 = 1_708_000_000;

const ALLOWED: &[(&str, &str)] = &[(
    "SELECT id, subject FROM messages",
    "the orphan rethread: once per mailbox, after its *first* sync only, \
         to join replies filed before the message they answer",
)];

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn mail(mailbox: &str, n: u32) -> Vec<u8> {
    let references = if n % 5 != 1 {
        format!(
            "In-Reply-To: <{mailbox}-{}@example.com>\r\nReferences: <{mailbox}-{}@example.com>\r\n",
            n - 1,
            n - 1
        )
    } else {
        String::new()
    };
    format!(
        "From: Person {s} <sender{s}@example.com>\r\n\
         To: Ada Lovelace <ada@example.com>, Grace <grace{s}@example.org>\r\n\
         Cc: Team <team@example.net>\r\n\
         Subject: {subject}\r\n\
         Message-ID: <{mailbox}-{n}@example.com>\r\n\
         {references}\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         The body of {mailbox} note {n}.\r\n",
        s = n % 37,
        subject = if n % 5 == 1 {
            format!("Topic {n}")
        } else {
            format!("Re: Topic {}", n - n % 5 + 1)
        },
    )
    .into_bytes()
}

fn mailbox_of(path: &str, count: u32) -> MockMailbox {
    let mut mailbox = MockMailbox::new(path).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        let mut flags = FlagSet::new();
        if n % 3 == 0 {
            flags = FlagSet::from_iter([Flag::Seen]);
        }
        mailbox = mailbox.message(
            MockMessage::new(mail(path, n))
                .with_flags(flags)
                .with_internal_date(at(n as i64)),
        );
    }
    mailbox
}

#[tokio::test]
async fn a_first_sync_issues_no_full_scan_of_a_table_that_grows() {
    let backend = MockBackend::builder()
        .mailbox(mailbox_of("INBOX", 400))
        .mailbox(mailbox_of("Archive", 300))
        .mailbox(mailbox_of("Sent", 100))
        .build();
    backend.connect().await.expect("connect");

    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let mut mailboxes: Vec<Mailbox> = Vec::new();
    for path in ["INBOX", "Archive", "Sent"] {
        mailboxes.push(test_support::mailbox(&connection, &account, path).await);
    }
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("blob store");

    counting::record();
    counting::reset();

    for mailbox in &mailboxes {
        sync_mailbox(&connection, &backend, mailbox, &CancelToken::new(), |_| {})
            .await
            .expect("first sync");
    }

    let messages = MessageRepository::new(&connection);
    let inbox = &mailboxes[0];
    for uid in 300..=400u32 {
        let message = messages
            .by_uid(inbox.id, Generation::new(VALIDITY), Uid::new(uid))
            .await
            .expect("look up")
            .expect("stored");
        let request = BodyRequest {
            message: MessageId::new(message.id.get()),
            mailbox: inbox.id,
            path: inbox.path.clone(),
            uid: Uid::new(uid),
            remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:{uid}")),
            size: 1_024,
            text_size: 1_024,
            rank: postio_sync::order::sync_priority(inbox.role),
            received_at: at(uid as i64),
            want: Want::Text,
        };
        fetch_body(
            &connection,
            &blobs,
            &backend,
            &request,
            BackfillPolicy::default().max_inline_bytes,
            None,
            &CancelToken::new(),
        )
        .await
        .expect("fetch body");
    }

    let mut backfill = Backfill::new(BackfillPolicy::default());
    for mailbox in &mailboxes {
        seed(&connection, &mut backfill, mailbox.id, 50)
            .await
            .expect("seed the backlog");
    }

    for mailbox in &mailboxes {
        resync_mailbox(&connection, &backend, mailbox, &CancelToken::new(), |_| {})
            .await
            .expect("resync");
    }

    let total = counting::here();
    let issued = counting::recorded();
    eprintln!("first sync + bodies + resync: {total:?}");

    let mut offenders = Vec::new();
    for (sql, runs) in &issued {
        if std::env::var_os("AUDIT_ALL").is_some() {
            let steps = counting::plan_steps(&connection, sql)
                .await
                .unwrap_or_default();
            eprintln!("ALL x{runs}: {}\n    {}", one_line(sql), steps.join(" | "));
        }
        let large = counting::unbounded(&connection, sql, counting::GROWING_TABLES).await;
        if large.is_empty() {
            continue;
        }
        let allowed = ALLOWED.iter().any(|(fragment, _)| sql.contains(fragment));
        eprintln!(
            "{} x{runs}: {large:?}\n    {}",
            if allowed { "ALLOWED" } else { "SCAN" },
            one_line(sql)
        );
        if !allowed {
            offenders.push(one_line(sql));
        }
    }
    eprintln!("{} distinct statements issued", issued.len());
    // Production runs no ANALYZE; with statistics the same statements plan worse
    // (docs/notes/2026-09-30-analyze-makes-the-hot-plans-worse.md).
    let stats: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM sqlite_stat1", ())
            .await
            .unwrap_or(0);
    assert_eq!(
        stats, 0,
        "the store has planner statistics, so this audit is not auditing production's plans"
    );
    assert!(
        offenders.is_empty(),
        "a first sync ran {} statement(s) the planner answers with a full scan of a table \
         that grows with the mailbox: {offenders:#?}",
        offenders.len()
    );
}

fn one_line(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}
