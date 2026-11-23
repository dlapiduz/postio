use chrono::Utc;
use postio_model::test_corpus;
use postio_model::{BodyState, Message, MessageId};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support::{self, counting};
use postio_storage::{BlobStore, Store};
use postio_ui::source::RawSegment;

pub(crate) struct Scene {
    pub(crate) database: Store,
    pub(crate) blobs: BlobStore,
    pub(crate) account: postio_model::Account,
    inbox: postio_model::MailboxId,
    _dir: tempfile::TempDir,
}

impl Scene {
    pub(crate) async fn new() -> Scene {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        drop(connection);
        let dir = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(dir.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        Scene {
            database,
            blobs,
            account,
            inbox,
            _dir: dir,
        }
    }

    /// The fixture filed with its body and header block, and its raw bytes when `raw`.
    pub(crate) async fn file(&self, fixture: &str, raw: bool) -> MessageId {
        let bytes = test_corpus::load(fixture).bytes();
        let parsed = postio_model::mime::parse(bytes);
        let mut message = parsed.into_message(self.account.id, self.inbox, Utc::now());
        if raw {
            message.raw_blob_id = Some(self.blobs.put(bytes).expect("the raw source is kept"));
        }
        let connection = self.database.connect().await.expect("a connection");
        let repository = MessageRepository::new(&connection);
        let id = repository.create(&mut message).await.expect("a message");
        repository
            .set_body(
                id,
                &StoredBody {
                    text: Some("The body.".to_owned()),
                    html: None,
                    headers: postio_model::headers::block_of(bytes).map(|block| block.text),
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("a body");
        id
    }

    pub(crate) async fn file_envelope_only(&self) -> MessageId {
        let mut message = Message::new(self.account.id, self.inbox, Utc::now());
        message.subject = Some("Not downloaded".to_owned());
        let connection = self.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message")
    }
}

fn current_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

#[test]
fn a_local_source_opens_in_a_bounded_number_of_reads() {
    current_thread().block_on(async {
        let scene = Scene::new().await;
        let message = scene.file("attachment-large", true).await;
        let mut opened = None;
        let counts = counting::counted_async(|| async {
            opened = Some(
                postio_host::source::view(&scene.database, &scene.blobs, message, false)
                    .await
                    .expect("the source opens"),
            );
        })
        .await;
        let opened = opened.expect("opened");
        assert!(!opened.fetch, "the bytes are here: nothing to fetch");
        assert_eq!(opened.view.raw.state_line, None);
        assert!(
            opened
                .view
                .raw
                .segments
                .iter()
                .any(|segment| matches!(segment, RawSegment::Fold { .. })),
            "the large part is folded"
        );
        assert_eq!(
            counts.statements, 8,
            "keyed lookups only: the message row with its attachments, labels and recipients, \
             its body, and its account with identities and signatures: {counts:?}"
        );
        assert!(counts.rows <= 8, "no mailbox scan: {counts:?}");
    });
}

#[test]
fn absent_bytes_say_fetching_at_once_and_ask_for_the_fetch() {
    current_thread().block_on(async {
        let scene = Scene::new().await;
        let message = scene.file("auth-results-verified", false).await;
        let opened = postio_host::source::view(&scene.database, &scene.blobs, message, false)
            .await
            .expect("the source opens");
        assert!(opened.fetch, "the person asked for these bytes");
        assert_eq!(
            opened.view.raw.state_line.as_deref(),
            Some(format!("Fetching the message from {}…", scene.account.display_name).as_str())
        );
        let doubted = opened
            .view
            .verification
            .sections
            .iter()
            .find(|section| section.title == "Headers Postio does not believe")
            .expect("the stored header block still answers Verification");
        assert_eq!(doubted.lines.len(), 1, "{doubted:#?}");
    });
}

#[test]
fn offline_says_so_and_asks_for_nothing() {
    current_thread().block_on(async {
        let scene = Scene::new().await;
        let message = scene.file("auth-results-verified", false).await;
        let opened = postio_host::source::view(&scene.database, &scene.blobs, message, true)
            .await
            .expect("the source opens");
        assert!(!opened.fetch);
        assert_eq!(
            opened.view.raw.state_line.as_deref(),
            Some(
                format!(
                    "This message isn't on this computer yet. Postio will show it when {} is back online.",
                    scene.account.display_name
                )
                .as_str()
            )
        );
    });
}

#[test]
fn an_envelope_without_its_body_says_the_checks_arrive_with_it() {
    current_thread().block_on(async {
        let scene = Scene::new().await;
        let message = scene.file_envelope_only().await;
        let opened = postio_host::source::view(&scene.database, &scene.blobs, message, true)
            .await
            .expect("the source opens");
        assert_eq!(
            opened.view.verification.sections[0].intro.as_deref(),
            Some("The headers that carry these checks arrive with the message body.")
        );
        assert_eq!(
            opened.view.headers.note.as_deref(),
            Some("More headers arrive with the body.")
        );
    });
}

fn patience() -> std::time::Duration {
    postio_test_support::scaled(std::time::Duration::from_secs(120))
}

#[test]
fn a_source_fetched_from_the_server_arrives_and_shows_its_bytes() {
    use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
    use postio_account::test_server::{TestMailbox, TestServer};
    use postio_core::Event;
    use postio_model::fetch::Sections;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime for the server");
    let database = rt.block_on(test_support::memory());
    let dir = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(dir.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store");
    let secrets = std::sync::Arc::new(MemorySecretStore::new());
    let host = postio_host::Host::start(database.clone(), blobs, {
        let secrets = std::sync::Arc::clone(&secrets);
        move |wiring| wiring.with_secrets(secrets)
    })
    .expect("a host");
    let server = rt.block_on(
        TestServer::builder()
            .account("test@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(["auth-results-verified"]))
            .start(),
    );
    let mut account = postio_model::Account::new(
        "Test",
        postio_model::EmailAddress::new(None::<String>, "test@example.com"),
    );
    account.incoming.host = server.addr().ip().to_string();
    account.incoming.port = server.addr().port();
    account.incoming.security = postio_model::TransportSecurity::None;
    account.incoming.username = server.account().to_owned();
    rt.block_on(async {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("the account row");
        secrets
            .store(
                &AccountKey::new(account.address.address.clone()),
                &Password::new("hunter2"),
            )
            .await
            .expect("a password");
    });
    let client = host.connect(postio_client::protocol::ClientKind::Test);
    let events = client.events();
    host.start_syncing();

    let deadline = std::time::Instant::now() + patience();
    let message = loop {
        let found = rt.block_on(async {
            let connection = database.connect().await.ok()?;
            MessageRepository::new(&connection)
                .ids_by_rfc_message_id(
                    account.id,
                    &postio_model::RfcMessageId::new("<auth-verified-01@example.org>"),
                )
                .await
                .ok()?
                .into_iter()
                .next()
        });
        if let Some(found) = found {
            break found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the sync never filed the message"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    let first = rt
        .block_on(client.source_view(message, false))
        .expect("the source opens at once");
    if first.raw.segments.is_empty() {
        assert_eq!(
            first.raw.state_line.as_deref(),
            Some("Fetching the message from Test…")
        );
        let deadline = std::time::Instant::now() + patience();
        loop {
            match events.try_recv() {
                Ok(envelope) => {
                    if let Event::BodyLoaded {
                        message: loaded,
                        sections: Sections::Whole,
                        ..
                    } = envelope.event
                        && loaded == message
                    {
                        break;
                    }
                }
                Err(_) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the whole message never arrived"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
    }
    let shown = rt
        .block_on(client.source_view(message, false))
        .expect("the source opens again");
    assert_eq!(shown.raw.state_line, None);
    let text: String = shown
        .raw
        .segments
        .iter()
        .filter_map(|segment| match segment {
            RawSegment::Text(text) => Some(text.as_str()),
            RawSegment::Fold { .. } => None,
        })
        .collect();
    assert!(
        text.contains("Message-ID: <auth-verified-01@example.org>"),
        "{text}"
    );
    host.stop();
}

#[test]
fn source_surface_makes_no_connection() {
    use postio_model::test_corpus::Category;

    let listener = postio_test_support::listener::Listener::start();
    let rt = current_thread();
    let scene = rt.block_on(async {
        let scene = Scene::new().await;
        let connection = scene.database.connect().await.expect("a connection");
        let mut account = scene.account.clone();
        account.incoming.host = "127.0.0.1".to_owned();
        account.incoming.port = listener.port();
        postio_storage::repository::AccountRepository::new(&connection)
            .update(&mut account)
            .await
            .expect("the account points at the listener");
        scene
    });
    let mut fixtures: Vec<&str> = test_corpus::by_category(Category::Hostile)
        .into_iter()
        .chain(test_corpus::by_category(Category::PromptInjection))
        .map(|fixture| fixture.name())
        .collect();
    fixtures.sort_unstable();
    fixtures.dedup();
    assert!(
        fixtures.len() >= 10,
        "the hostile corpus is here: {fixtures:?}"
    );
    let host =
        postio_host::Host::start(scene.database.clone(), scene.blobs.clone(), |wiring| wiring)
            .expect("a host");
    let client = host.connect(postio_client::protocol::ClientKind::Test);
    for fixture in &fixtures {
        let message = rt.block_on(scene.file(fixture, true));
        let view = rt
            .block_on(client.source_view(message, false))
            .unwrap_or_else(|error| panic!("{fixture}: {error:?}"));
        assert!(
            !view.raw.segments.is_empty() && !view.headers.rows.is_empty(),
            "{fixture}: every tab has its text"
        );
    }
    host.stop();
    listener.control();
    assert_eq!(
        listener.count(),
        1,
        "only the control's own connection: opening the surface reached the network"
    );
}

#[test]
fn the_source_surface_names_no_renderer() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let widgets = root.join("crates/postio-widgets/src/source");
    let mut files = vec![root.join("crates/postio-ui/src/source.rs")];
    files.extend(
        std::fs::read_dir(&widgets)
            .unwrap_or_else(|error| {
                panic!("the surface's widgets in {}: {error}", widgets.display())
            })
            .map(|entry| entry.expect("a directory entry").path()),
    );
    for file in files {
        let text = std::fs::read_to_string(&file).expect("readable source");
        for refused in [
            "postio_render",
            "webkit",
            "WebView",
            "LinkButton",
            "use_markup",
            "set_markup",
        ] {
            assert!(
                !text.contains(refused),
                "{} names {refused}: the Source surface is text, never rendered or linked",
                file.display()
            );
        }
    }
}
