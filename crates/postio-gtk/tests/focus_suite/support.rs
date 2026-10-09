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
        // A store that has synced before: its inbox says so, the way a
        // finished first pass leaves it (T220), so an empty one is empty.
        let mailboxes = postio_storage::repository::MailboxRepository::new(&connection);
        let mut synced = mailboxes
            .get(inbox)
            .await
            .expect("the inbox reads")
            .expect("the inbox");
        synced.last_synced_at = Some(Utc::now());
        mailboxes
            .update(&synced)
            .await
            .expect("the inbox is stamped");
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
    /// A host over the fixture's store, and the sink its events go out
    /// through: the seam a case says what sync did with, as the engine
    /// would, with no server behind it.
    pub fn host_telling(&self) -> (Host, postio_core::bridge::EventSink) {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        let sink = std::rc::Rc::new(RefCell::new(None));
        let host = Host::start(self.database.clone(), blobs, {
            let sink = std::rc::Rc::clone(&sink);
            move |wiring| {
                sink.replace(Some(wiring.events.clone()));
                wiring
            }
        })
        .expect("a host");
        let sink = sink.take().expect("the wiring's events");
        (host, sink)
    }

    /// The fixture's blob store, as the host opens it: what a case files a
    /// raw source into, and reads back.
    pub fn blob_store(&self) -> BlobStore {
        BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store")
    }

    pub fn host(&self) -> Host {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Host::start(self.database.clone(), blobs, |wiring| wiring).expect("a host")
    }
}

/// A store with no account at all: the shape a brand-new installation is in
/// (T171's "first run"), where `Fixture::empty` always seeds one.
pub struct NoAccount {
    pub database: Store,
    blobs: tempfile::TempDir,
}

impl NoAccount {
    /// An empty store, no account, nothing synced.
    pub async fn new() -> NoAccount {
        NoAccount {
            database: test_support::memory().await,
            blobs: tempfile::tempdir().expect("a blob directory"),
        }
    }

    /// A host over this store, as Focus's startup would open one, proving
    /// any account added against `backend` rather than a real network or
    /// keyring (T171): the seam `postio-host`'s own onboarding tests use
    /// (`Wiring::with_mail`), with the secrets store it also has to
    /// override -- `Wiring::new`'s default reaches the platform keyring,
    /// which no test in the default suite may touch.
    pub fn host_signing_in_to(&self, backend: postio_account::backend::MockBackend) -> Host {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Host::start(self.database.clone(), blobs, move |wiring| {
            wiring
                .with_secrets(std::sync::Arc::new(
                    postio_account::secret::MemorySecretStore::new(),
                ))
                .with_mail(postio_session::MailOverride {
                    backend: std::sync::Arc::new(backend),
                    smtp: std::sync::Arc::new(postio_smtp::transport::ScriptedConnector::new(
                        postio_smtp::transport::SmtpScript::new("220 ready"),
                    )),
                })
        })
        .expect("a host")
    }
}

