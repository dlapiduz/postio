//! `config.toml`, applied while Focus runs (US7 scenario 1, T060): a saved
//! `[keys]` rebind reaches the keyboard, every keycap and the key map at
//! once; a saved `[focus]` reaches what the empty inbox names.
//!
//! Config-watcher cases: under load, run them alone
//! (`--test-threads 1`), since inotify is shared machine-wide.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// Write `text` over `path` the way an editor saves: a scratch file renamed
/// into place, so the watcher sees one complete file.
fn save(path: &std::path::Path, text: &str) {
    let scratch = path.with_extension("toml.saving");
    std::fs::write(&scratch, text).expect("the scratch file");
    std::fs::rename(&scratch, path).expect("saved");
}

/// Open `fixture` under the config at `path`, following it.
async fn open_following(
    fixture: &Fixture,
    path: &std::path::Path,
) -> postio_gtk::window::FocusWindow {
    let config =
        postio_config::Config::from_toml_str(&std::fs::read_to_string(path).expect("the config"))
            .expect("a config");
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    let session = postio_gtk::startup::adopt(&window, fixture.host(), &config);
    assert!(
        session.follow_config(&window, path),
        "the config is watched"
    );
    support::keep(session);
    window
}

/// The keycaps on the bulk bar's Archive button.
fn archive_cap(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    support::with_class(window, "focus-bulk-archive")
        .first()
        .map(|button| {
            support::with_class(button, "postio-keyhint")
                .iter()
                .flat_map(support::texts)
                .collect()
        })
        .unwrap_or_default()
}

pub fn a_saved_rebind_reaches_the_keyboard_every_keycap_and_the_key_map() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "").expect("an empty config");
        let fixture = Fixture::empty().await;
        for (subject, minutes) in [("First", 10), ("Second", 20), ("Third", 30)] {
            fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    subject,
                    "A line.",
                    minutes,
                )
                .await;
        }
        let window = open_following(&fixture, &path).await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j", "x"]);
        assert!(
            crate::settle_until(async || archive_cap(&window) == ["a"]).await,
            "the bulk bar's Archive wears a: {:?}",
            archive_cap(&window)
        );

        save(&path, "[keys]\narchive = \"w\"\n");
        assert!(
            crate::settle_until(async || archive_cap(&window) == ["w"]).await,
            "the saved rebind never reached the Archive button: {:?}",
            archive_cap(&window)
        );
        support::press(&window, "question", gtk::gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                window.key_map().is_some_and(|dialog| {
                    support::with_class(&dialog, "focus-keymap-row")
                        .iter()
                        .any(|row| {
                            let said = support::texts(row);
                            said.first().is_some_and(|title| title == "Archive")
                                && said.contains(&"w".to_owned())
                        })
                })
            })
            .await,
            "the key map does not show w for Archive"
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(crate::settle_until(async || window.key_map().is_none()).await);

        support::keys(&window, &["Escape", "a"]);
        crate::settle();
        assert_eq!(support::subjects(&window).len(), 3, "a no longer archives");
        support::keys(&window, &["w"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "w archives now: {:?}",
            support::subjects(&window)
        );
        drop(directory);
    });
}

pub fn a_saved_focus_section_reaches_the_empty_inbox() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "[focus]\nfiltering = true\n").expect("a config");
        let fixture = Fixture::empty().await;
        let window = open_following(&fixture, &path).await;
        let says = |window: &postio_gtk::window::FocusWindow| {
            support::with_class(window, "focus-empty")
                .first()
                .map(support::texts)
                .unwrap_or_default()
        };
        assert!(
            crate::settle_until(async || says(&window).contains(&"0 filtered today".to_owned()))
                .await,
            "the empty inbox counts what was filtered: {:?}",
            says(&window)
        );
        save(&path, "[focus]\nfiltering = false\n");
        assert!(
            crate::settle_until(async || {
                let said = says(&window);
                !said.is_empty() && !said.iter().any(|line| line.contains("filtered today"))
            })
            .await,
            "turning filtering off never reached the empty inbox: {:?}",
            says(&window)
        );
        drop(directory);
    });
}

/// A sending identity on `fixture`'s account that signs with `text`: what a
/// reply signs with.
async fn signs_with(fixture: &Fixture, text: &str) {
    let connection = fixture.database.connect().await.expect("a connection");
    let mut identity =
        postio_model::Identity::new(fixture.account.id, fixture.account.address.clone());
    identity.is_default = true;
    identity.signature = Some(postio_model::Signature::new("Work", text));
    postio_storage::repository::IdentityRepository::new(&connection)
        .create(&mut identity)
        .await
        .expect("a sending identity");
}

