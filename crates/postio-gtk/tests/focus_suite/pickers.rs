//! The pickers at the row (US5; screens 11-14): `s`, `h`, `l` and `m` open
//! a small picker anchored to the focused row, acting on the selection when
//! there is one.

use chrono::Local;
use gtk::gdk;
use gtk::prelude::*;
use postio_storage::repository::MessageRepository;

use crate::support::{self};

/// US5 scenario 1, in Focus: `s` opens the snooze picker at the row, naming
/// the conversation; `2` snoozes it to tomorrow morning, and it leaves the
/// inbox.
pub fn s_then_2_snoozes_the_row_until_tomorrow_morning() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        let row = window.cursor_row().expect("j put the cursor on a row");
        let subject = support::subjects(&window)[0].clone();
        support::press(&window, "s", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("s opened the snooze picker");
        assert!(
            crate::settle_until(async || picker.is_shown()).await,
            "the snooze picker never drew"
        );
        let said = picker.texts();
        assert!(said.iter().any(|line| line == "Snooze until"), "{said:?}");
        assert!(
            said.iter().any(|line| line.ends_with(&subject)),
            "the picker names what it snoozes: {said:?}"
        );
        let expected = postio_ui::schedule::snooze_presets(Local::now())[1].1;
        support::press(&window, "2", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !support::subjects(&window).contains(&subject)).await,
            "the snoozed conversation stayed in the inbox"
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let stored = MessageRepository::new(&connection)
            .get(row.id())
            .await
            .expect("a read")
            .expect("the message");
        assert_eq!(
            stored.snoozed_until,
            Some(expected.to_utc()),
            "snoozed until tomorrow morning"
        );
    });
}

/// `h` opens the remind picker at the row; `3` sets a reminder for the end
/// of the week on the conversation, which stays in the inbox until then.
pub fn h_then_3_reminds_at_the_end_of_the_week() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let thread = row.thread().expect("a conversation");
        support::press(&window, "h", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("h opened the remind picker");
        assert!(crate::settle_until(async || picker.is_shown()).await);
        let said = picker.texts();
        assert!(
            said.iter()
                .any(|line| line == "Remind me if no one replies by"),
            "{said:?}"
        );
        let expected = postio_ui::schedule::remind_presets(Local::now())[2].1;
        support::press(&window, "3", gdk::ModifierType::empty());
        let connection = fixture.database.connect().await.expect("a connection");
        let reminders = postio_storage::repository::ReminderRepository::new(&connection);
        // Wait on the stored reminder with the suite's patience, not a count
        // of main-loop turns: under load 200 turns went by before the write.
        assert!(
            crate::settle_until(async || reminders
                .standing(thread)
                .await
                .expect("a read")
                .is_some())
            .await,
            "no reminder was set"
        );
        let standing = reminders
            .standing(thread)
            .await
            .expect("a read")
            .expect("the reminder just seen");
        assert_eq!(
            standing.due_at,
            expected.to_utc(),
            "due at the end of the week"
        );
        assert!(!picker.is_open(), "choosing closed the picker");
    });
}

/// US5 scenario 7: with a picker open, `Esc` closes it and nothing
/// changes -- the conversation stays in the inbox, and the list keeps its
/// cursor.
pub fn escape_closes_a_picker_and_changes_nothing() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let before = support::subjects(&window);
        for key in ["s", "h"] {
            support::press(&window, key, gdk::ModifierType::empty());
            let picker = window.open_picker().expect("a picker opened");
            assert!(crate::settle_until(async || picker.is_shown()).await);
            support::press(&window, "Escape", gdk::ModifierType::empty());
            assert!(!picker.is_open(), "Escape closed the {key} picker");
            assert!(window.open_picker().is_none());
        }
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        assert_eq!(support::subjects(&window), before, "nothing left the inbox");
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            Some(row.id()),
            "the cursor stayed"
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let stored = MessageRepository::new(&connection)
            .get(row.id())
            .await
            .expect("a read")
            .expect("the message");
        assert_eq!(stored.snoozed_until, None, "nothing was snoozed");
        let reminder = postio_storage::repository::ReminderRepository::new(&connection)
            .standing(row.thread().expect("a conversation"))
            .await
            .expect("a read");
        assert_eq!(reminder, None, "no reminder was set");
    });
}

/// The labels on `thread`, by name.
async fn labels_on(fixture: &support::Fixture, thread: postio_model::ThreadId) -> Vec<String> {
    let connection = fixture.database.connect().await.expect("a connection");
    let mut names: Vec<String> = postio_storage::repository::LabelRepository::new(&connection)
        .for_threads(&[thread])
        .await
        .expect("a read")
        .into_iter()
        .map(|(_, label)| label.name)
        .collect();
    names.sort();
    names
}

/// Wait for the labels on `thread` to be `wanted`.
async fn labels_become(
    fixture: &support::Fixture,
    thread: postio_model::ThreadId,
    wanted: &[&str],
) -> Vec<String> {
    // The suite's patience, not a count of main-loop turns, which load can
    // outrun before the write lands.
    crate::settle_until(async || labels_on(fixture, thread).await == wanted).await;
    labels_on(fixture, thread).await
}