/// Every widget under `root`, in tree order.
pub fn descendants(root: &impl gtk::prelude::IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    use gtk::prelude::*;
    let mut found = Vec::new();
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        found.push(widget.clone());
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

/// Every widget under `root`, `root` included, that wears `class`, in tree
/// order.
pub fn with_class(root: &impl gtk::prelude::IsA<gtk::Widget>, class: &str) -> Vec<gtk::Widget> {
    use gtk::prelude::*;
    let mut found = Vec::new();
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            found.push(widget.clone());
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

/// Whether every visible keycap under `root` has taught its control the
/// key as `KeyShortcuts` (`a11y::teach_shortcuts`, T142) -- and there was
/// at least one keycap to teach, so a surface with none does not pass by
/// vacuously finding nothing.
pub fn keycaps_are_taught(root: &impl gtk::prelude::IsA<gtk::Widget>) -> bool {
    use gtk::prelude::*;
    let mut found_a_cap = false;
    for cap in with_class(root, "postio-keyhint") {
        if !cap.is_visible() {
            continue;
        }
        let mut up = cap.parent();
        while let Some(widget) = up {
            if widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>() {
                found_a_cap = true;
                if !gtk::test_accessible_has_property(
                    &widget,
                    gtk::AccessibleProperty::KeyShortcuts,
                ) {
                    return false;
                }
                break;
            }
            up = widget.parent();
        }
    }
    found_a_cap
}

/// The one widget under `root` wearing `class`; fails the case if there is
/// not exactly one.
pub fn only(root: &impl gtk::prelude::IsA<gtk::Widget>, class: &str) -> gtk::Widget {
    let mut found = with_class(root, class);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one widget with class {class}, found {}",
        found.len()
    );
    found.remove(0)
}

/// What a person reads in `widget`: the text of every label under it that
/// is on screen, in tree order.
pub fn texts(widget: &impl gtk::prelude::IsA<gtk::Widget>) -> Vec<String> {
    use gtk::prelude::*;
    let mut said = Vec::new();
    let mut stack = vec![widget.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if !widget.is_mapped() {
            continue;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            let text = label.text().to_string();
            if !text.is_empty() {
                said.push(text);
            }
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    said
}

/// A fixture with three conversations in its inbox, and a Focus window
/// adopted over it, showing them: the shape most cases start from.
pub async fn three_in_the_inbox() -> (Fixture, postio_gtk::window::FocusWindow) {
    use gtk::prelude::*;
    let fixture = Fixture::empty().await;
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Atlas budget",
            "The numbers are attached.",
            30,
        )
        .await;
    fixture
        .file(
            ("Lena Park", "lena@example.org"),
            "Harbor draft",
            "Uploaded the second draft.",
            20,
        )
        .await;
    fixture
        .file(
            ("Tomás Reyes", "tomas@example.net"),
            "Staffing plan",
            "Sharing the draft before Monday.",
            10,
        )
        .await;
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    let session =
        postio_gtk::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
    assert!(
        crate::settle_until(async || window.rows_on_screen().len() >= 3).await,
        "the fixture's three conversations never reached the screen"
    );
    keep(session);
    (fixture, window)
}

impl Fixture {
    /// A store seeded with `messages` synthetic messages, threaded into
    /// conversations of mixed lengths and spread over the folders as a real
    /// account's are: the large fixture the paging benches read.
    pub async fn large(messages: usize) -> Fixture {
        let database = test_support::memory().await;
        let report = postio_storage::seed::seed_large(&database, 7, messages).await;
        let inbox = report
            .mailbox(postio_model::MailboxRole::Inbox)
            .expect("the seed files an inbox")
            .id;
        Fixture {
            database,
            account: report.account,
            inbox,
            blobs: tempfile::tempdir().expect("a blob directory"),
        }
    }

    /// A Focus window adopted over this store, and the client it reads
    /// through, once the first rows are on screen.
    pub async fn open(&self) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        self.open_sized(None).await
    }

    /// As [`open`](Self::open), the window asking to be `size` big when it
    /// is first shown -- the only time GTK lets a window ask. The
    /// compositor may still maximise it: its monitor is 1280x800.
    pub async fn open_sized(
        &self,
        size: Option<(i32, i32)>,
    ) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        use gtk::prelude::*;
        let window = postio_gtk::window::FocusWindow::new(None);
        if let Some((width, height)) = size {
            window.set_default_size(width, height);
        }
        window.present();
        let session =
            postio_gtk::startup::adopt(&window, self.host(), &postio_config::Config::default());
        let client = session.client().clone();
        keep(session);
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the store's inbox never reached the screen"
        );
        (window, client)
    }
}

impl Fixture {
    /// Put labels named `names` on `message`, making each, in that order.
    pub async fn label(&self, message: MessageId, names: &[&str]) {
        let connection = self.database.connect().await.expect("a connection");
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        for name in names {
            let mut label = postio_model::Label::new(self.account.id, *name);
            labels.create(&mut label).await.expect("a label");
            labels.attach(message, label.id).await.expect("attached");
        }
    }
}

impl Fixture {
    /// Index everything filed so far, as sync indexes what it files.
    pub async fn index(&self) {
        let connection = self.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("indexed");
    }

    /// Record `name <address>` as a correspondent of the fixture's account,
    /// as sync does for every address it sees.
    pub async fn correspondent(&self, name: &str, address: &str) {
        let connection = self.database.connect().await.expect("a connection");
        postio_storage::repository::ContactRepository::new(&connection)
            .record(
                Some(self.account.id),
                &EmailAddress::new(Some(name), address),
                now(),
            )
            .await
            .expect("a correspondent");
    }

