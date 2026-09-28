//! The digest window (US10, T137; screen 22's frame, the plain list):
//! `Enter` on a digest's row opens what it holds over the inbox; `⇧A`
//! archives all of it as one undo; `D` stops digesting a sender once
//! confirmed.

use chrono::Duration;
use gtk::gdk;

use crate::support::{self, Fixture};

/// Three conversations, and two newsletters held for "Newsletters" and
/// delivered fifteen minutes ago: the digest's row is second.
async fn delivered() -> Fixture {
    let fixture = Fixture::empty().await;
    for (from, subject, minutes) in [
        (("Ada Moreno", "ada@example.com"), "Atlas budget", 30),
        (("Lena Park", "lena@example.org"), "Harbor draft", 20),
        (("Tomás Reyes", "tomas@example.net"), "Staffing plan", 10),
    ] {
        fixture.file(from, subject, "Hello.", minutes).await;
    }
    let mut held = Vec::new();
    for (subject, minutes) in [("The weekly numbers", 40), ("The rate decision", 50)] {
        let (message, _) = fixture
            .file(("Ledger", "news@ledger.test"), subject, "Rates.", minutes)
            .await;
        held.push(message);
    }
    let connection = fixture.database.connect().await.expect("a connection");
    let digests = postio_storage::repository::DigestRepository::new(&connection);
    for message in &held {
        digests
            .hold(*message, "Newsletters", support::now())
            .await
            .expect("held");
    }
    digests
        .deliver(
            "Newsletters",
            support::now() - Duration::minutes(15),
            support::now() - Duration::minutes(15),
        )
        .await
        .expect("delivered")
        .expect("a delivery");
    drop(connection);
    fixture
}

/// Open the digest's row: the cursor on it, then `Enter`.
async fn open_digest(
    window: &postio_focus::window::FocusWindow,
) -> std::rc::Rc<postio_focus::digest::DigestWindow> {
    assert!(
        crate::settle_until(async || support::subjects(window).len() == 4).await,
        "the inbox and its digest never reached the screen: {:?}",
        support::subjects(window)
    );
    support::keys(window, &["j", "j"]);
    let _ = window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
    let digest = window.digest().expect("Enter opened the digest");
    assert!(
        crate::settle_until(async || digest.subjects().len() == 2).await,
        "the digest never listed what it holds: {:?}",
        digest.subjects()
    );
    digest
}

/// US10 scenario 4: the digest opens on its plain list, newest first; `⇧A`
/// archives all of it and its row leaves the inbox; one `Ctrl+Z` brings
/// it back.
pub fn enter_opens_a_digest_and_shift_a_archives_all_of_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = delivered().await;
        let (window, _client) = fixture.open().await;
        let digest = open_digest(&window).await;
        assert_eq!(
            digest.subjects(),
            ["The weekly numbers", "The rate decision"],
            "newest first"
        );
        let said = digest.texts();
        for wanted in ["Newsletters", "Archive all 2", "2 messages from 1 sender"] {
            assert!(
                said.iter().any(|text| text.contains(wanted)),
                "no {wanted:?} in {said:?}"
            );
        }
        support::press(&window, "A", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Staffing plan", "Harbor draft", "Atlas budget"]
            })
            .await,
            "the digest's row stayed: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || window.digest().is_none()).await,
            "archiving all of it closed the window"
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "one Ctrl+Z did not bring the digest back: {:?}",
            support::subjects(&window)
        );
    });
}

/// US10 scenario 5: `D` on a message in the digest asks first, and once
/// confirmed its sender stops being digested -- out of `config.toml`.
pub fn d_stops_digesting_the_sender_once_confirmed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = delivered().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@ledger.test\"]\n\
             cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
        )
        .expect("a config");
        let config = postio_config::Config::load_from_path(&path).expect("it reads");
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt_at(
            &window,
            fixture.host(),
            &config,
            Some(&path),
        ));
        support::keep(directory);
        let digest = open_digest(&window).await;
        let _ = digest;
        support::press(&window, "D", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || window.stop_digesting_confirmation().is_some()).await,
            "D asked nothing"
        );
        let dialog = window
            .stop_digesting_confirmation()
            .expect("the confirmation");
        let said = format!(
            "{} {}",
            gtk::prelude::ObjectExt::property::<String>(&dialog, "heading"),
            gtk::prelude::ObjectExt::property::<String>(&dialog, "body")
        );
        assert!(
            said.contains("news@ledger.test"),
            "it names the sender: {said}"
        );
        gtk::prelude::ObjectExt::emit_by_name::<()>(&dialog, "response", &[&"stop"]);
        adw::prelude::AdwDialogExt::close(&dialog);
        assert!(
            crate::settle_until(async || {
                !std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("news@ledger.test")
            })
            .await,
            "the sender is still digested:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
    });
}
