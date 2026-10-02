//! Commands Focus offers and must answer (T236, T238, T257): the go-to keys
//! `g s`, `g r`, `g z` and `g *`, Unsnooze on `B`, and Flag on `*`, each
//! pressed as GTK delivers it, and each judged by what the list shows and
//! what the store holds.

use chrono::Duration;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, MessageTarget};
use postio_model::{Flag, MailboxRole, MessageId};
use postio_storage::repository::{MailboxRepository, MessageRepository};

use crate::support::{self, Fixture};

type Window = postio_focus::window::FocusWindow;

async fn stored(fixture: &Fixture, message: MessageId) -> postio_model::Message {
    let connection = fixture.database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .await
        .expect("a read")
        .expect("the message")
}

async fn archive_of(fixture: &Fixture) -> postio_model::MailboxId {
    let connection = fixture.database.connect().await.expect("a connection");
    MailboxRepository::new(&connection)
        .by_role(fixture.account.id, MailboxRole::Archive)
        .await
        .expect("a read")
        .expect("an archive")
        .id
}

async fn showing(window: &Window, place: &str, subjects: &[&str]) {
    assert!(
        crate::settle_until(async || {
            window.place_name() == place && support::subjects(window) == subjects
        })
        .await,
        "expected {place:?} listing {subjects:?}; the window says {:?} listing {:?}",
        window.place_name(),
        support::subjects(window)
    );
}

async fn snooze(fixture: &Fixture, message: MessageId) {
    let connection = fixture.database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .snooze(&[message], chrono::Utc::now() + Duration::days(2))
        .await
        .expect("snoozed");
}

async fn flag(client: &postio_client::Client, message: MessageId) {
    client
        .send(Command::Flag {
            target: MessageTarget::Messages(vec![message]),
            flagged: Some(true),
        })
        .await
        .expect("flagged");
}

/// `g s` and `g r` go to the Sent and Archive folders, as `g t` goes to
/// Drafts; `g i` comes back.
pub fn g_s_and_g_r_list_sent_and_archive() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "In the inbox", "x", 5)
            .await;
        let sent = fixture.folder("Sent").await;
        fixture.file_in(sent, "Sent note", 20).await;
        let archive = archive_of(&fixture).await;
        fixture.file_in(archive, "Old archive", 30).await;
        let (window, _client) = fixture.open().await;

        support::keys(&window, &["g", "s"]);
        showing(&window, "Sent", &["Sent note"]).await;
        support::keys(&window, &["g", "r"]);
        showing(&window, "Archive", &["Old archive"]).await;
        support::keys(&window, &["g", "i"]);
        showing(&window, "Inbox", &["In the inbox"]).await;
    });
}

/// `g z` lists what is snoozed, and `g *` what is flagged.
pub fn g_z_lists_snoozed_and_g_star_lists_flagged() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
            .await;
        let (harbor, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor draft", "x", 20)
            .await;
        fixture
            .file(
                ("Tomas Reyes", "tomas@example.net"),
                "Staffing plan",
                "x",
                10,
            )
            .await;
        snooze(&fixture, harbor).await;
        let (window, client) = fixture.open().await;
        flag(&client, atlas).await;

        support::keys(&window, &["g", "z"]);
        showing(&window, "Snoozed", &["Harbor draft"]).await;
        support::keys(&window, &["g", "asterisk"]);
        showing(&window, "Flagged", &["Atlas budget"]).await;
    });
}

/// A fixture with "Atlas budget" snoozed and "Staffing plan" in the inbox,
/// and the window showing the Snoozed list with the cursor on the first row.
async fn in_snoozed() -> (Fixture, Window, MessageId) {
    let fixture = Fixture::empty().await;
    let (atlas, _) = fixture
        .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
        .await;
    fixture
        .file(
            ("Tomas Reyes", "tomas@example.net"),
            "Staffing plan",
            "x",
            10,
        )
        .await;
    snooze(&fixture, atlas).await;
    let (window, _client) = fixture.open().await;
    support::keys(&window, &["g", "z"]);
    showing(&window, "Snoozed", &["Atlas budget"]).await;
    (fixture, window, atlas)
}

