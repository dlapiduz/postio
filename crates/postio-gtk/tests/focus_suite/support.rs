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

/// Dropped outside every runtime: a host owns a runtime of its own, which may not be dropped from
/// inside another's `block_on`.
pub fn keep<T: 'static>(value: T) {
    KEPT.with(|kept| kept.borrow_mut().push(Box::new(value)));
}

pub fn drop_kept() {
    let kept = KEPT.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    drop(kept);
}

pub fn display() -> bool {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    true
}

pub struct Fixture {
    pub database: Store,
    pub account: Account,
    pub inbox: MailboxId,
    blobs: tempfile::TempDir,
}

pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 14, 9, 0)
        .single()
        .expect("a fixed, valid date")
}

impl Fixture {
    pub async fn empty() -> Fixture {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        // The search index is part of opening a store; without it every search a case reaches
        // fails.
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the search index");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Archive").await;
        test_support::mailbox(&connection, &account, "Trash").await;
        // A store that has synced before, so an empty inbox reads as empty.
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

    pub async fn file_from_ada(
        &self,
        subject: &str,
        preview: &str,
        minutes: i64,
    ) -> (MessageId, ThreadId) {
        self.file(("Ada Moreno", "ada@example.com"), subject, preview, minutes)
            .await
    }

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

pub struct NoAccount {
    pub database: Store,
    blobs: tempfile::TempDir,
}

impl NoAccount {
    pub async fn new() -> NoAccount {
        NoAccount {
            database: test_support::memory().await,
            blobs: tempfile::tempdir().expect("a blob directory"),
        }
    }

    /// Overrides the secrets store as well as the backend: `Wiring::new`'s default reaches the
    /// platform keyring, which no default-suite test may touch.
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

pub async fn three_in_the_inbox() -> (Fixture, postio_gtk::window::FocusWindow) {
    use gtk::prelude::*;
    let fixture = Fixture::empty().await;
    fixture
        .file_from_ada("Atlas budget", "The numbers are attached.", 30)
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

    pub async fn open(&self) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        self.open_sized(None).await
    }

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
    pub async fn index(&self) {
        let connection = self.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("indexed");
    }

    pub async fn correspondent(&self, name: &str, address: &str) {
        let connection = self.database.connect().await.expect("a connection");
        postio_storage::repository::ContactRepository::new(&connection)
            .create(Some(name), &[EmailAddress::new(Some(name), address)])
            .await
            .expect("a correspondent");
    }

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

    pub async fn folder(&self, name: &str) -> MailboxId {
        let connection = self.database.connect().await.expect("a connection");
        test_support::mailbox(&connection, &self.account, name)
            .await
            .id
    }

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

    pub async fn read_first(&self, thread: &[MessageId], count: usize) {
        let connection = self.database.connect().await.expect("a connection");
        let mut seen = postio_model::FlagSet::new();
        seen.insert(postio_model::Flag::Seen);
        for message in &thread[..count.min(thread.len())] {
            MessageRepository::new(&connection)
                .set_flags(
                    *message,
                    &seen,
                    postio_storage::repository::FlagSource::Server,
                )
                .await
                .expect("the message is read");
        }
    }

    pub async fn read_all_but_last(&self, thread: &[MessageId]) {
        let connection = self.database.connect().await.expect("a connection");
        let mut seen = postio_model::FlagSet::new();
        seen.insert(postio_model::Flag::Seen);
        for message in &thread[..thread.len().saturating_sub(1)] {
            MessageRepository::new(&connection)
                .set_flags(
                    *message,
                    &seen,
                    postio_storage::repository::FlagSource::Server,
                )
                .await
                .expect("the earlier message is read");
        }
    }

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

