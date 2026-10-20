//! A sender's pin, markers and digest are kept on the correspondent, and the
//! engine reads them beside the file (specs/011-focus-contacts S3, ADR 0048).

use std::sync::{Arc, Mutex};

use postio_config::{Config, DigestRule, Due, FocusConfig};
use postio_model::EmailAddress;
use postio_session::focus::FocusSettings;
use postio_storage::repository::ContactRepository;
use postio_storage::{Store, test_support};

/// A Focus mode installed over a `config.toml` holding `text`, with the last
/// configuration it published.
struct Setup {
    settings: FocusSettings,
    database: Store,
    path: std::path::PathBuf,
    published: Arc<Mutex<Option<FocusConfig>>>,
    _dir: tempfile::TempDir,
}

async fn setup(text: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    let database = test_support::memory().await;
    let settings = FocusSettings::default();
    let published = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&published);
    settings.install(
        Some(path.clone()),
        Config::from_toml_str(text).unwrap().focus,
        Arc::new(move |written: &FocusConfig| {
            *seen.lock().unwrap() = Some(written.clone());
        }),
    );
    settings.refresh(&database).await.expect("the rules load");
    Setup {
        settings,
        database,
        path,
        published,
        _dir: dir,
    }
}

fn at(address: &str) -> EmailAddress {
    EmailAddress::new(None::<String>, address)
}

fn ledger() -> DigestRule {
    DigestRule::from_due(
        "Ledger",
        vec!["from:News@Ledger.example".into()],
        Due::Weekly {
            day: chrono::Weekday::Sat,
            at: chrono::NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
        },
    )
}

#[tokio::test]
async fn a_pin_on_an_address_lands_on_the_person_and_both_the_verbs_and_the_pass_read_it() {
    let s = setup("").await;
    assert!(
        s.settings
            .pin(&s.database, "Ada@Example.org", true)
            .await
            .unwrap()
    );

    assert!(
        s.settings
            .config()
            .unwrap()
            .filter
            .never_filters(&at("ada@example.org"))
    );
    let published = s
        .published
        .lock()
        .unwrap()
        .clone()
        .expect("the pass was told");
    assert!(published.filter.never_filters(&at("ada@example.org")));
    assert!(
        !std::fs::read_to_string(&s.path)
            .unwrap()
            .contains("ada@example.org"),
        "an address is not written to the file any more"
    );
    assert!(
        !s.settings
            .pin(&s.database, "ada@example.org", true)
            .await
            .unwrap(),
        "already so"
    );

    assert!(
        s.settings
            .pin(&s.database, "ada@example.org", false)
            .await
            .unwrap()
    );
    assert!(
        !s.settings
            .config()
            .unwrap()
            .filter
            .never_filters(&at("ada@example.org"))
    );
}

#[tokio::test]
async fn a_domain_stays_in_the_file() {
    let s = setup("").await;
    assert!(
        s.settings
            .pin(&s.database, "@example.net", true)
            .await
            .unwrap()
    );
    assert!(
        std::fs::read_to_string(&s.path)
            .unwrap()
            .contains("@example.net")
    );
    assert!(
        s.settings
            .config()
            .unwrap()
            .filter
            .never_filters(&at("x@example.net"))
    );
}

#[tokio::test]
async fn taking_a_pin_back_clears_it_wherever_it_was_kept() {
    let s = setup("[focus.filter]\nnever = [\"ada@example.org\"]\n").await;
    s.settings
        .pin(&s.database, "bob@example.org", true)
        .await
        .unwrap();

    s.settings
        .pin(&s.database, "ada@example.org", false)
        .await
        .unwrap();
    s.settings
        .pin(&s.database, "bob@example.org", false)
        .await
        .unwrap();

    let config = s.settings.config().unwrap();
    assert!(
        !config.filter.never_filters(&at("ada@example.org")),
        "the file's entry"
    );
    assert!(
        !config.filter.never_filters(&at("bob@example.org")),
        "the person's"
    );
}

#[tokio::test]
async fn a_pin_for_someone_with_a_digest_stays_in_the_file_so_both_still_apply() {
    let s = setup("").await;
    s.settings
        .put_digest(&s.database, None, &ledger(), None)
        .await
        .unwrap();

    s.settings
        .pin(&s.database, "news@ledger.example", true)
        .await
        .unwrap();

    let config = s.settings.config().unwrap();
    assert!(config.filter.never_filters(&at("news@ledger.example")));
    assert_eq!(
        config.applicable_digests().len(),
        1,
        "and the digest is still theirs"
    );
    assert!(
        std::fs::read_to_string(&s.path)
            .unwrap()
            .contains("news@ledger.example")
    );
}