async fn unsnoozed_back_in_the_inbox(fixture: &Fixture, window: &Window, atlas: MessageId) {
    assert!(
        crate::settle_until(async || stored(fixture, atlas).await.snoozed_until.is_none()).await,
        "the store still has it asleep"
    );
    assert!(
        crate::settle_until(async || support::subjects(window).is_empty()).await,
        "the Snoozed list still shows it: {:?}",
        support::subjects(window)
    );
    assert!(
        crate::settle_until(async || window.toast_showing().is_some()).await,
        "no undo toast"
    );
    support::keys(window, &["g", "i"]);
    showing(window, "Inbox", &["Staffing plan", "Atlas budget"]).await;
}

/// Right-click the first row until the menu is up: the list is still
/// settling after a change of folder, and a press that lands while it
/// redraws is lost, as a person's would be.
async fn right_clicked(window: &Window) -> gtk::Popover {
    let up = |window: &Window| {
        window
            .row_menu()
            .is_some_and(|menu| menu.is_open() && menu.widget().width() > 0)
    };
    assert!(
        crate::settle_until(async || {
            if !up(window) {
                crate::row_menu::right_click(window, 0);
            }
            up(window)
        })
        .await,
        "no menu opened on the row"
    );
    window.row_menu().expect("the menu").widget().clone()
}

/// `B` on a snoozed conversation wakes it: it is back in the inbox, the
/// toast offers Undo, and Undo puts it back to sleep until the same time.
pub fn b_unsnoozes_the_row_and_undo_puts_it_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, atlas) = in_snoozed().await;
        let until = stored(&fixture, atlas).await.snoozed_until;
        assert!(until.is_some());
        support::keys(&window, &["j", "B"]);
        unsnoozed_back_in_the_inbox(&fixture, &window, atlas).await;

        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas).await.snoozed_until == until)
                .await,
            "Undo did not put it back to sleep"
        );
        showing(&window, "Inbox", &["Staffing plan"]).await;
    });
}

/// The row menu on a snoozed conversation offers Unsnooze, with its key;
/// one in the inbox does not.
pub fn the_row_menu_unsnoozes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, atlas) = in_snoozed().await;
        let menu = right_clicked(&window).await;
        let said = support::texts(&menu);
        let at = said
            .iter()
            .position(|text| text == "Unsnooze")
            .unwrap_or_else(|| panic!("no Unsnooze in the menu: {said:?}"));
        assert_eq!(said.get(at + 1).map(String::as_str), Some("B"), "{said:?}");
        crate::row_menu::choose(&menu, "focus-row-menu-unsnooze");
        unsnoozed_back_in_the_inbox(&fixture, &window, atlas).await;

        let menu = right_clicked(&window).await;
        assert!(
            !support::texts(&menu).iter().any(|text| text == "Unsnooze"),
            "an inbox row offers Unsnooze: {:?}",
            support::texts(&menu)
        );
    });
}

/// `B` in the open message wakes it, and the dialog leaves it.
pub fn b_in_the_open_message_unsnoozes_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, atlas) = in_snoozed().await;
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        let reading = window.reading().expect("Return opened the message");
        assert!(
            crate::settle_until(async || reading.is_open() && reading.title() == "Atlas budget")
                .await,
            "the message never opened"
        );
        support::deliver_with(&window, "B", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas).await.snoozed_until.is_none())
                .await,
            "B in the open message did not unsnooze it"
        );
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "the dialog stayed on a message that left the list"
        );
        assert!(window.toast_showing().is_some(), "no undo toast");
    });
}

