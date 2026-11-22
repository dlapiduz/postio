use gtk::gdk;
use gtk::prelude::*;

use crate::support::{self};

pub fn m_moves_three_to_receipts_and_ctrl_z_returns_them() {
    crate::gtk_case(async {
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
        support::rows_shown(&window, 3).await;
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
