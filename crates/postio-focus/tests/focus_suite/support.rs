//! What the cases share: a store with mail in it, a host over it as Focus's
//! startup would have opened one, and somewhere to keep that host until the
//! case is over.

use std::any::Any;
use std::cell::RefCell;

use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_host::Host;
use postio_model::{Account, EmailAddress, MailboxId, Message, MessageId, ThreadId};
use postio_storage::repository::{MessageRepository, ThreadingRepository};
use postio_storage::{BlobStore, Store, test_support};

thread_local! {
    static KEPT: RefCell<Vec<Box<dyn Any>>> = RefCell::new(Vec::new());
}

/// Hold `value` until the case is over, and drop it then, outside every
/// runtime: a host owns a runtime of its own, which may not be dropped from
/// inside another's `block_on`, which is where a case's body runs.
pub fn keep<T: 'static>(value: T) {
    KEPT.with(|kept| kept.borrow_mut().push(Box::new(value)));
}

/// Drop what the last case kept. The harness calls this between cases.
pub fn drop_kept() {
    let kept = KEPT.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    drop(kept);
}

/// Whether there is a display to draw on. Says so when there is not, and a
/// case that needs one returns: the runner puts the suite on a headless
/// compositor, and a machine without one skips rather than fails.
pub fn display() -> bool {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    true
}

/// A throwaway store, and the directory its blobs live in.
pub struct Fixture {
    /// The store.
    pub database: Store,
    /// The one account.
    pub account: Account,
    /// Its inbox.
    pub inbox: MailboxId,
    blobs: tempfile::TempDir,
}

/// A fixed moment the fixtures' mail is dated from: Saturday 26 September
/// 2026, 16:09 local time as the screens draw it, in UTC.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 14, 9, 0)
        .single()
        .expect("a fixed, valid date")
}

impl Fixture {
    /// One account with an inbox, an archive and a trash, and no mail.
    pub async fn empty() -> Fixture {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Archive").await;
        test_support::mailbox(&connection, &account, "Trash").await;
        drop(connection);
        Fixture {
            database,
            account,
            inbox,
            blobs: tempfile::tempdir().expect("a blob directory"),
        }
    }

    /// File one message into the inbox, threaded as sync would thread it:
    /// from `from` (a name and an address), with `subject` and `preview`,
    /// `minutes` before [`now`]. Answers the message and its conversation.
    pub async fn file(
        &self,
        from: (&str, &str),
        subject: &str,
        preview: &str,
        minutes: i64,
    ) -> (MessageId, ThreadId) {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(
            self.account.id,
            self.inbox,
            now() - Duration::minutes(minutes),
        );
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.subject = Some(subject.to_owned());
        message.preview = Some(preview.to_owned());
        static FILED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = FILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<fixture.{serial}@example.test>"
        )));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        let thread = ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded")
            .thread_id;
        (message.id, thread)
    }

    /// A host over this store, started as Focus's startup starts one once
    /// the store is open: in this process, with its own runtime.
    pub fn host(&self) -> Host {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Host::start(self.database.clone(), blobs, |wiring| wiring).expect("a host")
    }
}
