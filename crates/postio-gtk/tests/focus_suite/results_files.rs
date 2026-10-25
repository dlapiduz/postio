//! The results' Files tab, Quick Look and Save file (spec 010 T173,
//! screens 10 and 11): GTK draws the controller's cards and panel, and
//! hands a file's copy to the desktop.
//!
//! What a file search finds and which card has the ring are the
//! controller's, asserted in `postio-focus`. These cases drive the window
//! as a person does, with spec 010's search staged on.

use std::cell::RefCell;
use std::rc::Rc;

use postio_core::CommandId;

use crate::support::{self, Fixture};

/// A window over a report with its PDF kept and one other message, the
/// search staged on, and "report"'s results open.
async fn report_results() -> (
    postio_gtk::window::FocusWindow,
    Fixture,
    Rc<postio_gtk::results::ResultsPane>,
) {
    postio_gtk::list::feed::stage_results_view(true);
    let fixture = Fixture::empty().await;
    fixture.file_with_kept_attachment("Quarterly report").await;
    fixture
        .file(("Tomas Ruiz", "tomas@example.com"), "Lunch", "Noon?", 10)
        .await;
    let (window, _client) = fixture.open().await;
    postio_gtk::list::feed::stage_results_view(false);
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 2).await,
        "the inbox never reached the screen"
    );
    support::press(&window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    bar.set_text("report");
    assert!(
        crate::settle_until(async || bar
            .dropdown()
            .texts()
            .iter()
            .any(|text| text.contains(postio_ui::search_view::SHOW_ALL_DETAIL)))
        .await,
        "no row to show the results for report: {:?}",
        bar.dropdown().texts()
    );
    bar.run_search_row();
    assert!(
        crate::settle_until(async || window.results().is_some()).await,
        "Show all did not open the results"
    );
    let results = window.results().expect("the results are up");
    (window, fixture, results)
}

/// The Files tab draws the PDF's card; Save asks where, writes the file's
/// own bytes there, and the copy made for it goes.
pub fn the_files_tab_saves_a_file_where_the_person_says() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture, results) = report_results().await;
        let place = tempfile::tempdir().expect("a directory");
        let to = place.path().join("saved.pdf");
        let offered = Rc::new(RefCell::new(None));
        window.set_file_picker({
            let to = to.clone();
            let offered = Rc::clone(&offered);
            move |pick, answer| {
                if let postio_gtk::chooser::SavePick::File { suggested } = pick {
                    offered.replace(Some(suggested));
                }
                answer(Some(to.clone()));
            }
        });
        assert!(
            crate::settle_until(async || results.click_tab("Files")).await,
            "no Files tab"
        );
        assert!(
            crate::settle_until(async || results.files().names() == ["report.pdf"]).await,
            "the Files tab does not show the PDF: {:?}",
            results.files().names()
        );
        assert!(
            crate::settle_until(async || results.files().focused() == Some(0)).await,
            "no ring on the card"
        );
        window.press_command(CommandId::SaveFile);
        assert!(
            crate::settle_until(async || to.exists()).await,
            "Save wrote nothing"
        );
        assert_eq!(offered.borrow().as_deref(), Some("report.pdf"));
        assert_eq!(
            std::fs::read(&to).expect("the saved file"),
            support::ATTACHED,
            "the saved file is not the attachment's bytes"
        );
        assert!(
            crate::settle_until(async || {
                std::fs::read_dir(postio_focus::file_copies())
                    .map(|entries| {
                        entries
                            .flatten()
                            .all(|entry| entry.file_name() != "report.pdf")
                    })
                    .unwrap_or(true)
            })
            .await,
            "the copy made for the save stayed"
        );
    });
}

/// Space on a file opens it in the desktop's app for its type: a copy,
/// handed over as a file URI.
pub fn space_on_a_file_opens_it_in_the_desktop_app() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture, results) = report_results().await;
        let launched = Rc::new(RefCell::new(Vec::<String>::new()));
        window.set_launcher({
            let launched = Rc::clone(&launched);
            move |uri| launched.borrow_mut().push(uri.to_owned())
        });
        assert!(crate::settle_until(async || results.click_tab("Files")).await);
        assert!(crate::settle_until(async || results.files().focused() == Some(0)).await);
        window.press_command(CommandId::QuickLook);
        assert!(
            crate::settle_until(async || !launched.borrow().is_empty()).await,
            "Space opened nothing"
        );
        let uri = launched.borrow()[0].clone();
        assert!(
            uri.starts_with("file://") && uri.ends_with("report.pdf"),
            "not the file's copy: {uri}"
        );
    });
}

/// Space on a conversation opens Quick Look over the results, its match
/// cards with one ringed; Space again closes it.
pub fn quick_look_shows_a_conversation_s_matches() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture, results) = report_results().await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "no ring on the results"
        );
        window.press_command(CommandId::QuickLook);
        let panel = results.quick_look();
        assert!(
            crate::settle_until(async || panel.is_open() && !panel.cards().is_empty()).await,
            "Quick Look did not open with its matches: {:?}",
            panel.cards()
        );
        assert_eq!(
            panel.cards().iter().filter(|(_, current)| *current).count(),
            1,
            "not one card ringed: {:?}",
            panel.cards()
        );
        window.press_command(CommandId::QuickLook);
        assert!(
            crate::settle_until(async || !panel.is_open()).await,
            "Space again did not close Quick Look"
        );
    });
}
