use postio_ffi::{ProviderHintFfi, RouteFfi, Session, SessionOptions, provider_hint};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

async fn a_session() -> (std::sync::Arc<Session>, postio_storage::Store) {
    let database = test_support::memory().await;
    // A keyring of its own, or the suite writes into the developer's login keychain.
    let secrets = std::sync::Arc::new(postio_account::secret::MemorySecretStore::default());
    let session =
        Session::open(SessionOptions::in_memory_with(database.clone()).with_secrets(secrets))
            .expect("a session over the store");
    (session, database)
}

/// Read from the preset table: a test naming a provider is the mistake the code is forbidden to
/// make.
async fn an_address_at_a_known_provider() -> (String, String) {
    let preset = postio_account::discovery::presets()
        .first()
        .expect("the preset table ships at least one provider");
    let domain = preset
        .domains()
        .first()
        .expect("a preset claims at least one domain")
        .clone();
    (
        format!("someone@{domain}"),
        preset.display_name().to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_at_a_known_provider_is_recognised_and_says_which() {
    let (address, provider) = an_address_at_a_known_provider().await;
    let hint = provider_hint(address);

    assert_eq!(hint.provider, provider, "the strip names what it found");
    assert!(
        hint.verdict.contains(&provider),
        "and says so in a sentence: {}",
        hint.verdict
    );
    assert!(!hint.imap_host.is_empty(), "and it knows where to connect");
}

#[test]
fn an_address_nobody_recognises_falls_to_imap_and_says_so() {
    // A self-hosted domain is not a failure, and must not guess `imap.<domain>`, which dials
    // somebody else's server.
    let hint = provider_hint("ada@ostwald.invalid".to_owned());

    assert_eq!(hint.route, RouteFfi::Imap);
    assert!(
        hint.imap_host.is_empty(),
        "no host is guessed from the domain"
    );
    assert!(
        !hint.verdict.is_empty(),
        "a strip that says nothing leaves the user with no idea what happens next"
    );
}

#[test]
fn something_that_is_not_an_address_is_not_recognised_as_anything() {
    let hint = provider_hint("not-an-address".to_owned());
    assert_eq!(hint.route, RouteFfi::Imap);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hint_is_a_record_the_sheet_can_draw_without_asking_again() {
    let (address, _) = an_address_at_a_known_provider().await;
    let hint: ProviderHintFfi = provider_hint(address);
    assert!(hint.imap_port > 0);
    assert!(hint.smtp_port > 0);
    assert!(!hint.provider.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_without_a_client_id_says_why_rather_than_failing_obscurely() {
    // ADR 0006 Q1: a client id inside an open-source application is shared by every user and
    // revoked for all at once.
    let (session, _) = a_session().await;

    let complaint = session
        .sign_in_with_browser("mara@example.com".to_owned(), "  ".to_owned(), None)
        .await
        .expect("a refusal");

    assert!(complaint.contains("client id"), "{complaint}");
    assert!(
        complaint.contains("shares") || complaint.contains("ships none"),
        "and says why: {complaint}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_with_no_browser_sign_in_says_so_and_names_the_way_in() {
    let (session, _) = a_session().await;

    let complaint = session
        .sign_in_with_browser(
            "ada@ostwald.invalid".to_owned(),
            "the-users-own-client".to_owned(),
            None,
        )
        .await
        .expect("a refusal");

    assert!(complaint.contains("IMAP"), "{complaint}");
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_in_flight_before_a_sign_in_starts() {
    let (session, _) = a_session().await;
    let progress = session.sign_in_progress();

    assert!(!progress.waiting);
    assert_eq!(progress.port, 0);
    assert!(progress.message.is_empty(), "and it says nothing at all");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_when_nothing_is_signing_in_is_harmless() {
    let (session, _) = a_session().await;
    session.cancel_sign_in();
    assert!(!session.sign_in_progress().waiting);
}

#[test]
fn the_scopes_are_stated_in_plain_words_and_so_is_what_is_not_asked_for() {
    // Only the application can say what it deliberately left out of a consent screen.
    let scopes = postio_ffi::sign_in_scopes("someone@example.com".to_owned());

    assert!(scopes.asked_for.to_lowercase().contains("mail"));
    for absent in ["contacts", "calendar", "files"] {
        assert!(
            scopes.not_asked_for.to_lowercase().contains(absent),
            "{absent} is not named: {}",
            scopes.not_asked_for
        );
    }
}

#[test]
fn a_provider_that_offers_a_browser_sign_in_reports_the_scopes_its_row_asks_for() {
    // From the provider's table row, not a Swift list, so there is one answer to what was consented
    // to.
    let Some((domain, expected)) = postio_account::discovery::presets()
        .iter()
        .find_map(|preset| {
            let offer = preset.oauth()?;
            let domain = preset.domains().first()?.clone();
            (!offer.scopes.is_empty()).then(|| (domain, offer.scopes.clone()))
        })
    else {
        panic!("the preset table ships no provider with OAuth scopes");
    };

    let scopes = postio_ffi::sign_in_scopes(format!("someone@{domain}"));
    assert_eq!(scopes.requested, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_with_no_mail_weighs_nothing_and_says_nothing() {
    let (session, _) = a_session().await;
    assert_eq!(session.account_weight(1).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_with_mail_says_how_much_of_it_is_on_this_disk() {
    use chrono::Utc;
    use postio_model::Message;
    use postio_storage::repository::MessageRepository;

    let database = postio_storage::test_support::memory().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        for _ in 0..3 {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.size = 400_000;
            message.sync.body_state = postio_model::message::BodyState::Full;
            repository.create(&mut message).await.expect("a message");
        }
        account.id.get()
    };
    let session = Session::open(SessionOptions::in_memory_with(database).with_secrets(
        std::sync::Arc::new(postio_account::secret::MemorySecretStore::default()),
    ))
    .expect("a session");

    let said = session
        .account_weight(account)
        .await
        .expect("three messages weigh something");

    assert!(said.contains("downloaded"), "{said}");
    assert!(said.contains("MB") || said.contains("KB"), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn weighing_an_account_is_a_fixed_handful_of_statements_not_one_per_message() {
    // Aggregates per account are fine; a query per message hangs the settings window on a large
    // store.
    // Counted at `MessageRepository::footprint`: the session's pooled connections have
    // per-connection trace hooks, so counting through it measures nothing.
    use chrono::Utc;
    use postio_model::Message;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support::counting;

    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
    let repository = MessageRepository::new(&connection);
    for _ in 0..50 {
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.size = 1_000;
        repository.create(&mut message).await.expect("a message");
    }

    counting::install(&connection);
    // `counted_async`: in the sync form a `let _ =` on a future drops it unrun and the counter
    // stays at zero.
    let counts = counting::counted_async(|| async {
        MessageRepository::new(&connection)
            .footprint(account.id)
            .await
            .expect("a footprint");
    })
    .await;

    assert!(
        counts.statements <= 8,
        "weighing an account took {} statements over 50 messages; that is \
         per-message work in a window that opens on a whole store",
        counts.statements
    );
}

fn a_maildir() -> tempfile::TempDir {
    let tree = tempfile::tempdir().expect("a temporary directory");
    for subdir in ["cur", "new", "tmp"] {
        std::fs::create_dir_all(tree.path().join(subdir)).expect("a maildir subdirectory");
    }
    std::fs::write(
        tree.path().join("cur").join("1770000000.host:2,S"),
        "Subject: already here\r\n\r\nbody\r\n",
    )
    .expect("a delivered message");
    tree
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_that_is_not_a_mail_store_is_refused_before_anything_is_written() {
    let (session, database) = a_session().await;
    let tree = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(tree.path().join("holiday.jpg"), "not mail").expect("a file");
    let path = tree.path().display().to_string();

    let complaint = session
        .inspect_local_store(path.clone())
        .expect("that is not a mail store");
    assert!(
        complaint.contains(&path),
        "the complaint names the directory: {complaint}"
    );

    assert!(
        session
            .add_local_account("ada@example.com".to_owned(), path)
            .await
            .is_some()
    );
    let connection = database.connect().await.expect("checkout");
    assert!(
        AccountRepository::new(&connection)
            .list_enabled()
            .await
            .expect("read")
            .is_empty(),
        "nothing was written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_maildir_becomes_an_account_with_no_credential_anywhere() {
    let (session, database) = a_session().await;
    let tree = a_maildir();
    let path = tree.path().display().to_string();

    assert_eq!(
        session.inspect_local_store(path.clone()),
        None,
        "a maildir is a store Postio can open"
    );
    assert_eq!(
        session
            .add_local_account("ada@example.com".to_owned(), path.clone())
            .await,
        None,
        "and adding it needs nothing else — no password, no server"
    );

    let connection = database.connect().await.expect("checkout");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
        .await
        .expect("read");
    assert_eq!(accounts.len(), 1);
    assert_eq!(
        accounts[0].backend,
        postio_model::account::Backend::Maildir { root: path },
        "the account is the directory"
    );
}
