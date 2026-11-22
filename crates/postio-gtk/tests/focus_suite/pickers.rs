use chrono::Local;
use gtk::gdk;
use gtk::prelude::*;
use postio_storage::repository::MessageRepository;

use crate::support::{self};

pub fn s_then_2_snoozes_the_row_until_tomorrow_morning() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        let row = window.cursor_row().expect("j put the cursor on a row");
        let subject = support::subjects(&window)[0].clone();
        support::press(&window, "b", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("b opened the snooze picker");
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
        // Wait on the stored reminder with the suite's patience, not a count of loop turns: under
        // load 200 turns went by before the write.
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

pub fn escape_closes_a_picker_and_changes_nothing() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let before = support::subjects(&window);
        for key in ["b", "h"] {
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

async fn labels_become(
    fixture: &support::Fixture,
    thread: postio_model::ThreadId,
    wanted: &[&str],
) -> Vec<String> {
    // The suite's patience, not a count of main-loop turns, which load can outrun before the write
    // lands.
    crate::settle_until(async || labels_on(fixture, thread).await == wanted).await;
    labels_on(fixture, thread).await
}

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

        support::press(&window, "space", gdk::ModifierType::empty());
        assert_eq!(
            labels_become(&fixture, thread, &[]).await,
            Vec::<String>::new(),
            "Space took Atlas off"
        );
        assert!(picker.is_open(), "toggling keeps the picker open");

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
        support::press(&window, "v", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("v opened the move picker");
        assert!(
            crate::settle_until(async || picker.texts().iter().any(|line| line == "Travel")).await,
            "the folders never listed: {:?}",
            picker.texts()
        );
        let said = picker.texts();
        for wanted in [
            "Move to folder",
            "3 threads",
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
