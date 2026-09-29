//! The capture sheet (US15, T158; screen 25): `t` on a message captures a
//! task into the person's Obsidian vault, `n` a note, over a vault that is
//! a folder on this computer. What is appended is asserted byte for byte,
//! and every other note is asserted unchanged.

use chrono::TimeZone;
use gtk::gdk;
use postio_storage::repository::{Marker, MarkerRepository, MarkerSource};

use crate::support::{self, Fixture};

/// A vault with a Harbor project note (in the projects folder) and a tasks
/// note, each with something already in it.
struct TempVault {
    dir: tempfile::TempDir,
}

const HARBOR: &str = "---\ntype: project\n---\n# Harbor\n\n- [ ] Earlier task\n";
const TASKS: &str = "# Tasks\n";

impl TempVault {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a vault folder");
        std::fs::create_dir(dir.path().join("Projects")).expect("a projects folder");
        std::fs::write(dir.path().join("Projects/Harbor.md"), HARBOR).expect("Harbor");
        std::fs::write(dir.path().join("Tasks.md"), TASKS).expect("Tasks");
        TempVault { dir }
    }

    fn config(&self) -> postio_config::Config {
        postio_config::Config::from_toml_str(&format!(
            "[focus.vault]\npath = \"{}\"\nprojects = \"Projects\"\n",
            self.dir.path().display()
        ))
        .expect("a config with a vault")
    }

    fn read(&self, note: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(note)).expect("the note reads")
    }
}

/// Lena's "Harbor API draft v3", with a to-do marker quoting "Please leave
/// comments by Wednesday", due Wednesday 30 September 2026.
async fn harbor_to_do(fixture: &Fixture) -> postio_model::MessageId {
    let (message, _) = fixture
        .file(
            ("Lena Park", "lena@example.org"),
            "Harbor API draft v3",
            "Uploaded v3. Please leave comments by Wednesday; I'd like to freeze it Thursday.",
            20,
        )
        .await;
    let sentence = "Please leave comments by Wednesday";
    let connection = fixture.database.connect().await.expect("a connection");
    MarkerRepository::new(&connection)
        .insert(&Marker {
            message,
            kind: postio_model::listing::MarkerKind::Todo,
            source: MarkerSource::Detector,
            span: Some((0, sentence.chars().count() as u32)),
            excerpt: Some(sentence.to_owned()),
            starts_at: None,
            ends_at: None,
            // Noon on the day, so it is the 30th in any zone the suite runs.
            due_at: Some(
                chrono::Utc
                    .with_ymd_and_hms(2026, 9, 30, 12, 0, 0)
                    .single()
                    .expect("a time"),
            ),
            invite: None,
            invite_state: None,
            answer: None,
            dismissed_at: None,
        })
        .await
        .expect("a marker");
    message
}

/// The actions the first row on screen draws, by their words.
fn row_actions(window: &postio_focus::window::FocusWindow) -> Vec<String> {
    window
        .pane()
        .and_then(|pane| pane.rows_on_screen().into_iter().next())
        .map(|row| {
            row.drawn()
                .actions
                .into_iter()
                .map(|(_, words, _)| words)
                .collect()
        })
        .unwrap_or_default()
}