#[tokio::test]
async fn a_marker_turned_off_for_an_address_is_the_persons() {
    let s = setup("").await;
    assert!(
        s.settings
            .stop_marker(&s.database, "ada@example.org", "todo", true)
            .await
            .unwrap()
    );
    let config = s.settings.config().unwrap();
    assert!(config.filter.stops(&at("ada@example.org"), "todo"));
    assert!(!config.filter.stops(&at("ada@example.org"), "question"));
    assert!(
        !std::fs::read_to_string(&s.path)
            .unwrap()
            .contains("ada@example.org")
    );

    assert!(
        s.settings
            .stop_marker(&s.database, "ada@example.org", "todo", false)
            .await
            .unwrap()
    );
    assert!(
        !s.settings
            .config()
            .unwrap()
            .filter
            .stops(&at("ada@example.org"), "todo")
    );
}

#[tokio::test]
async fn a_digest_for_one_sender_is_the_persons_and_keeps_its_name() {
    let s = setup("").await;
    s.settings
        .put_digest(&s.database, None, &ledger(), None)
        .await
        .unwrap();

    let config = s.settings.config().unwrap();
    let (rule, due) = config.applicable_digests().remove(0);
    assert_eq!(
        rule.name, "Ledger",
        "the held mail is filed under this name"
    );
    assert_eq!(rule.queries, ["from:news@ledger.example"]);
    assert!(matches!(due, Due::Weekly { .. }));
    assert!(
        !std::fs::read_to_string(&s.path).unwrap().contains("Ledger"),
        "not in the file"
    );

    // Edited under the same name, then taken back with what it was kept as.
    let mut later = ledger();
    later.at = "18:30".into();
    s.settings
        .put_digest(&s.database, Some("Ledger"), &later, None)
        .await
        .unwrap();
    assert_eq!(
        s.settings.config().unwrap().digests.len(),
        1,
        "replaced, not added"
    );
    let (_, kept) = s
        .settings
        .remove_digest(&s.database, "Ledger")
        .await
        .unwrap()
        .expect("it was there");
    assert_eq!(kept.at, "18:30");
    assert!(s.settings.config().unwrap().digests.is_empty());
    s.settings
        .put_digest(&s.database, Some("Ledger"), &kept, Some(0))
        .await
        .unwrap();
    assert_eq!(
        s.settings.config().unwrap().digests.len(),
        1,
        "and put back"
    );
}

#[tokio::test]
async fn a_rule_that_is_not_one_senders_stays_in_the_file() {
    let s = setup("").await;
    let lists = DigestRule::from_due(
        "Lists",
        vec!["list:dev.lists.example".into()],
        Due::Daily {
            at: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        },
    );
    s.settings
        .put_digest(&s.database, None, &lists, None)
        .await
        .unwrap();
    assert!(std::fs::read_to_string(&s.path).unwrap().contains("Lists"));
    assert_eq!(s.settings.config().unwrap().digests.len(), 1);
}

#[tokio::test]
async fn the_rules_the_database_holds_reach_the_engine_when_it_starts() {
    let s = setup("").await;
    s.settings
        .pin(&s.database, "ada@example.org", true)
        .await
        .unwrap();

    // A new process: the same store, a fresh Focus mode.
    let again = FocusSettings::default();
    again.install(None, FocusConfig::default(), Arc::new(|_| {}));
    assert!(
        !again
            .config()
            .unwrap()
            .filter
            .never_filters(&at("ada@example.org"))
    );
    again.refresh(&s.database).await.unwrap();
    assert!(
        again
            .config()
            .unwrap()
            .filter
            .never_filters(&at("ada@example.org"))
    );
    let connection = s.database.connect().await.unwrap();
    assert!(
        ContactRepository::new(&connection)
            .by_address("ada@example.org")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_treatments_are_read_with_the_rules_so_a_reader_starts_from_memory() {
    let s = setup("").await;
    {
        let connection = s.database.connect().await.unwrap();
        let ada = ContactRepository::new(&connection)
            .person_for_email(&at("ada@example.org"))
            .await
            .unwrap();
        postio_storage::repository::ContactSettingsRepository::new(&connection)
            .put(
                ada,
                &postio_model::ContactSettings {
                    html_treatment: Some("paper".into()),
                    ..postio_model::ContactSettings::default()
                },
            )
            .await
            .unwrap();
    }
    assert!(s.settings.treatments().is_empty());
    s.settings.refresh(&s.database).await.unwrap();
    assert_eq!(
        s.settings.treatments(),
        [("ada@example.org".to_owned(), "paper".to_owned())]
    );
}