    pub async fn refuse_body(&self, message: MessageId) {
        let connection = self.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .mark_body_failed(message)
            .await
            .expect("a mark");
    }

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

pub fn press(window: &postio_gtk::window::FocusWindow, name: &str, state: gtk::gdk::ModifierType) {
    deliver_with(window, name, state);
}

pub async fn open_hit(
    window: &postio_gtk::window::FocusWindow,
    words: &str,
    subject: &str,
) -> std::rc::Rc<postio_gtk::bar::Bar> {
    press(window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    for end in 1..=words.chars().count() {
        bar.set_text(&words.chars().take(end).collect::<String>());
        crate::settle();
    }
    assert!(
        crate::settle_until(async || bar.result_subjects().iter().any(|hit| hit == subject)).await,
        "no hit for {subject:?} under {words:?}: {:?}",
        bar.texts()
    );
    let on_it = || {
        bar.highlighted()
            .is_some_and(|(kind, text)| kind == "message" && text.contains(subject))
    };
    for _ in 0..8 {
        if on_it() {
            break;
        }
        press(window, "Up", gtk::gdk::ModifierType::empty());
    }
    assert!(
        on_it(),
        "the arrows never reached {subject:?}: {:?}",
        bar.highlighted()
    );
    press(window, "Return", gtk::gdk::ModifierType::empty());
    assert!(
        crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
        "the hit never opened"
    );
    bar
}

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
                .file_from_ada(subject, "A line of text.", minutes)
                .await;
            filed.push(message);
        }
        filed
    }

    pub async fn five(&self) -> (postio_gtk::window::FocusWindow, postio_client::Client) {
        self.file_five().await;
        self.open_five().await
    }

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

    pub async fn label_in(&self, message: MessageId, name: &str, colour: &str) {
        let connection = self.database.connect().await.expect("a connection");
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        let mut label = postio_model::Label::new(self.account.id, name);
        label.color = Some(colour.to_owned());
        labels.create(&mut label).await.expect("a label");
        labels.attach(message, label.id).await.expect("attached");
    }
}