/// US15 scenario 1: with a vault configured, the to-do's row offers Task
/// `t`; `t` opens the sheet with the sentence verbatim, the due day from
/// the mail, and the Harbor project suggested with its reason; the preview
/// is the exact line; and `Ctrl+Enter` appends exactly that line to
/// Harbor's note, with the link before 📅, and changes nothing else.
pub fn t_appends_one_tasks_line_with_the_link_before_the_date() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let vault = TempVault::new();
        let fixture = Fixture::empty().await;
        let message = harbor_to_do(&fixture).await;
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &vault.config(),
        ));
        assert!(
            crate::settle_until(async || row_actions(&window).contains(&"Task".to_owned())).await,
            "with a vault, the to-do's row offers Task: {:?}",
            row_actions(&window)
        );

        support::keys(&window, &["j", "t"]);
        let sheet = window.capture().expect("t opened the capture sheet");
        assert!(
            crate::settle_until(async || sheet.is_shown()).await,
            "the sheet never drew"
        );
        assert_eq!(
            sheet.text(),
            "Please leave comments by Wednesday",
            "the sentence, as written"
        );
        let line = format!(
            "- [ ] Please leave comments by Wednesday [\u{2709}](postio://message/{}) \u{1F4C5} 2026-09-30",
            message.get()
        );
        assert!(
            crate::settle_until(async || sheet.preview() == line).await,
            "the preview is not the exact line: {:?}",
            sheet.preview()
        );
        let said = sheet.texts();
        for wanted in ["Add task", "This exact line will be appended", "Harbor"] {
            assert!(
                said.iter().any(|text| text == wanted),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            said.iter()
                .any(|text| text.starts_with("Project \u{b7} suggested") && text.contains("Harbor")),
            "the project is suggested with its reason: {said:?}"
        );

        support::press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !sheet.is_open()).await,
            "Ctrl+Enter did not write and close"
        );
        assert!(
            crate::settle_until(async || vault.read("Projects/Harbor.md") != HARBOR).await,
            "nothing was appended to Harbor's note"
        );
        assert_eq!(
            vault.read("Projects/Harbor.md"),
            format!("{HARBOR}{line}\n"),
            "exactly one line, appended, and the note's own bytes untouched"
        );
        assert_eq!(vault.read("Tasks.md"), TASKS, "no other note changed");
        assert!(
            crate::settle_until(async || window
                .toast_showing()
                .is_some_and(|toast| toast.starts_with("Task added to Harbor")))
            .await,
            "no word that the task was added: {:?}",
            window.toast_showing()
        );
    });
}

/// `n` opens the sheet as a note, `alt+s` puts the subject in place of the
/// sentence, and the note entry is appended to the project's note.
pub fn n_appends_a_note_and_alt_s_uses_the_subject() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let vault = TempVault::new();
        let fixture = Fixture::empty().await;
        let message = harbor_to_do(&fixture).await;
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &vault.config(),
        ));
        assert!(
            crate::settle_until(async || !support::subjects(&window).is_empty()).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j", "n"]);
        let sheet = window.capture().expect("n opened the capture sheet");
        assert!(crate::settle_until(async || sheet.is_shown()).await);
        assert!(
            sheet.texts().iter().any(|text| text == "Add note"),
            "{:?}",
            sheet.texts()
        );
        support::press(&window, "s", gdk::ModifierType::ALT_MASK);
        assert_eq!(sheet.text(), "Harbor API draft v3", "alt+s: the subject");
        let line = format!(
            "- Harbor API draft v3 [\u{2709}](postio://message/{})",
            message.get()
        );
        assert!(
            crate::settle_until(async || sheet.preview() == line).await,
            "the preview is not the note's line: {:?}",
            sheet.preview()
        );
        support::press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || vault.read("Projects/Harbor.md") != HARBOR).await,
            "nothing was appended"
        );
        assert_eq!(
            vault.read("Projects/Harbor.md"),
            format!("{HARBOR}{line}\n")
        );
        assert_eq!(vault.read("Tasks.md"), TASKS);
    });
}

/// With no `[focus.vault]`, the to-do offers no Task, and `t` opens no
/// sheet: it says why instead (spec C9).
pub fn without_a_vault_there_is_no_task_and_t_says_why() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        harbor_to_do(&fixture).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || !row_actions(&window).is_empty()).await,
            "the marker never drew"
        );
        assert_eq!(row_actions(&window), ["Snooze"], "no Task without a vault");
        support::keys(&window, &["j", "t"]);
        assert!(
            window.capture().is_none_or(|sheet| !sheet.is_open()),
            "no sheet without a vault"
        );
        assert!(
            crate::settle_until(async || window
                .toast_showing()
                .is_some_and(|toast| toast.contains("[focus.vault]")))
            .await,
            "t did not say why: {:?}",
            window.toast_showing()
        );
    });
}