/// `*` flags the row; the store has it, `g *` lists it, the row carries no
/// mark, and Undo clears it. `*` again unflags.
pub fn star_flags_the_row_and_undo_clears_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
            .await;
        fixture
            .file(
                ("Tomas Reyes", "tomas@example.net"),
                "Staffing plan",
                "x",
                10,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let before = window.rows_on_screen();
        support::keys(&window, &["j", "j", "asterisk"]);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "`*` did not flag it"
        );
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "no undo toast"
        );
        assert_eq!(
            window.rows_on_screen().len(),
            before.len(),
            "the row left the inbox"
        );
        assert!(
            !window.rows_on_screen()[1].contains("lagged"),
            "the row carries a flag mark: {:?}",
            window.rows_on_screen()
        );
        support::keys(&window, &["g", "asterisk"]);
        showing(&window, "Flagged", &["Atlas budget"]).await;
        support::keys(&window, &["g", "i"]);
        showing(&window, "Inbox", &["Staffing plan", "Atlas budget"]).await;

        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "Undo did not clear the flag"
        );

        // Pressed on a flagged row, it clears the flag.
        support::keys(&window, &["j", "j", "asterisk"]);
        assert!(
            crate::settle_until(async || stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await
        );
        support::keys(&window, &["asterisk"]);
        assert!(
            crate::settle_until(async || !stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "a second `*` did not unflag it"
        );
    });
}

/// The row menu says Flag on an unflagged row and Unflag on a flagged one,
/// and runs the same command `*` does.
pub fn the_row_menu_flags_and_unflags() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
            .await;
        let (window, _client) = fixture.open().await;
        let menu = right_clicked(&window).await;
        let said = support::texts(&menu);
        let at = said
            .iter()
            .position(|text| text == "Flag")
            .unwrap_or_else(|| panic!("no Flag in the menu: {said:?}"));
        assert_eq!(said.get(at + 1).map(String::as_str), Some("*"), "{said:?}");
        crate::row_menu::choose(&menu, "focus-row-menu-flag");
        assert!(
            crate::settle_until(async || stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "the menu's Flag did not flag it"
        );

        assert!(
            crate::settle_until(async || {
                let menu = right_clicked(&window).await;
                support::texts(&menu).iter().any(|text| text == "Unflag")
            })
            .await,
            "a flagged row's menu never says Unflag"
        );
        let menu = window.row_menu().expect("the menu").widget().clone();
        crate::row_menu::choose(&menu, "focus-row-menu-flag");
        assert!(
            crate::settle_until(async || !stored(&fixture, atlas)
                .await
                .flags
                .contains(&Flag::Flagged))
            .await,
            "the menu's Unflag did not clear it"
        );
    });
}

/// `A`, Archive thread, archives the row's conversation: a Focus row is one,
/// so it does what `a` does.
pub fn capital_a_archives_the_conversation() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j", "A"]);
        assert!(
            crate::settle_until(
                async || support::subjects(&window) == ["Harbor draft", "Atlas budget"]
            )
            .await,
            "`A` left the conversation in the inbox: {:?}",
            support::subjects(&window)
        );
    });
}

/// Focus is one inbox across accounts, so `g z` and `g *` list every
/// account's snoozed and flagged mail, not the first account's.
pub fn g_z_and_g_star_span_every_account() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (first_snoozed, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "First asleep", "x", 50)
            .await;
        let (first_flagged, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "First starred", "x", 40)
            .await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "First plain", "x", 5)
            .await;
        let (second, second_inbox) = fixture.second_account().await;
        let second_snoozed = fixture
            .file_as(second.id, second_inbox, "Second asleep", 30)
            .await;
        let second_flagged = fixture
            .file_as(second.id, second_inbox, "Second starred", 20)
            .await;
        snooze(&fixture, first_snoozed).await;
        snooze(&fixture, second_snoozed).await;
        let (window, client) = fixture.open().await;
        flag(&client, first_flagged).await;
        flag(&client, second_flagged).await;

        support::keys(&window, &["g", "z"]);
        showing(&window, "Snoozed", &["Second asleep", "First asleep"]).await;
        support::keys(&window, &["g", "asterisk"]);
        showing(&window, "Flagged", &["Second starred", "First starred"]).await;
    });
}
