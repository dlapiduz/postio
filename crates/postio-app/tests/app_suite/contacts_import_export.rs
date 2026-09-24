//! vCard in and out of the Contacts screen (specs/005-contacts User Story 6).
//!
//! `v i` through the seam that stands in for the file dialog (a real
//! `GtkFileDialog` cannot be opened safely inside the suite -- #988): the
//! people and the group appear and a line says what happened. `v x` with
//! nothing selected writes exactly the people the list shows. Nothing leaves
//! the machine, and the log carries counts, never a name or an address.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts, the
// one moment it is sound.

use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::contacts_support::{display, drawn, press, wire};
use crate::{settle, settle_until};
use gtk::prelude::*;
use postio_storage::test_support;

const FILE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Yara Quux\r\n\
EMAIL;TYPE=INTERNET,PREF:yara@home.example\r\nEMAIL:yara@work.example\r\n\
X-VENDOR:kept\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:z\r\nFN:Zebulon Quux\r\nEMAIL:zq@example.net\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Quuxes\r\nMEMBER:urn:uuid:z\r\n\
MEMBER:mailto:yara@work.example\r\nEND:VCARD\r\n";

/// Everything logged on this thread while it is installed.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn v_i_imports_and_v_x_exports_what_the_list_shows() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
        if !display() {
            return;
        }
        let log = Captured::default();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer({
                    let log = log.clone();
                    move || log.clone()
                })
                .finish(),
        );
        let files = tempfile::tempdir().expect("a directory");
        let import = files.path().join("friends.vcf");
        let export = files.path().join("out.vcf");
        std::fs::write(&import, FILE).expect("the file");

        let database = test_support::memory().await;
        postio_storage::seed::seed_small(&database, 11).await;
        let egress_before = egress_rows(&database).await;
        let app = wire(&database).await;
        let window = &app.window;
        assert!(settle_until(async || window.list().model().n_items() > 0).await);
        let pane = window.contacts();
        let asked: Arc<Mutex<Vec<postio_gtk::contacts::FileAsk>>> = Arc::default();
        pane.connect_file_ask({
            let asked = asked.clone();
            move |ask| asked.lock().expect("asks").push(ask)
        });

        // ── v i ─────────────────────────────────────────────────────────────
        press(window, "g");
        press(window, "c");
        press(window, "v");
        press(window, "i");
        assert_eq!(
            *asked.lock().expect("asks"),
            [postio_gtk::contacts::FileAsk::Import]
        );
        pane.file_chosen(import.clone());
        assert!(
            settle_until(async || pane.hint_text().starts_with("Imported")).await,
            "hint {:?}",
            pane.hint_text()
        );
        assert!(
            pane.hint_text().contains("2 new people") && pane.hint_text().contains("1 group"),
            "{}",
            pane.hint_text()
        );
        pane.filter().set_text("quux");
        assert!(
            settle_until(async || drawn(window) == ["Yara Quux", "Zebulon Quux"]).await,
            "drew {:?}",
            drawn(window)
        );
        assert!(
            settle_until(async || pane.group_lines().contains(&"Quuxes".to_owned())).await,
            "groups {:?}",
            pane.group_lines()
        );

        // ── v x, nothing selected, the default view ────────────────────────
        pane.filter().set_text("");
        settle();
        press(window, "v");
        press(window, "x");
        pane.file_chosen(export.clone());
        assert!(settle_until(async || pane.hint_text().starts_with("Exported")).await);
        let written = std::fs::read_to_string(&export).expect("the export");
        assert!(written.contains("FN:Yara Quux") && written.contains("FN:Zebulon Quux"));
        assert!(
            written.contains("X-VENDOR:kept"),
            "the card's own lines survive"
        );
        let people = written.matches("BEGIN:VCARD").count() - written.matches("KIND:group").count();
        assert_eq!(
            people,
            drawn(window).len(),
            "exactly the people the list shows -- none of the mail-only senders"
        );
        assert!(written.contains("FN:Quuxes"), "and the group they are in");

        assert_eq!(
            egress_rows(&database).await,
            egress_before,
            "nothing left the machine"
        );
        let logged = String::from_utf8_lossy(&log.0.lock().expect("log")).into_owned();
        assert!(
            logged.contains("imported contacts"),
            "the import was logged: {logged}"
        );
        for private in ["Yara", "Zebulon", "yara@", "zq@", "Quuxes"] {
            assert!(
                !logged.contains(private),
                "the log named {private}: {logged}"
            );
        }
        app.bridge.shutdown();
    });
}

async fn egress_rows(database: &postio_storage::Store) -> i64 {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::scalar(&connection, "SELECT count(*) FROM egress_log", ())
        .await
        .expect("count the egress log")
}