/// Press `name` as GTK delivers a key: to the widget the keyboard is on, through each key
/// controller from the window down in the capture phase, then the target's own, then back up in the
/// bubble phase, until one claims it. [`press`] calls the window's handler directly, which cannot
/// see a controller taking the key first. A widget class's own key bindings cannot run: GTK offers
/// no way but a real event.
pub fn deliver(window: &postio_gtk::window::FocusWindow, name: &str) -> bool {
    let state = if name.chars().count() == 1 && name.chars().all(char::is_uppercase) {
        gtk::gdk::ModifierType::SHIFT_MASK
    } else {
        gtk::gdk::ModifierType::empty()
    };
    deliver_with(window, name, state)
}

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
    // GTK's key path climbs from the focus through its parents. A focus that left the window (its
    // row was destroyed in a redraw) has none, so a person's keyboard is dead until a click: say so
    // rather than let the key find nothing.
    assert!(
        target.root().is_some(),
        "{name}: the keyboard is on a {} that has left the window",
        target.type_().name()
    );
    // GTK runs a key through the widgets from the focus up to the innermost dialog presented over
    // the window, and no further (measured with real key presses in the headless compositor, GTK
    // 4.22, libadwaita 1.9).
    let mut chain = vec![target.clone()];
    while !chain
        .last()
        .is_some_and(|widget| widget.is::<adw::Dialog>())
        && let Some(parent) = chain.last().and_then(|widget| widget.parent())
    {
        chain.push(parent);
    }
    // A widget's own `ShortcutController` of local scope answers in its phase: a key whose trigger
    // matches runs the action on the widget.
    let shortcut = |widget: &gtk::Widget, shortcuts: &gtk::ShortcutController| -> bool {
        (0..shortcuts.n_items()).any(|at| {
            let Some(shortcut) = shortcuts.item(at).and_downcast::<gtk::Shortcut>() else {
                return false;
            };
            let Some(trigger) = shortcut
                .trigger()
                .and_then(|trigger| trigger.downcast::<gtk::KeyvalTrigger>().ok())
            else {
                return false;
            };
            trigger.keyval() == key
                && trigger.modifiers() == state
                && shortcut.action().is_some_and(|action| {
                    action.activate(gtk::ShortcutActionFlags::empty(), widget, None)
                })
        })
    };
    let fire = |widget: &gtk::Widget, phase: gtk::PropagationPhase| -> bool {
        let controllers = widget.observe_controllers();
        (0..controllers.n_items()).any(|at| {
            let Some(controller) = controllers.item(at).and_downcast::<gtk::EventController>()
            else {
                return false;
            };
            if controller.propagation_phase() != phase {
                return false;
            }
            if let Some(keys) = controller.downcast_ref::<gtk::EventControllerKey>() {
                return keys.emit_by_name::<bool>("key-pressed", &[&keyval, &0u32, &state]);
            }
            controller
                .downcast_ref::<gtk::ShortcutController>()
                .is_some_and(|shortcuts| {
                    shortcuts.scope() == gtk::ShortcutScope::Local && shortcut(widget, shortcuts)
                })
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

/// A list still redrawing replaces its rows, so a row found a moment ago may be gone: this finds
/// one afresh until a click at its middle reaches it.
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
            wait_to_be_still(&native, &row);
            // Mapped is not laid out: a row built a moment ago has no size until the next frame.
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

pub fn click_in(
    window: &postio_gtk::window::FocusWindow,
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    x: f32,
    y: f32,
    n_press: i32,
) {
    let native = native_of(widget.as_ref(), window);
    wait_to_be_still(&native, widget.as_ref());
    let (x, y) = wait_to_be_pickable(&native, widget.as_ref(), x, y);
    click_on(&native, x, y, n_press);
}

pub fn click(
    window: &postio_gtk::window::FocusWindow,
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    n_press: i32,
) {
    use gtk::prelude::*;
    let widget = widget.as_ref();
    // A widget a moment old has no size yet, and a click at its corner can miss it.
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !(widget.is_mapped() && widget.width() > 0 && widget.height() > 0)
        && std::time::Instant::now() < deadline
    {
        crate::settle();
    }
    let native = native_of(widget, window);
    wait_to_be_still(&native, widget);
    let (width, height) = (widget.width() as f32, widget.height() as f32);
    let (x, y) = wait_to_be_pickable(&native, widget, width / 2.0, height / 2.0);
    click_on(&native, x, y, n_press);
}

pub fn secondary_click(
    window: &postio_gtk::window::FocusWindow,
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
) {
    use gtk::prelude::*;
    let widget = widget.as_ref();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !(widget.is_mapped() && widget.width() > 0 && widget.height() > 0)
        && std::time::Instant::now() < deadline
    {
        crate::settle();
    }
    let native = native_of(widget, window);
    wait_to_be_still(&native, widget);
    let (width, height) = (widget.width() as f32, widget.height() as f32);
    let (x, y) = wait_to_be_pickable(&native, widget, width / 2.0, height / 2.0);
    press_on(&native, x, y, 1, gtk::gdk::BUTTON_SECONDARY);
}

fn native_of(widget: &gtk::Widget, window: &postio_gtk::window::FocusWindow) -> gtk::Widget {
    use gtk::prelude::*;
    widget
        .native()
        .map(|native| native.upcast())
        .unwrap_or_else(|| window.clone().upcast())
}

const DRAWN_AT_SIZE: f32 = 0.01;

/// Waits for `widget` to stop moving on `native`: libadwaita opens a dialog by scaling it from 0.8
/// to 1, so a press and a release a frame later land at two places (#1817). Whether `widget` is
/// drawn at its own size now: a stalled frame clock draws no frames and stalls an opening dialog's
/// scale too, so "no frames" alone clicked Settings at 0.81 of its size (#1817).
fn drawn_at_own_size(native: &gtk::Widget, widget: &gtk::Widget) -> bool {
    use gtk::prelude::*;
    match (widget.compute_bounds(native), widget.compute_bounds(widget)) {
        (Some(drawn), Some(own)) => {
            (drawn.width() - own.width()).abs() < DRAWN_AT_SIZE
                && (drawn.height() - own.height()).abs() < DRAWN_AT_SIZE
        }
        _ => true,
    }
}

pub fn wait_for_dialog_opened(
    window: &postio_gtk::window::FocusWindow,
    dialog: &impl gtk::prelude::IsA<adw::Dialog>,
) {
    use adw::prelude::*;
    let content = dialog.as_ref().child().expect("the dialog's content");
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !(content.is_mapped() && content.width() > 0) && std::time::Instant::now() < deadline {
        crate::settle();
    }
    let native = native_of(&content, window);
    wait_to_be_still(&native, &content);
    assert!(
        drawn_at_own_size(&native, &content),
        "the dialog never finished opening"
    );
}

fn wait_to_be_still(native: &gtk::Widget, widget: &gtk::Widget) {
    use gtk::prelude::*;
    type Frames = Vec<Option<gtk::graphene::Rect>>;
    let frames: std::rc::Rc<std::cell::RefCell<Frames>> = std::rc::Rc::default();
    let tick = widget.add_tick_callback({
        let frames = std::rc::Rc::clone(&frames);
        let native = native.clone();
        move |widget, _| {
            frames.borrow_mut().push(widget.compute_bounds(&native));
            gtk::glib::ControlFlow::Continue
        }
    });
    let at_rest = |widget: &gtk::Widget| {
        let frames = frames.borrow();
        let [.., Some(before), Some(last)] = frames.as_slice() else {
            return false;
        };
        // Its own size: its box in its own space, which no transform scales (`width()` is the
        // content box, so not the thing to compare).
        let Some(own) = widget.compute_bounds(widget) else {
            return false;
        };
        before == last
            && (last.width() - own.width()).abs() < DRAWN_AT_SIZE
            && (last.height() - own.height()).abs() < DRAWN_AT_SIZE
    };
    let started = std::time::Instant::now();
    let deadline = started + postio_test_support::scaled(std::time::Duration::from_secs(10));
    // A frame comes every few milliseconds on a drawn surface; none in this long means it is not
    // drawn, so there is no motion to wait out.
    let first_frames = started + postio_test_support::scaled(std::time::Duration::from_secs(1));
    loop {
        crate::settle();
        // A widget that left the screen draws no frames (a redrawn list replaces its rows) and is
        // not one to wait on.
        if at_rest(widget)
            || !widget.is_mapped()
            || (frames.borrow().len() < 2
                && std::time::Instant::now() > first_frames
                && drawn_at_own_size(native, widget))
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            assert!(
                frames.borrow().len() < 2,
                "the {} never came to rest on screen: {:?}",
                widget.type_().name(),
                frames.borrow().iter().rev().take(3).collect::<Vec<_>>()
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    tick.remove();
}

/// A dialog slides in, and until it stops the point is over the scrim, not the button. Panics
/// naming what covers it when it never is: a button under something is a bug.
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

/// GTK offers no way to build a button event in-process, so this drives the gestures' `pressed` and
/// `released` signals in GTK's order and along its path, to whatever is picked there (a widget
/// under a dialog's scrim, unmapped, clipped or insensitive cannot be clicked), no further than the
/// innermost dialog or popover surface. A `GtkButton` claims what reaches it, and nothing above one
/// is run.
pub fn click_at(window: &postio_gtk::window::FocusWindow, x: f64, y: f64, n_press: i32) {
    use gtk::prelude::*;
    click_on(window.upcast_ref::<gtk::Widget>(), x, y, n_press);
}

fn click_on(window: &gtk::Widget, x: f64, y: f64, n_press: i32) {
    press_on(window, x, y, n_press, gtk::gdk::BUTTON_PRIMARY);
}

fn press_on(window: &gtk::Widget, x: f64, y: f64, n_press: i32, button: u32) {
    use gtk::prelude::*;
    let picked = window
        .pick(x, y, gtk::PickFlags::DEFAULT)
        .unwrap_or_else(|| window.clone().upcast());
    // A popover is a surface of its own: an event on it goes up to the popover and no further.
    // A dialog is not, so the window's capture gestures see a press on it first.
    let mut chain = vec![picked.clone()];
    while !chain
        .last()
        .is_some_and(|widget| widget.is::<gtk::Native>())
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
            // A gesture for another button never sees it.
            .filter(|gesture| gesture.button() == 0 || gesture.button() == button)
            .collect()
    };
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

pub const ATTACHED: &[u8] = b"%PDF-1.4\n\x00\xff\x10 the plan\r\n";
const ATTACHED_BASE64: &str = "JVBERi0xLjQKAP8QIHRoZSBwbGFuDQo=";

/// A settings detail page slides in when its row is clicked, carrying its buttons, and a press and
/// a release at two places are no click: on a slow runner "Add signature" was pressed mid-slide.
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

pub async fn rows_shown(window: &postio_gtk::window::FocusWindow, count: usize) {
    assert!(
        crate::settle_until(async || subjects(window).len() == count).await,
        "the inbox never reached {count} rows"
    );
}
