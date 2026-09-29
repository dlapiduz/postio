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
    delivered_holding().await.0
}

/// As [`delivered`], with the delivery and what it holds, newest first:
/// "The weekly numbers", then "The rate decision".
pub async fn delivered_holding() -> (
    Fixture,
    postio_model::DeliveryId,
    Vec<postio_model::MessageId>,
) {
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
    let delivery = digests
        .deliver(
            "Newsletters",
            support::now() - Duration::minutes(15),
            support::now() - Duration::minutes(15),
        )
        .await
        .expect("delivered")
        .expect("a delivery");
    drop(connection);
    (fixture, delivery, held)
}

/// Open the digest's row: the cursor on it, then `Enter`.
pub async fn open_digest(
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
        // US13 scenario 4: with no summary it opens on its plain list, and
        // offers no Summary tab.
        assert_eq!(
            digest.showing(),
            postio_focus::digest::DigestPage::List,
            "no summary, the list"
        );
        assert!(
            digest.texts().iter().all(|text| text != "Summary"),
            "a Summary tab with no summary: {:?}",
            digest.texts()
        );
        // j walks the digest's rows, as the list's (Context::Digest).
        let first = digest.focused().map(|row| row.id);
        support::press(&window, "j", gdk::ModifierType::empty());
        assert!(
            digest.focused().is_some() && digest.focused().map(|row| row.id) != first,
            "j did not move in the digest"
        );
        let said = digest.texts();
        for wanted in ["Newsletters", "Archive all 2", "2 messages from 1 sender"] {
            assert!(
                said.iter().any(|text| text.contains(wanted)),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            support::keycaps_are_taught(digest.dialog()),
            "the digest window's keycaps do not teach their controls (T142)"
        );
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

/// US10 scenario 1 (T138): `d` on a message opens "Digest this sender"
/// with its address; the preview's count is what the executor finds from
/// that sender in the last 90 days; Create writes the rule to
/// `config.toml`.
pub fn d_on_a_message_previews_the_rule_and_create_writes_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for (subject, days) in [
            ("Issue 3", 3),
            ("Issue 2", 20),
            ("Issue 1", 60),
            ("Old issue", 100),
        ] {
            fixture
                .file(
                    ("Ledger", "news@ledger.test"),
                    subject,
                    "Rates.",
                    days * 24 * 60,
                )
                .await;
        }
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        fixture.index().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "# Mine.\n").expect("a config");
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
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 5).await,
            "the inbox never reached the screen"
        );
        // The cursor on Ledger's newest: the second row.
        support::keys(&window, &["j", "j"]);
        support::press(&window, "d", gdk::ModifierType::empty());
        let dialog = window.rule_dialog().expect("d opened the rule dialog");
        assert!(
            crate::settle_until(async || {
                dialog
                    .texts()
                    .iter()
                    .any(|text| text == "Would have caught 3 messages in the last 90 days")
            })
            .await,
            "the preview is not the executor's three: {:?}",
            dialog.texts()
        );
        let said = dialog.texts();
        for wanted in ["Digest this sender", "news@ledger.test", "Issue 3"] {
            assert!(
                said.iter().any(|text| text == wanted),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            support::keycaps_are_taught(dialog.dialog()),
            "the rule dialog's keycaps do not teach their controls (T142)"
        );
        dialog.create();
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("from:news@ledger.test")
            })
            .await,
            "Create wrote no rule:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .starts_with("# Mine."),
            "the rest of the file is as it was"
        );
        assert!(
            crate::settle_until(async || window.rule_dialog().is_none()).await,
            "Create closed the dialog"
        );
    });
}

