//! What the composer costs the thread that draws, counted rather than timed
//! (#1608): autosave writes on the runtime and keeps one row, opening a new
//! draft or a reply reads nothing on the GTK thread, and typing a recipient
//! opens no store connection while still offering the contact.
//!
//! The counting seam is thread-local, so what it sees here is exactly what
//! ran on this thread.

use postio_storage::Store;
use postio_storage::test_support::counting::{checkouts, counted};

use crate::support::{self, Fixture};

/// A fixture the composer can write from: an identity, and a Drafts folder.
async fn writable() -> Fixture {
    let fixture = Fixture::empty().await;
    let connection = fixture.database.connect().await.expect("a connection");
    postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
    postio_storage::test_support::mailbox(&connection, &fixture.account, "Sent").await;
    let mut identity =
        postio_model::Identity::new(fixture.account.id, fixture.account.address.clone());
    identity.display_name = "Test User".to_owned();
    identity.is_default = true;
    postio_storage::repository::IdentityRepository::new(&connection)
        .create(&mut identity)
        .await
        .expect("an identity");
    drop(connection);
    fixture
}

async fn drafts(database: &Store) -> i64 {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::one(&connection, "SELECT count(*) FROM drafts", (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .expect("a count")
}

/// Focus's window over `fixture` with one conversation to reply to, and the
/// composer open on a new draft.
async fn composing(
    fixture: &Fixture,
) -> (
    postio_focus::window::FocusWindow,
    postio_widgets::composer::Composer,
) {
    fixture
        .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
        .await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || window.composer().is_some()).await,
        "Focus mounted no composer"
    );
    support::keys(&window, &["c"]);
    assert!(
        crate::settle_until(async || window.compose_dialog().is_some()).await,
        "c opened no composer"
    );
    let composer = window.composer().expect("the composer");
    (window, composer)
}

pub fn autosave_writes_off_the_main_thread_and_keeps_one_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = writable().await;
        let before = drafts(&fixture.database).await;
        let (_window, composer) = composing(&fixture).await;
        composer.test_set_subject("Tide gate interlock");
        let counts = counted(|| {
            composer.save();
            composer.test_set_body("first words");
            composer.save();
        });
        assert_eq!(
            counts.statements, 0,
            "two autosaves ran {} statements on the GTK thread",
            counts.statements
        );

        assert!(
            crate::settle_until(async || composer.draft().id.is_assigned()).await,
            "the composer never learnt the id its first save assigned"
        );
        assert_eq!(
            drafts(&fixture.database).await - before,
            1,
            "two saves in a row inserted two rows rather than updating one"
        );

        composer.test_set_subject("");
        composer.test_set_body("");
        composer.discard();
        assert!(
            crate::settle_until(async || drafts(&fixture.database).await == before).await,
            "closing an emptied composer left its autosaved row behind"
        );
    });
}

pub fn opening_a_draft_or_a_reply_reads_nothing_on_the_main_thread() {
    // The composer asks "what does a new draft sign with" and "what is `e`
    // replying to" through seams that once answered with store reads on the
    // GTK thread: two cold connections, five statements and a body decode in
    // front of a reply.
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = writable().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "Focus mounted no composer"
        );

        let counts = counted(|| support::keys(&window, &["c"]));
        assert!(window.compose_dialog().is_some(), "`c` opened no composer");
        assert_eq!(
            counts.statements, 0,
            "opening a new draft ran {} statements on the GTK thread",
            counts.statements
        );
        window.composer().expect("the composer").discard();
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "the composer did not close"
        );

        support::keys(&window, &["j"]);
        let counts = counted(|| support::keys(&window, &["e"]));
        assert_eq!(
            counts.statements, 0,
            "asking to reply ran {} statements on the GTK thread",
            counts.statements
        );
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "`e` never opened a reply to the message on screen"
        );
    });
}

pub fn typing_a_recipient_opens_no_connections_and_still_completes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = writable().await;
        {
            // A contact the test names: the account's own, so completion
            // offers it.
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::ContactRepository::new(&connection)
                .create(
                    Some(fixture.account.id),
                    &postio_model::EmailAddress::new(None::<String>, "wilhelmina@example.com"),
                    Some("Wilhelmina Quartz"),
                )
                .await
                .expect("create the contact");
        }
        let (_window, composer) = composing(&fixture).await;

        // The directory is read when the composer opens; wait for it the way
        // a person would -- by typing and seeing the contact offered.
        // Alternating, because an unchanged text asks nothing, and the first
        // ask may land before the read does.
        let flip = std::cell::Cell::new(false);
        let offered = crate::settle_until(async || {
            composer.test_set_to(if flip.replace(!flip.get()) {
                "wil"
            } else {
                "wilh"
            });
            composer.test_recipient_suggestion_count() > 0
        })
        .await;
        assert!(offered, "the composer never offered the contact");

        // Now the budget: a name typed letter by letter.
        let before = checkouts();
        for typed in ["w", "wi", "wil", "wilh", "wilhe", "quar", "quartz"] {
            composer.test_set_to(typed);
            crate::settle();
            assert!(
                composer.test_recipient_suggestion_count() > 0,
                "`{typed}` offered nothing"
            );
        }
        assert_eq!(
            checkouts() - before,
            0,
            "typing a recipient opened store connections on the thread that draws"
        );
    });
}
