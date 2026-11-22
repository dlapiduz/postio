use postio_storage::Store;
use postio_storage::test_support::counting::{checkouts, counted};

use crate::support::{self, Fixture};

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

async fn composing(
    fixture: &Fixture,
) -> (
    postio_gtk::window::FocusWindow,
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
    // The composer asks through seams that once answered with store reads on the GTK thread: two
    // cold connections, five statements and a body decode in front of a reply.
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
        let counts = counted(|| support::keys(&window, &["r"]));
        assert_eq!(
            counts.statements, 0,
            "asking to reply ran {} statements on the GTK thread",
            counts.statements
        );
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "`r` never opened a reply to the message on screen"
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
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::ContactRepository::new(&connection)
                .create(
                    Some("Wilhelmina Quartz"),
                    &[postio_model::EmailAddress::new(
                        None::<String>,
                        "wilhelmina@example.com",
                    )],
                )
                .await
                .expect("create the contact");
        }
        let (_window, composer) = composing(&fixture).await;

        // Alternating, because an unchanged text asks nothing, and the first ask may land before
        // the directory read does.
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

        // The counter is process-wide and the app's startup work opens connections on worker
        // threads: let it finish, and outlast a pending autosave, before taking the reading.
        let seen = std::cell::Cell::new((checkouts(), std::time::Instant::now()));
        let quiet = crate::settle_until_within(std::time::Duration::from_secs(30), async || {
            let (count, since) = seen.get();
            if checkouts() != count {
                seen.set((checkouts(), std::time::Instant::now()));
                return false;
            }
            since.elapsed() >= std::time::Duration::from_secs(2)
        })
        .await;
        assert!(
            quiet,
            "the store never went quiet after the composer opened"
        );

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

/// A hide and a show a frame apart gave the popup's surface a new role while the renderer had a
/// frame in flight, and the compositor dropped the client ("Error 71").
pub fn typing_back_and_forth_across_the_threshold_stays_connected() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = writable().await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::ContactRepository::new(&connection)
                .create(
                    Some("Wilhelmina Quartz"),
                    &[postio_model::EmailAddress::new(
                        None::<String>,
                        "wilhelmina@example.com",
                    )],
                )
                .await
                .expect("create the contact");
        }
        let (_window, composer) = composing(&fixture).await;
        let flip = std::cell::Cell::new(false);
        assert!(
            crate::settle_until(async || {
                composer.test_set_to(if flip.replace(!flip.get()) {
                    "wil"
                } else {
                    "wilh"
                });
                composer.test_recipient_suggestion_count() > 0
            })
            .await,
            "the composer never offered the contact"
        );
        for _ in 0..40 {
            composer.test_set_to("wilh");
            crate::settle();
            composer.test_set_to("wil");
            crate::settle();
        }
        composer.test_set_to("wilh");
        assert!(
            crate::settle_until(async || composer.test_recipient_suggestion_count() > 0).await,
            "four characters still offer the contact"
        );
    });
}