/// T139 (FR-126): `g d` lists every rule with what it matches, when it
/// delivers and what it holds now; `Delete` removes the focused rule once
/// confirmed, and what it held comes into the inbox.
pub fn g_d_lists_the_rules_and_delete_releases_what_one_held() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let mut held = Vec::new();
        for (subject, minutes) in [("The weekly numbers", 40), ("The rate decision", 50)] {
            let (message, _) = fixture
                .file(("Ledger", "news@ledger.test"), subject, "Rates.", minutes)
                .await;
            held.push(message);
        }
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let digests = postio_storage::repository::DigestRepository::new(&connection);
            // Held now, by the real clock: its Sunday has not come.
            for message in &held {
                digests
                    .hold(*message, "Newsletters", chrono::Utc::now())
                    .await
                    .expect("held");
            }
        }
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
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Atlas budget"]).await,
            "held mail is not in the inbox: {:?}",
            support::subjects(&window)
        );

        support::keys(&window, &["g", "d"]);
        let rules = window.rules().expect("g d opened the rules");
        assert!(
            crate::settle_until(async || rules.texts().iter().any(|text| text == "holds 2")).await,
            "the rule never said what it holds: {:?}",
            rules.texts()
        );
        let said = rules.texts();
        for wanted in [
            "Digest rules",
            "Newsletters",
            "from:news@ledger.test",
            "Weekly, Sunday 09:00",
        ] {
            assert!(
                said.iter().any(|text| text == wanted),
                "no {wanted:?} in {said:?}"
            );
        }

        assert!(
            support::keycaps_are_taught(rules.widget()),
            "the rules list's keycaps do not teach their controls (T142)"
        );

        support::press(&window, "Delete", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.remove_rule_confirmation().is_some()).await,
            "Delete asked nothing"
        );
        let dialog = window.remove_rule_confirmation().expect("the confirmation");
        gtk::prelude::ObjectExt::emit_by_name::<()>(&dialog, "response", &[&"remove"]);
        adw::prelude::AdwDialogExt::close(&dialog);
        assert!(
            crate::settle_until(async || {
                !std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("Newsletters")
            })
            .await,
            "the rule is still in config.toml"
        );
        assert!(
            crate::settle_until(async || rules.texts().iter().all(|text| text != "Newsletters"))
                .await,
            "the rule is still listed: {:?}",
            rules.texts()
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "what the rule held did not come into the inbox: {:?}",
            support::subjects(&window)
        );
    });
}

/// US10 scenario 7 (T140): held mail is never hidden from search, and a
/// result says where it waits -- the rule that holds it -- rather than
/// the folder it is filed in.
pub fn held_mail_is_found_by_search_and_says_where_it_waits() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let (held, _) = fixture
            .file(
                ("Ledger", "news@ledger.test"),
                "The weekly numbers",
                "Rates.",
                40,
            )
            .await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::DigestRepository::new(&connection)
                .hold(held, "Newsletters", chrono::Utc::now())
                .await
                .expect("held");
        }
        fixture.index().await;
        let config = postio_config::Config::from_toml_str(
            "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@ledger.test\"]\n\
             cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
        )
        .expect("a config");
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &config,
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Atlas budget"]).await,
            "held mail is in the inbox: {:?}",
            support::subjects(&window)
        );
        // T140: the strip says how many rules there are, with g d.
        let chrome = window.chrome().expect("the strip");
        assert!(
            crate::settle_until(async || {
                chrome.digest_rules_said().as_deref() == Some("1 digest rule")
            })
            .await,
            "the strip does not count the rules: {:?}",
            chrome.digest_rules_said()
        );
        assert!(
            support::texts(chrome.strip())
                .iter()
                .any(|text| text == "g d"),
            "with its key"
        );
        support::press(&window, "slash", gdk::ModifierType::empty());
        let bar = window.bar().expect("the bar");
        bar.set_text("from:news@ledger.test");
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["The weekly numbers"]).await,
            "search did not find the held message: {:?}",
            bar.result_subjects()
        );
        assert!(
            crate::settle_until(async || {
                bar.texts()
                    .iter()
                    .any(|text| text == "held \u{b7} Newsletters")
            })
            .await,
            "the result does not say where it waits: {:?}",
            bar.texts()
        );
    });
}