    /// A message from `from` about `subject`, filed into the archive
    /// `minutes` before the fixture's now and filtered there for `reason`
    /// ("notification", as the store spells it), as Focus's filing does.
    pub async fn filtered(
        &self,
        from: (&str, &str),
        subject: &str,
        reason: &str,
        source: Option<&str>,
        minutes: i64,
    ) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let archive: i64 = postio_storage::sql::scalar(
            &connection,
            "SELECT id FROM mailboxes WHERE account_id = ?1 AND name = 'Archive'",
            [self.account.id.get()],
        )
        .await
        .expect("the archive");
        let at = now() - Duration::minutes(minutes);
        let mut message = Message::new(self.account.id, MailboxId::new(archive), at);
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.subject = Some(subject.to_owned());
        message.preview = Some(format!("About {subject}."));
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        postio_storage::repository::FilterDecisionRepository::new(&connection)
            .record(&postio_storage::repository::FilterDecision {
                message: id,
                reason: postio_storage::repository::FilterReason::from_name(reason)
                    .expect("a reason the store has"),
                source: source.map(str::to_owned),
                layer: postio_storage::repository::FilterLayer::Header,
                decided_at: at,
            })
            .await
            .expect("a decision");
        id
    }

    /// A second account, enabled, with an inbox of its own.
    pub async fn second_account(&self) -> (Account, MailboxId) {
        let connection = self.database.connect().await.expect("a connection");
        let mut account = Account::new(
            "Second",
            EmailAddress::new(Some("Second User"), "second@example.org"),
        );
        account.incoming.host = "imap.example.org".to_owned();
        account.outgoing.host = "smtp.example.org".to_owned();
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("a second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX")
            .await
            .id;
        (account, inbox)
    }

    /// File a message from Lena about `subject` into `account`'s
    /// `mailbox`, `minutes` before the fixture's now, threaded.
    pub async fn file_as(
        &self,
        account: postio_model::AccountId,
        mailbox: MailboxId,
        subject: &str,
        minutes: i64,
    ) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(account, mailbox, now() - Duration::minutes(minutes));
        message.from = vec![EmailAddress::new(Some("Lena Park"), "lena@example.org")];
        message.subject = Some(subject.to_owned());
        message.preview = Some(format!("About {subject}."));
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<file-as.{}.{}@example.test>",
            account.get(),
            subject.replace(' ', ".")
        )));
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadingRepository::new(&connection, account)
            .thread(&message)
            .await
            .expect("threaded");
        id
    }

    /// A folder named `name` in the fixture's account.
    pub async fn folder(&self, name: &str) -> MailboxId {
        let connection = self.database.connect().await.expect("a connection");
        test_support::mailbox(&connection, &self.account, name)
            .await
            .id
    }

    /// File a message from Ada about `subject` into `mailbox`, `minutes`
    /// before the fixture's now.
    pub async fn file_in(&self, mailbox: MailboxId, subject: &str, minutes: i64) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message =
            Message::new(self.account.id, mailbox, now() - Duration::minutes(minutes));
        message.from = vec![EmailAddress::new(Some("Ada Moreno"), "ada@example.com")];
        message.subject = Some(subject.to_owned());
        message.preview = Some(format!("About {subject}."));
        static FILED_IN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = FILED_IN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<filed-in.{serial}@example.test>"
        )));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        message.id
    }

    /// File a message from `from` about `subject`, `minutes` before the
    /// fixture's now, whose `List-Id` is `list_id` (US14 scenario 1): what
    /// a `list:` rule matches.
    pub async fn file_from_list(
        &self,
        from: (&str, &str),
        subject: &str,
        list_id: &str,
        minutes: i64,
    ) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(
            self.account.id,
            self.inbox,
            now() - Duration::minutes(minutes),
        );
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.subject = Some(subject.to_owned());
        message.list_id = Some(list_id.to_owned());
        static FILED_LIST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = FILED_LIST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<fixture-list.{serial}@example.test>"
        )));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        message.id
    }

    /// Store `html` as `message`'s body, fetched in full.
    pub async fn write_html_body(&self, message: MessageId, html: &str) {
        let connection = self.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .set_body(
                message,
                &postio_storage::repository::StoredBody {
                    text: None,
                    html: Some(html.to_owned()),
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
    }

    /// A message from Ada with one attached file, `name`, in the inbox.
    pub async fn file_with_attachment(&self, subject: &str, name: &str) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(self.account.id, self.inbox, now() - Duration::minutes(5));
        message.from = vec![EmailAddress::new(Some("Ada Moreno"), "ada@example.com")];
        message.subject = Some(subject.to_owned());
        message.preview = Some("See attached.".to_owned());
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<attached.{name}@example.test>"
        )));
        let mut attachment =
            postio_model::Attachment::new(MessageId::UNASSIGNED, "application/pdf", 48_000);
        attachment.filename = Some(name.to_owned());
        // A part the reader draws is one it can address.
        attachment.part_id = Some("2".to_owned());
        message.attachments.push(attachment);
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        message.id
    }

    /// A message from Ada with one attached file, `report.pdf`, whose bytes
    /// are [`ATTACHED`]: its raw source is kept, so the host can write the
    /// part out as a save does, with no server behind it.
    pub async fn file_with_kept_attachment(&self, subject: &str) -> MessageId {
        let message = self.file_with_attachment(subject, "report.pdf").await;
        self.write_body(message, "See attached.").await;
        let raw = format!(
            "From: Ada Moreno <ada@example.com>\r\nSubject: {subject}\r\n\
             MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"edge\"\r\n\r\n\
             --edge\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
             --edge\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\n\
             Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{ATTACHED_BASE64}\r\n--edge--\r\n"
        );
        self.write_raw(message, raw.as_bytes()).await;
        message
    }

    /// Keep `raw` as `message`'s raw source, as a fetched message is kept.
    pub async fn write_raw(&self, message: MessageId, raw: &[u8]) {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        let blob = blobs.put(raw).expect("the raw source is kept");
        let connection = self.database.connect().await.expect("a connection");
        let repository = MessageRepository::new(&connection);
        let mut row = repository
            .get(message)
            .await
            .expect("the message reads")
            .expect("the message is there");
        row.raw_blob_id = Some(blob);
        repository.update(&mut row).await.expect("the row names it");
    }

    /// A conversation of `count` messages about `subject`, each a reply to
    /// the one before, the newest `minutes` ago and each earlier one an hour
    /// before it; the body of the nth (from 1, oldest first) is "Message n".
    /// Answers the messages, oldest first.
    pub async fn thread_of(&self, subject: &str, count: usize, minutes: i64) -> Vec<MessageId> {
        let connection = self.database.connect().await.expect("a connection");
        static THREADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let thread = THREADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut previous: Option<postio_model::RfcMessageId> = None;
        let mut ids = Vec::new();
        for n in 1..=count {
            let at = now() - Duration::minutes(minutes + 60 * (count - n) as i64);
            let mut message = Message::new(self.account.id, self.inbox, at);
            message.from = vec![EmailAddress::new(Some("Ada Moreno"), "ada@example.com")];
            message.subject = Some(if n == 1 {
                subject.to_owned()
            } else {
                format!("Re: {subject}")
            });
            message.preview = Some(format!("Message {n}"));
            let id = postio_model::RfcMessageId::new(format!("<thread.{thread}.{n}@example.test>"));
            message.rfc_message_id = Some(id.clone());
            if let Some(parent) = previous.replace(id) {
                message.in_reply_to = Some(parent.clone());
                message.references = vec![parent];
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            ThreadingRepository::new(&connection, self.account.id)
                .thread(&message)
                .await
                .expect("threaded");
            ids.push(message.id);
        }
        drop(connection);
        for (n, id) in ids.iter().enumerate() {
            self.write_body(*id, &format!("Message {}", n + 1)).await;
        }
        ids
    }

    /// Store `text` as `message`'s body, fetched in full.
    pub async fn write_body(&self, message: MessageId, text: &str) {
        let connection = self.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .set_body(
                message,
                &postio_storage::repository::StoredBody {
                    text: Some(text.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
    }

    /// [`Fixture::write_body`], and the body in the search index too, as the
    /// running app's body indexer would put it: what a search can find in it.
    pub async fn write_searchable_body(&self, message: MessageId, text: &str) {
        self.write_body(message, text).await;
        let connection = self.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("indexed");
        postio_index::index::index_body(&connection, message.get(), Some(text))
            .await
            .expect("a searchable body");
    }
}

impl Fixture {
    /// Mark `message` with a question quoting `sentence`, as the built-in
    /// detector would have.
    pub async fn ask(&self, message: MessageId, sentence: &str) {
        use postio_storage::repository::{Marker, MarkerRepository, MarkerSource};
        let connection = self.database.connect().await.expect("a connection");
        MarkerRepository::new(&connection)
            .insert(&Marker {
                message,
                kind: postio_model::listing::MarkerKind::Question,
                source: MarkerSource::Detector,
                span: Some((0, sentence.chars().count() as u32)),
                excerpt: Some(sentence.to_owned()),
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
                invite_state: None,
                answer: None,
                dismissed_at: None,
            })
            .await
            .expect("a marker");
    }
}

/// Press `name` (a GDK key name: "x", "J", "exclam", "Escape") with
/// `state`'s modifiers, as GTK would deliver it: [`deliver_with`], not a call
/// to the window's handler (T200).
pub fn press(window: &postio_gtk::window::FocusWindow, name: &str, state: gtk::gdk::ModifierType) {
    deliver_with(window, name, state);
}

/// Press each of `keys`, unmodified but for shift on a capital.
pub fn keys(window: &postio_gtk::window::FocusWindow, keys: &[&str]) {
    for name in keys {
        let shifted = name.chars().count() == 1 && name.chars().all(char::is_uppercase);
        let state = if shifted {
            gtk::gdk::ModifierType::SHIFT_MASK
        } else {
            gtk::gdk::ModifierType::empty()
        };
        press(window, name, state);
    }
}

/// The subjects of the rows on screen, top to bottom.
pub fn subjects(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    window
        .pane()
        .map(|pane| {
            pane.rows_on_screen()
                .iter()
                .filter_map(|row| row.item())
                .map(|item| match item.as_conversation() {
                    Some(row) => row
                        .summary
                        .representative
                        .subject
                        .clone()
                        .unwrap_or_default(),
                    None => match &item {
                        postio_gtk::list::FocusRow::Digest(digest) => {
                            format!("digest: {}", digest.rule)
                        }
                        _ => String::new(),
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Fixture {
    /// Five conversations in the inbox, "First" the newest, "Fifth" the
    /// oldest. Answers their messages, in that order.
    pub async fn file_five(&self) -> Vec<MessageId> {
        let mut filed = Vec::new();
        for (subject, minutes) in [
            ("First", 10),
            ("Second", 20),
            ("Third", 30),
            ("Fourth", 40),
            ("Fifth", 50),
        ] {
            let (message, _) = self
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    subject,
                    "A line of text.",
                    minutes,
                )
                .await;
            filed.push(message);
        }
        filed
    }

    /// [`Self::file_five`], and a window open over them.
    pub async fn five(&self) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        self.file_five().await;
        self.open_five().await
    }

    /// A window open over five conversations already filed.
    pub async fn open_five(&self) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        let (window, client) = self.open().await;
        assert!(
            crate::settle_until(async || subjects(&window).len() == 5).await,
            "the five conversations never reached the screen: {:?}",
            subjects(&window)
        );
        (window, client)
    }
}

impl Fixture {
    /// A message from `from`, to `to` and copied to `cc` (each a name and an
    /// address), dated `date`, with a plain body. Answers its id.
    pub async fn file_addressed(
        &self,
        from: (&str, &str),
        to: &[(&str, &str)],
        cc: &[(&str, &str)],
        subject: &str,
        date: DateTime<Utc>,
    ) -> MessageId {
        let connection = self.database.connect().await.expect("a connection");
        let mut message = Message::new(self.account.id, self.inbox, date);
        message.date = Some(date);
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.to = to
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect();
        message.cc = cc
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect();
        message.subject = Some(subject.to_owned());
        message.preview = Some("Body.".to_owned());
        static ADDRESSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = ADDRESSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<addressed.{serial}@example.test>"
        )));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadingRepository::new(&connection, self.account.id)
            .thread(&message)
            .await
            .expect("threaded");
        drop(connection);
        self.write_body(message.id, "Body of the message.").await;
        message.id
    }

    /// Put a label named `name`, drawn in `colour` (`#rrggbb`), on `message`.
    pub async fn label_in(&self, message: MessageId, name: &str, colour: &str) {
        let connection = self.database.connect().await.expect("a connection");
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        let mut label = postio_model::Label::new(self.account.id, name);
        label.color = Some(colour.to_owned());
        labels.create(&mut label).await.expect("a label");
        labels.attach(message, label.id).await.expect("attached");
    }
}

/// Press `name` (a GDK key name) as GTK delivers a key press: to the widget
/// the window's keyboard is on (the window itself when nothing has it),
/// through each key controller from the window down to it in the capture
/// phase, then the target's own, then back up in the bubble phase, until
/// one claims it. Answers whether one did.
///
/// [`press`] calls the window's handler directly, which is not what a key
/// press does: it cannot see the keyboard sitting somewhere a controller
/// takes the key first, or the handler never being reached -- which is
/// what happened under every dialog (T195). This can. What it cannot run is a widget class's own key bindings (a
/// scroller's, a window's focus moves): GTK offers no way to run them but a
/// real event.
pub fn deliver(window: &postio_gtk::window::FocusWindow, name: &str) -> bool {
    let state = if name.chars().count() == 1 && name.chars().all(char::is_uppercase) {
        gtk::gdk::ModifierType::SHIFT_MASK
    } else {
        gtk::gdk::ModifierType::empty()
    };
    deliver_with(window, name, state)
}

/// [`deliver`] with the modifiers held, for `Ctrl+Return` and the like.
pub fn deliver_with(
    window: &postio_gtk::window::FocusWindow,
    name: &str,
    state: gtk::gdk::ModifierType,
) -> bool {
    use gtk::prelude::*;
    let key = gtk::gdk::Key::from_name(name).unwrap_or_else(|| panic!("{name} is a key"));
    let keyval = gtk::glib::translate::IntoGlib::into_glib(key);
    let target: gtk::Widget =
        gtk::prelude::GtkWindowExt::focus(window).unwrap_or_else(|| window.clone().upcast());
    // GTK's key path starts at the focus and climbs its parents. A focus
    // that has left the window (its row was destroyed in a redraw) has none,
    // so no key would reach the window: a person's keyboard is dead until a
    // click. Say so, rather than let the key quietly find nothing (T200).
    assert!(
        target.root().is_some(),
        "{name}: the keyboard is on a {} that has left the window",
        target.type_().name()
    );
    // GTK runs a key through the widgets from the focus up to the innermost
    // dialog presented over the window, and no further: a controller on the
    // window, or anywhere between it and the dialog, never sees a key the
    // dialog's keyboard is given. Measured with real key presses injected
    // into the headless compositor (mutter's RemoteDesktop), GTK 4.22 and
    // libadwaita 1.9 (T195).
    let mut chain = vec![target.clone()];
    while !chain
        .last()
        .is_some_and(|widget| widget.is::<adw::Dialog>())
        && let Some(parent) = chain.last().and_then(|widget| widget.parent())
    {
        chain.push(parent);
    }
    let fire = |widget: &gtk::Widget, phase: gtk::PropagationPhase| -> bool {
        let controllers = widget.observe_controllers();
        (0..controllers.n_items()).any(|at| {
            let Some(keys) = controllers
                .item(at)
                .and_downcast::<gtk::EventControllerKey>()
            else {
                return false;
            };
            keys.propagation_phase() == phase
                && keys.emit_by_name::<bool>("key-pressed", &[&keyval, &0u32, &state])
        })
    };
    let claimed = chain
        .iter()
        .rev()
        .any(|widget| fire(widget, gtk::PropagationPhase::Capture))
        || fire(&target, gtk::PropagationPhase::Target)
        || chain
            .iter()
            .any(|widget| fire(widget, gtk::PropagationPhase::Bubble));
    crate::settle();
    claimed
}

/// The button under `root` that says `label`: the one a person clicks,
/// once it is on screen (a dialog's buttons are not, until it has opened).
pub fn button_labelled(root: &impl gtk::prelude::IsA<gtk::Widget>, label: &str) -> gtk::Widget {
    use gtk::prelude::*;
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    loop {
        crate::settle();
        if let Some(found) = descendants(root).into_iter().find(|widget| {
            widget.is::<gtk::Button>()
                && widget.is_mapped()
                && texts(widget).iter().any(|text| text == label)
        }) {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no button says {label:?}: {:?}",
            texts(root)
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Click the list row under `root` that says `text`, as a person does: the
/// row on screen when they click. A list that is still redrawing replaces its
/// rows, so a row found a moment ago may be gone; this finds one afresh until
/// a click at its middle reaches it.
pub fn click_row_saying(
    window: &postio_gtk::window::FocusWindow,
    root: &impl gtk::prelude::IsA<gtk::Widget>,
    text: &str,
) {
    use gtk::prelude::*;
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    loop {
        crate::settle();
        let row = descendants(root).into_iter().find(|widget| {
            widget.is::<gtk::ListBoxRow>()
                && widget.is_mapped()
                && texts(widget).iter().any(|said| said == text)
        });
        if let Some(row) = row {
            let native = native_of(&row, window);
            // Mapped is not laid out: a row built a moment ago has no size
            // until the next frame, and nothing can be clicked at no size.
            let point = row
                .compute_bounds(&native)
                .filter(|bounds| bounds.width() > 0.0 && bounds.height() > 0.0)
                .map(|bounds| {
                    gtk::graphene::Point::new(
                        bounds.x() + bounds.width() / 2.0,
                        bounds.y() + bounds.height() / 2.0,
                    )
                });
            if let Some(point) = point {
                let (x, y) = (f64::from(point.x()), f64::from(point.y()));
                let picked = native.pick(x, y, gtk::PickFlags::DEFAULT);
                if picked
                    .as_ref()
                    .is_some_and(|picked| *picked == row || picked.is_ancestor(&row))
                {
                    click_on(&native, x, y, 1);
                    return;
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no row saying {text:?} could be clicked: {:?}",
            texts(root)
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// [`click_at`] at (`x`, `y`) in `widget`'s own space, once a click there
/// reaches `widget` (see [`wait_to_be_pickable`]).
pub fn click_in(
    window: &postio_gtk::window::FocusWindow,
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    x: f32,
    y: f32,
    n_press: i32,
) {
    let native = native_of(widget.as_ref(), window);
    let (x, y) = wait_to_be_pickable(&native, widget.as_ref(), x, y);
    click_on(&native, x, y, n_press);
}

/// Click the middle of `widget` as a person would, `n_press` times (2 is a
/// double-click): see [`click_at`].
pub fn click(
    window: &postio_gtk::window::FocusWindow,
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    n_press: i32,
) {
    use gtk::prelude::*;
    let widget = widget.as_ref();
    // Its middle is only known once it is laid out: a widget a moment old
    // has no size yet, and a click at its corner can miss it -- the click a
    // person never makes, since they aim at what they see.
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !(widget.is_mapped() && widget.width() > 0 && widget.height() > 0)
        && std::time::Instant::now() < deadline
    {
        crate::settle();
    }
    let (width, height) = (widget.width() as f32, widget.height() as f32);
    let native = native_of(widget, window);
    let (x, y) = wait_to_be_pickable(&native, widget, width / 2.0, height / 2.0);
    click_on(&native, x, y, n_press);
}

/// The surface `widget` is drawn on: the window, or a popover's own.
fn native_of(widget: &gtk::Widget, window: &postio_gtk::window::FocusWindow) -> gtk::Widget {
    use gtk::prelude::*;
    widget
        .native()
        .map(|native| native.upcast())
        .unwrap_or_else(|| window.clone().upcast())
}

/// The window coordinates of (`x`, `y`) in `widget`, once that is a place a
/// click reaches `widget`: what a person waits for. A dialog slides in, and
/// until it has stopped the point is over the scrim, not the button; a
/// person clicks when the button is where they see it. Panics, naming what
/// covers it, when it never is: a button under something is a bug.
fn wait_to_be_pickable(window: &gtk::Widget, widget: &gtk::Widget, x: f32, y: f32) -> (f64, f64) {
    use gtk::prelude::*;
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    loop {
        crate::settle();
        let point = widget
            .compute_point(window, &gtk::graphene::Point::new(x, y))
            .unwrap_or_else(|| {
                panic!(
                    "{} has no place in the {}",
                    widget.type_().name(),
                    window.type_().name()
                )
            });
        let (x, y) = (f64::from(point.x()), f64::from(point.y()));
        let picked = window.pick(x, y, gtk::PickFlags::DEFAULT);
        if widget.width() > 0
            && widget.height() > 0
            && picked
                .as_ref()
                .is_some_and(|picked| picked == widget || picked.is_ancestor(widget))
        {
            return (x, y);
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "a click at ({x}, {y}) never reached the {}: it lands on {}",
                widget.type_().name(),
                picked.map_or("nothing".to_owned(), |picked| picked
                    .type_()
                    .name()
                    .to_string())
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Press and release the primary button at (`x`, `y`) in the window,
/// `n_press` times over, as GTK delivers a click: to whatever is *picked*
/// there (so a widget covered by a dialog's scrim is not the one that gets
/// it, and one that is not mapped, clipped away or insensitive cannot be
/// clicked), through each `GestureClick` from the window down to it in the
/// capture phase, then the picked widget's own, then back up in the bubble
/// phase; and no further than the innermost dialog or popover surface on the
/// way (a popover is a surface of its own; stopping at a dialog is the
/// conservative reading of what [`deliver`] measured for keys, not something
/// measured for the pointer: a click on the scrim is outside the dialog and
/// reaches the window either way).
///
/// GTK offers no way to build a button event in-process (`GdkEvent`s are
/// made by a backend), so this drives the gestures' `pressed` and `released`
/// signals itself, in the order and along the path GTK would, with the
/// press count of the click it is. What it cannot see is a gesture claiming
/// the sequence (no sequence exists to claim), so it approximates that:
/// a `GtkButton` claims what reaches it, and nothing above one is run.
pub fn click_at(window: &postio_gtk::window::FocusWindow, x: f64, y: f64, n_press: i32) {
    use gtk::prelude::*;
    click_on(window.upcast_ref::<gtk::Widget>(), x, y, n_press);
}

/// [`click_at`] on a surface `native` (a window, or a popover's own), in its
/// coordinates.
fn click_on(window: &gtk::Widget, x: f64, y: f64, n_press: i32) {
    use gtk::prelude::*;
    let picked = window
        .pick(x, y, gtk::PickFlags::DEFAULT)
        .unwrap_or_else(|| window.clone().upcast());
    // A popover is a surface of its own: an event on it goes up to the
    // popover and no further, as it does to a dialog.
    let mut chain = vec![picked.clone()];
    while !chain
        .last()
        .is_some_and(|widget| widget.is::<adw::Dialog>() || widget.is::<gtk::Native>())
        && let Some(parent) = chain.last().and_then(|widget| widget.parent())
    {
        chain.push(parent);
    }
    let window_widget = window.clone();
    let gestures = |widget: &gtk::Widget, phase: gtk::PropagationPhase| -> Vec<gtk::GestureClick> {
        let controllers = widget.observe_controllers();
        (0..controllers.n_items())
            .filter_map(|at| controllers.item(at).and_downcast::<gtk::GestureClick>())
            .filter(|gesture| gesture.propagation_phase() == phase)
            // A primary click: a gesture for another button (a row's
            // context menu on the secondary) never sees it.
            .filter(|gesture| matches!(gesture.button(), 0 | gtk::gdk::BUTTON_PRIMARY))
            .collect()
    };
    // The path, in the order GTK runs it: widget and gesture.
    let mut path: Vec<(gtk::Widget, gtk::GestureClick)> = Vec::new();
    for widget in chain.iter().rev() {
        path.extend(
            gestures(widget, gtk::PropagationPhase::Capture)
                .into_iter()
                .map(|gesture| (widget.clone(), gesture)),
        );
    }
    path.extend(
        gestures(&picked, gtk::PropagationPhase::Target)
            .into_iter()
            .map(|gesture| (picked.clone(), gesture)),
    );
    for widget in &chain {
        path.extend(
            gestures(widget, gtk::PropagationPhase::Bubble)
                .into_iter()
                .map(|gesture| (widget.clone(), gesture)),
        );
    }
    let to = |widget: &gtk::Widget| {
        let point = window_widget
            .compute_point(widget, &gtk::graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
        (f64::from(point.x()), f64::from(point.y()))
    };
    for press in 1..=n_press {
        let mut reached = Vec::new();
        for (widget, gesture) in &path {
            let (x, y) = to(widget);
            gesture.emit_by_name::<()>("pressed", &[&press, &x, &y]);
            reached.push((widget, gesture));
            if widget.is::<gtk::Button>() {
                break;
            }
        }
        crate::settle();
        for (widget, gesture) in reached {
            let (x, y) = to(widget);
            gesture.emit_by_name::<()>("released", &[&press, &x, &y]);
        }
        crate::settle();
    }
}

/// Where the window's keyboard is, as the widget types and CSS classes
/// from the window down: for a failure to name.
pub fn focus_path(window: &postio_gtk::window::FocusWindow) -> String {
    use gtk::prelude::*;
    let mut chain = Vec::new();
    let mut at: Option<gtk::Widget> = gtk::prelude::GtkWindowExt::focus(window);
    while let Some(widget) = at {
        chain.push(format!(
            "{}{:?}",
            widget.type_().name(),
            widget.css_classes()
        ));
        at = widget.parent();
    }
    chain.reverse();
    if chain.is_empty() {
        "nothing".to_owned()
    } else {
        chain.join(" > ")
    }
}

/// The bytes of the part [`Fixture::file_with_kept_attachment`] attaches:
/// not text, so a save that went through a string would show.
pub const ATTACHED: &[u8] = b"%PDF-1.4\n\x00\xff\x10 the plan\r\n";
const ATTACHED_BASE64: &str = "JVBERi0xLjQKAP8QIHRoZSBwbGFuDQo=";

/// Wait for `widget` to stop moving within `root`: a settings detail page
/// slides in when its row is clicked, carrying its buttons with it, and a
/// press and a release that land at two places are no click -- on a slow
/// runner the case pressed "Add signature" mid-slide and nothing opened.
pub fn settle_still(
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    root: &impl gtk::prelude::IsA<gtk::Widget>,
) {
    use gtk::prelude::*;
    let widget = widget.as_ref();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(2));
    let mut last = widget.compute_bounds(root);
    while std::time::Instant::now() < deadline {
        crate::settle();
        std::thread::sleep(std::time::Duration::from_millis(30));
        crate::settle();
        let now = widget.compute_bounds(root);
        if now.is_some() && now == last {
            return;
        }
        last = now;
    }
}