/// What the open composer's body says, once it says anything.
fn composed(window: &postio_gtk::window::FocusWindow) -> String {
    window
        .composer()
        .and_then(|composer| composer.draft().body.text)
        .unwrap_or_default()
}

/// `[compose]` applied live (T235): a saved `signature_on_reply` moves the
/// next reply's signature from above the quote to below it, with no
/// restart.
pub fn a_saved_compose_section_reaches_the_next_reply() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor", "x", 10)
            .await;
        fixture.write_body(message, "The quoted line.").await;
        signs_with(&fixture, "Ada at Atlas").await;
        let (window, _directory, path) = crate::settings::open_under(
            &fixture,
            "[compose]\nsignature_on_reply = \"above_quote\"\n",
        )
        .await;
        assert!(crate::settings::watched(), "the config is watched");
        let order = |text: &str| -> Option<bool> {
            Some(text.find("Ada at Atlas")? < text.find("The quoted line.")?)
        };

        support::keys(&window, &["j", "e"]);
        assert!(
            crate::settle_until(async || order(&composed(&window)).is_some()).await,
            "the reply quotes and signs: {:?}",
            composed(&window)
        );
        assert_eq!(
            order(&composed(&window)),
            Some(true),
            "above the quote, as the file says: {:?}",
            composed(&window)
        );
        window.composer().expect("the composer").discard();
        crate::settle();

        save(&path, "[compose]\nsignature_on_reply = \"below_quote\"\n");
        // The watcher's own debounce, then the reload.
        crate::settle_for(std::time::Duration::from_millis(600)).await;
        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || order(&composed(&window)) == Some(false)).await,
            "the saved placement never reached the next reply: {:?}",
            composed(&window)
        );
    });
}

/// `[reader]` applied live (T235): a saved zoom is the zoom the open
/// message reads at.
pub fn a_saved_reader_zoom_reaches_the_open_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor", "x", 10)
            .await;
        fixture.write_body(message, "A body to read.").await;
        let (window, _directory, path) =
            crate::settings::open_under(&fixture, "[reader]\nzoom = 100\n").await;
        assert!(crate::settings::watched(), "the config is watched");
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let zoom = || {
            window
                .reading()
                .map(|reading| reading.reader().zoom())
                .unwrap_or_default()
        };
        assert!(
            crate::settle_until(async || zoom() == 100).await,
            "the message opens at the file's zoom: {}",
            zoom()
        );
        save(&path, "[reader]\nzoom = 150\n");
        assert!(
            crate::settle_until(async || zoom() == 150).await,
            "the saved zoom never reached the open message: {}",
            zoom()
        );
    });
}

/// `[storage] max_bytes`, edited live, reaches the store Focus opened
/// (#929; T235): the classic app's `storage_ceiling_wiring`, ported. The
/// file starts with no `[storage]`, so the startup pass evicts nothing and
/// anything evicted can only be the live path.
pub fn editing_the_ceiling_live_evicts_a_running_stores_oldest_blobs() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        let blobs = fixture.blob_store();
        let mut written = Vec::new();
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let messages = postio_storage::repository::MessageRepository::new(&connection);
            for index in 0..3u8 {
                let blob = blobs
                    .put(&vec![b'a' + index; 40_000])
                    .expect("a raw source");
                let received =
                    chrono::TimeZone::timestamp_opt(&chrono::Utc, 1_000 + i64::from(index), 0)
                        .single()
                        .expect("a timestamp");
                let mut message =
                    postio_model::Message::new(fixture.account.id, fixture.inbox, received);
                message.server.uid = Some(postio_model::ids::Uid::new(u32::from(index) + 1));
                message.server.uid_validity = Some(postio_model::ids::UidValidity::new(1));
                message.raw_blob_id = Some(blob.clone());
                messages.create(&mut message).await.expect("a message");
                written.push(blob);
            }
        }
        let (_window, _directory, path) = crate::settings::open_under(&fixture, "").await;
        assert!(crate::settings::watched(), "the config is watched");
        assert!(
            crate::settle_until(async || written.iter().all(|blob| blobs.contains(blob))).await,
            "with no [storage] at all, nothing is evicted at start"
        );

        let budget = blobs.len_of(&written[2]).expect("its length") + 16;
        save(&path, &format!("[storage]\nmax_bytes = {budget}\n"));
        assert!(
            crate::settle_until(
                async || !blobs.contains(&written[0]) && !blobs.contains(&written[1])
            )
            .await,
            "the saved ceiling never reached the running store"
        );
        assert!(
            blobs.contains(&written[2]),
            "the newest fit the budget and is kept"
        );
    });
}
