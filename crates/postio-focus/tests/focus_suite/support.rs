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

    pub fn host(&self) -> Host {
        let blobs = BlobStore::open(self.blobs.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Host::start(self.database.clone(), blobs, |wiring| wiring).expect("a host")
    }
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
pub async fn three_in_the_inbox() -> (Fixture, postio_focus::window::FocusWindow) {
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
    let window = postio_focus::window::FocusWindow::new(None);
    window.present();
    let session =
        postio_focus::startup::adopt(&window, fixture.host(), &postio_config::Config::default());
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
    pub async fn open(&self) -> (postio_focus::window::FocusWindow, postio_client::Client) {
        use gtk::prelude::*;
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        let session =
            postio_focus::startup::adopt(&window, self.host(), &postio_config::Config::default());
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
/// `state`'s modifiers, through the window's one keyboard path.
pub fn press(
    window: &postio_focus::window::FocusWindow,
    name: &str,
    state: gtk::gdk::ModifierType,
) {
    let key = gtk::gdk::Key::from_name(name).unwrap_or_else(|| panic!("{name} is a key"));
    let _ = window.handle_key(key, state);
    crate::settle();
}

/// Press each of `keys`, unmodified but for shift on a capital.
pub fn keys(window: &postio_focus::window::FocusWindow, keys: &[&str]) {
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
pub fn subjects(window: &postio_focus::window::FocusWindow) -> Vec<String> {
    window
        .pane()
        .map(|pane| {
            pane.rows_on_screen()
                .iter()
                .filter_map(|row| row.item())
                .map(|item| {
                    item.row()
                        .summary
                        .representative
                        .subject
                        .clone()
                        .unwrap_or_default()
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
    pub async fn five(&self) -> (postio_focus::window::FocusWindow, postio_client::Client) {
        self.file_five().await;
        self.open_five().await
    }

    /// A window open over five conversations already filed.
    pub async fn open_five(&self) -> (postio_focus::window::FocusWindow, postio_client::Client) {
        let (window, client) = self.open().await;
        assert!(
            crate::settle_until(async || subjects(&window).len() == 5).await,
            "the five conversations never reached the screen: {:?}",
            subjects(&window)
        );
        (window, client)
    }
}