/// US5 scenario 5: `l` lists the labels, each applied one marked and the
/// rest counted; `Space` takes an applied label off; a new name typed and
/// confirmed is created and applied; and each change is undone by `Ctrl+Z`.
pub fn l_toggles_a_label_and_creates_a_new_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let thread = row.thread().expect("a conversation");
        fixture.label(row.id(), &["Atlas"]).await;
        let others = support::subjects(&window);
        let other = window
            .pane()
            .expect("the list")
            .rows_on_screen()
            .into_iter()
            .filter_map(|widget| widget.item())
            .find(|item| item.id() != row.id())
            .expect("another conversation");
        fixture.label(other.id(), &["Harbor"]).await;
        assert!(!others.is_empty());

        support::press(&window, "l", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("l opened the label picker");
        assert!(
            crate::settle_until(async || {
                let said = picker.texts();
                said.iter().any(|line| line == "Harbor")
                    && said.iter().any(|line| line == "\u{2713} applied")
            })
            .await,
            "the labels never listed: {:?}",
            picker.texts()
        );
        let said = picker.texts();
        assert!(said.iter().any(|line| line == "Labels"), "{said:?}");
        assert!(
            said.iter().any(|line| line == "1"),
            "Harbor's count: {said:?}"
        );

        // Space on the applied Atlas takes it off.
        support::press(&window, "space", gdk::ModifierType::empty());
        assert_eq!(
            labels_become(&fixture, thread, &[]).await,
            Vec::<String>::new(),
            "Space took Atlas off"
        );
        assert!(picker.is_open(), "toggling keeps the picker open");

        // A name nobody has is offered, created and applied.
        picker.entry().set_text("Receipts");
        assert!(
            crate::settle_until(async || {
                picker
                    .texts()
                    .iter()
                    .any(|line| line == "Create label \u{201c}Receipts\u{201d}")
            })
            .await,
            "no Create label row: {:?}",
            picker.texts()
        );
        picker.entry().emit_activate();
        assert_eq!(
            labels_become(&fixture, thread, &["Receipts"]).await,
            ["Receipts"],
            "the new label was created and applied"
        );
        assert!(
            crate::settle_until(async || !picker.is_open()).await,
            "Enter closed the picker"
        );

        // Each change is undoable: the last first.
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert_eq!(
            labels_become(&fixture, thread, &[]).await,
            Vec::<String>::new(),
            "Ctrl+Z took the new label off"
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert_eq!(
            labels_become(&fixture, thread, &["Atlas"]).await,
            ["Atlas"],
            "Ctrl+Z put Atlas back"
        );
    });
}

/// US5 scenario 6: with three rows selected, `m` opens the move picker
/// naming all three, the last destination first under Recent with `1`; `1`
/// moves all three out of the inbox, and one `Ctrl+Z` returns them.
pub fn m_moves_three_to_receipts_and_ctrl_z_returns_them() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = support::Fixture::empty().await;
        for (from, subject, minutes) in [
            (("Ada Moreno", "ada@example.com"), "Atlas budget", 30),
            (("Lena Park", "lena@example.org"), "Harbor draft", 20),
            (("Tomás Reyes", "tomas@example.net"), "Staffing plan", 10),
        ] {
            fixture.file(from, subject, "Hello.", minutes).await;
        }
        let receipts = fixture.folder("Receipts").await;
        fixture.folder("Travel").await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::SettingsRepository::new(&connection)
                .note_move(receipts)
                .await
                .expect("a recent move");
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["x", "j", "x", "j", "x"]);
        support::press(&window, "m", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("m opened the move picker");
        assert!(
            crate::settle_until(async || picker.texts().iter().any(|line| line == "Travel")).await,
            "the folders never listed: {:?}",
            picker.texts()
        );
        let said = picker.texts();
        for wanted in [
            "Move to folder",
            "3 conversations",
            "Recent",
            "Receipts",
            "All folders",
            "Archive",
        ] {
            assert!(
                said.iter().any(|line| line == wanted),
                "no {wanted:?} in {said:?}"
            );
        }
        support::press(&window, "1", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || support::subjects(&window).is_empty()).await,
            "the three did not leave the inbox: {:?}",
            support::subjects(&window)
        );
        assert!(!picker.is_open(), "moving closed the picker");
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "one Ctrl+Z did not return all three: {:?}",
            support::subjects(&window)
        );
    });
}

/// T170: the label picker offers the labels of the row's own account --
/// labelling a row of the second account offers the second's labels, not
/// the first's.
pub fn l_offers_the_labels_of_the_row_s_own_account() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = support::Fixture::empty().await;
        let (atlas, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                30,
            )
            .await;
        fixture.label(atlas, &["Atlas"]).await;
        let (second, second_inbox) = fixture.second_account().await;
        let harbor = fixture
            .file_as(second.id, second_inbox, "Harbor draft", 10)
            .await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let labels = postio_storage::repository::LabelRepository::new(&connection);
            let mut label = postio_model::Label::new(second.id, "Harbor");
            labels.create(&mut label).await.expect("a label");
            labels.attach(harbor, label.id).await.expect("attached");
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "both accounts' mail never reached the screen: {:?}",
            support::subjects(&window)
        );
        // The newest first: the second account's row is on top.
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            Some(harbor),
            "the cursor is on the second account's row"
        );
        support::press(&window, "l", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("l opened the label picker");
        assert!(
            crate::settle_until(async || picker.texts().iter().any(|text| text == "Harbor")).await,
            "the second account's labels are not offered: {:?}",
            picker.texts()
        );
        assert!(
            !picker.texts().iter().any(|text| text == "Atlas"),
            "the first account's labels are offered: {:?}",
            picker.texts()
        );
    });
}
