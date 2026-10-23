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
            postio_ffi::SyncModeFfi::TwoWay,
        )
    })
    .await
    .expect("planned");
    let sheet = result.sheet.expect("a sheet");
    assert_eq!(sheet.title, "Dry run: nothing was written");
    let plan = sheet.plan.expect("the plan");
    assert_eq!(
        plan.summary,
        "Would bring in 1, add 1, update 0, ask about 0, delete 0"
    );
    assert_eq!(
        plan.bring_in.expect("a button").label,
        "Bring 1 into Postio"
    );
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
            postio_ffi::SyncModeFfi::TwoWay,
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

async fn contacts_listed(database: &postio_storage::Store) -> Vec<String> {
    let reader = database.read().await.expect("a reader");
    ContactRepository::new(&reader)
        .saved_for_sync()
        .await
        .expect("saved people")
        .into_iter()
        .map(|(person, _)| person.shown_as().into_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn bringing_in_saves_the_people_only_contacts_has_and_only_for_the_plan_shown() {
    let (session, database) = session_with_grace().await;
    let records = vec![
        theirs("x1", "Ada", "ada@example.org"),
        theirs("x2", "Ben", "ben@example.org"),
    ];
    let s = session.clone();
    let shown = {
        let records = records.clone();
        tokio::task::spawn_blocking(move || {
            s.contacts_sync_plan(
                records,
                SyncScopeFfi::PeopleAndCompanies,
                postio_ffi::SyncModeFfi::ReadOnly,
            )
        })
        .await
        .expect("planned")
    };
    let button = shown
        .sheet
        .and_then(|sheet| sheet.plan)
        .and_then(|plan| plan.bring_in)
        .expect("two people could come in");
    assert_eq!(button.label, "Bring 2 into Postio");

    // A snapshot that changed since the preview is not the plan that was
    // shown: nothing is written and the person is told to preview again.
    let s = session.clone();
    let stale = {
        let mut changed = records.clone();
        changed.push(theirs("x3", "Cy", "cy@example.org"));
        let fingerprint = button.fingerprint;
        tokio::task::spawn_blocking(move || {
            s.contacts_sync_bring_in(
                changed,
                SyncScopeFfi::PeopleAndCompanies,
                postio_ffi::SyncModeFfi::ReadOnly,
                fingerprint,
            )
        })
        .await
        .expect("answered")
    };
    assert_eq!(
        stale.error.as_deref(),
        Some("Contacts changed since the preview. Preview again.")
    );
    assert_eq!(contacts_listed(&database).await, ["Grace"]);

    let s = session.clone();
    let fingerprint = button.fingerprint;
    let done = tokio::task::spawn_blocking(move || {
        s.contacts_sync_bring_in(
            records,
            SyncScopeFfi::PeopleAndCompanies,
            postio_ffi::SyncModeFfi::ReadOnly,
            fingerprint,
        )
    })
    .await
    .expect("answered");
    assert_eq!(
        done.note.as_deref(),
        Some("Brought 2 people into Postio. Nothing was written to Contacts."),
        "{done:?}"
    );
    let mut listed = contacts_listed(&database).await;
    listed.sort();
    assert_eq!(listed, ["Ada", "Ben", "Grace"]);
    session.shutdown();
}
