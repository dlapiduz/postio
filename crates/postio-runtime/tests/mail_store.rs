//! The store seam, without a database.
//!
//! [`MailStore`] is what a frontend holds, and the whole point of it being a
//! trait is that neither side needs the other's dependencies: this file
//! answers from a table rather than a database, which is what any frontend
//! or test can do.
//!
//! Nothing here touches the network.

use std::sync::Mutex;

use chrono::{TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_runtime::store::{
    ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read, StoreError,
};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;

/// A store that makes a page up on demand.
///
/// Holds a count rather than a mailbox, on purpose: a fake that kept every row
/// in memory would be testing the opposite of what the seam is for.
struct Fake {
    total: u32,
    asked: Mutex<Vec<PageRequest>>,
}

impl Fake {
    fn row(&self, position: u32) -> MessageSummary {
        MessageSummary {
            id: MessageId::new(position as i64 + 1),
            thread: Some(ThreadId::new(position as i64 + 1)),
            from: None,
            subject: Some(format!("message {position}")),
            preview: None,
            received_at: Utc
                .timestamp_opt(1_700_000_000 - position as i64, 0)
                .unwrap(),
            seen: position.is_multiple_of(2),
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
        }
    }
}

impl MailStore for Fake {
    fn message_page(&self, request: PageRequest) -> Read<'_, MessagePage> {
        self.asked.lock().expect("not poisoned").push(request);
        let end = (request.offset + request.limit).min(self.total);
        let rows = (request.offset..end).map(|at| self.row(at)).collect();
        let total = self.total;
        Box::pin(async move { Ok(MessagePage { total, rows }) })
    }

    fn message_count(&self, _scope: ListScope) -> Read<'_, u32> {
        let total = self.total;
        Box::pin(async move { Ok(total) })
    }

    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
        // Keyed off the position the id encodes, so the answer is in the
        // order asked rather than the order the fake happens to iterate.
        let rows = ids
            .iter()
            .filter_map(|id| u32::try_from(id.get() - 1).ok())
            .filter(|position| *position < self.total)
            .map(|position| self.row(position))
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

#[tokio::test]
async fn a_store_is_usable_behind_a_trait_object() {
    // How a frontend holds it: one store, chosen once, behind a `dyn`. If the
    // trait ever stops being object-safe this is what says so.
    let store: Box<dyn MailStore> = Box::new(Fake {
        total: 100_000,
        asked: Mutex::new(Vec::new()),
    });

    let page = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(MailboxId::new(INBOX)),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the fake answers");

    assert_eq!(page.total, 100_000, "the count comes with the page");
    assert_eq!(page.rows.len(), 50, "and only a page of rows with it");
    assert_eq!(
        store
            .mailboxes(AccountId::new(ACCOUNT))
            .await
            .expect("folders")
            .len(),
        1
    );
}

#[tokio::test]
async fn asking_for_a_window_asks_for_a_window() {
    // spec.md §18: a mailbox is never loaded into memory. The seam has to be
    // able to express "these fifty, starting here" or the windowed list model
    // above it has nothing to window over.
    let fake = Fake {
        total: 100_000,
        asked: Mutex::new(Vec::new()),
    };
    let scope = ListScope::Mailbox(MailboxId::new(INBOX));
    for offset in [0, 50, 100] {
        fake.message_page(PageRequest {
            scope,
            offset,
            limit: 50,
        })
        .await
        .expect("the fake answers");
    }

    let asked = fake.asked.lock().expect("not poisoned").clone();
    assert_eq!(
        asked
            .iter()
            .map(|request| request.offset)
            .collect::<Vec<_>>(),
        [0, 50, 100]
    );
    assert!(
        asked.iter().all(|request| request.limit == 50),
        "nothing asked for more than a page"
    );
}

#[tokio::test]
async fn a_read_that_fails_carries_a_sentence_rather_than_a_sql_error() {
    struct Broken;
    impl MailStore for Broken {
        fn message_page(&self, _: PageRequest) -> Read<'_, MessagePage> {
            Box::pin(async { Err(StoreError::new("the database is locked")) })
        }
        fn message_count(&self, _: ListScope) -> Read<'_, u32> {
            Box::pin(async { Err(StoreError::new("the database is locked")) })
        }
        fn message_rows(&self, _: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
            Box::pin(async { Err(StoreError::new("the database is locked")) })
        }
        fn mailboxes(&self, _: AccountId) -> Read<'_, Vec<Mailbox>> {
            Box::pin(async { Err(StoreError::new("the database is locked")) })
        }
    }

    let error = Broken
        .message_count(ListScope::Account(AccountId::new(ACCOUNT)))
        .await
        .expect_err("this one fails");
    assert_eq!(error.message(), "the database is locked");
    assert_eq!(
        error.to_string(),
        "the database is locked",
        "what the user is shown is the whole of it"
    );
}
