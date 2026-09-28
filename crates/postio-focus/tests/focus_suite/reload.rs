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
) -> postio_focus::window::FocusWindow {
    let config =
        postio_config::Config::from_toml_str(&std::fs::read_to_string(path).expect("the config"))
            .expect("a config");
    let window = postio_focus::window::FocusWindow::new(None);
    window.present();
    let session = postio_focus::startup::adopt(&window, fixture.host(), &config);
    assert!(
        session.follow_config(&window, path),
        "the config is watched"
    );
    support::keep(session);
    window
}

/// The keycaps on the bulk bar's Archive button.
fn archive_cap(window: &postio_focus::window::FocusWindow) -> Vec<String> {
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
        let says = |window: &postio_focus::window::FocusWindow| {
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
