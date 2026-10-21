//! Syncing with Contacts across the boundary, as a dry run only
//! (specs/011-focus-contacts S12). The Mac hands over a snapshot of its side;
//! Rust says what a sync would do. Nothing here is an address book: the
//! snapshot is a fake, and nothing is written to either side.

use postio_ffi::{ExternalRecordFfi, Session, SessionOptions, SyncScopeFfi};
use postio_model::{ContactForm, FormAddress};
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

fn theirs(id: &str, given: &str, address: &str) -> ExternalRecordFfi {
    ExternalRecordFfi {
        id: id.to_owned(),
        given: given.to_owned(),
        family: String::new(),
        emails: vec![address.to_owned()],
        phones: Vec::new(),
        organization: String::new(),
        title: String::new(),
    }
}

async fn session_with_grace() -> (std::sync::Arc<Session>, postio_storage::Store) {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let mut form = ContactForm::blank();
        form.given = "Grace".into();
        form.addresses = vec![FormAddress {
            address: "grace@example.org".into(),
            ..FormAddress::blank()
        }];
        ContactRepository::new(&connection)
            .save_form(None, &form)
            .await
            .expect("a saved person");
    }
    (
        Session::open(SessionOptions::in_memory_with(database.clone())).expect("a session"),
        database,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_intro_asks_nothing_and_the_plan_says_what_would_happen() {
    let (session, _database) = session_with_grace().await;
    let intro = session.contacts_sync_intro(SyncScopeFfi::PeopleAndCompanies);
    assert_eq!(intro.title, "Sync with Contacts");
    assert!(intro.plan.is_none());
    assert_eq!(intro.primary.label, "Turn on and preview");

    let s = session.clone();
    let result = tokio::task::spawn_blocking(move || {
        s.contacts_sync_plan(
            vec![theirs("x1", "Ada", "ada@example.org")],
            SyncScopeFfi::PeopleAndCompanies,
        )
    })
    .await
    .expect("planned");
    let sheet = result.sheet.expect("a sheet");
    assert_eq!(sheet.title, "Dry run: nothing was written");
    let plan = sheet.plan.expect("the plan");
    assert_eq!(plan.summary, "Would add 1, update 0, ask about 0, delete 0");
    assert!(
        plan.lines.iter().any(|l| l == "Add Grace to Contacts"),
        "{:?}",
        plan.lines
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_changes_nothing_in_postio() {
    let (session, database) = session_with_grace().await;
    let before = state(&database).await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || {
        s.contacts_sync_plan(
            vec![
                theirs("x1", "Grace", "grace@example.org"),
                theirs("x2", "Zed", "zed@example.org"),
            ],
            SyncScopeFfi::Everything,
        )
    })
    .await
    .expect("planned");
    assert_eq!(state(&database).await, before);
    session.shutdown();
}

async fn state(database: &postio_storage::Store) -> (i64, i64) {
    let reader = database.read().await.expect("a reader");
    (
        postio_storage::sql::scalar(
            &reader,
            "SELECT count(*) * 1000003 + coalesce(sum(updated_at), 0) FROM contacts",
            (),
        )
        .await
        .unwrap(),
        postio_storage::sql::scalar(&reader, "SELECT count(*) FROM addresses", ())
            .await
            .unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_has_its_own_sheet() {
    let (session, _database) = session_with_grace().await;
    let denied = session.contacts_sync_denied();
    assert_eq!(denied.title, "Contacts access was declined");
    session.shutdown();
}
