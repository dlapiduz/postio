//! Saving attachments (T240, row 37): the `o` chooser offers Save and Save
//! all through the file-chooser portal, and a click on an attachment chip
//! opens that chooser at the part. The portal is a seam here -- a case says
//! where the person chose, and what was asked of it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use postio_gtk::chooser::SavePick;

use crate::support::{self, Fixture};

/// A window with one message, `Invoice`, open and drawn; the portal answers
/// `chosen` to each ask, and every ask is kept.
async fn opened(
    fixture: &Fixture,
    chosen: Option<PathBuf>,
) -> (postio_gtk::window::FocusWindow, Rc<RefCell<Vec<SavePick>>>) {
    fixture.file_with_kept_attachment("Invoice").await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 1).await,
        "the inbox never reached the screen"
    );
    let asked: Rc<RefCell<Vec<SavePick>>> = Rc::default();
    window.set_file_picker({
        let asked = Rc::clone(&asked);
        move |pick, answer| {
            asked.borrow_mut().push(pick);
            answer(chosen.clone());
        }
    });
    support::keys(&window, &["j"]);
    support::press(&window, "Return", gtk::gdk::ModifierType::empty());
    let reading = window.reading().expect("open");
    assert!(
        crate::settle_until(async || reading.body_text().contains("See attached")
            && !reading.attachment_chips().is_empty())
        .await,
        "the message never drew with its chip"
    );
    (window, asked)
}

fn dialog(window: &postio_gtk::window::FocusWindow) -> adw::Dialog {
    window.visible_dialog().expect("the chooser is up")
}

async fn written(path: &std::path::Path) -> Option<Vec<u8>> {
    crate::settle_until(async || path.exists()).await;
    std::fs::read(path).ok()
}

pub fn save_in_the_chooser_writes_the_part_where_the_person_chose() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let into = tempfile::tempdir().expect("a directory");
        let target = into.path().join("chosen name.pdf");
        let (window, asked) = opened(&fixture, Some(target.clone())).await;
        support::keys(&window, &["o"]);
        assert!(
            crate::settle_until(async || !window.choices_shown().is_empty()).await,
            "o offered nothing"
        );
        assert!(asked.borrow().is_empty(), "the portal opened before Save");
        let dialog = dialog(&window);
        let save = support::button_labelled(&dialog, "Save");
        support::click(&window, &save, 1);
        assert_eq!(
            *asked.borrow(),
            [SavePick::File {
                suggested: "report.pdf".to_owned()
            }],
            "Save asked the portal for a file, named as the part is"
        );
        assert_eq!(
            written(&target).await.as_deref(),
            Some(support::ATTACHED),
            "the part is not the bytes the sender attached"
        );
    });
}

pub fn save_all_in_the_chooser_writes_every_part_into_the_chosen_folder() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let into = tempfile::tempdir().expect("a directory");
        let (window, asked) = opened(&fixture, Some(into.path().to_path_buf())).await;
        support::keys(&window, &["o"]);
        assert!(crate::settle_until(async || !window.choices_shown().is_empty()).await);
        let save_all = support::button_labelled(&dialog(&window), "Save all");
        support::click(&window, &save_all, 1);
        assert_eq!(*asked.borrow(), [SavePick::Folder]);
        assert_eq!(
            written(&into.path().join("report.pdf")).await.as_deref(),
            Some(support::ATTACHED)
        );
    });
}

pub fn a_cancelled_portal_writes_nothing() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, asked) = opened(&fixture, None).await;
        support::keys(&window, &["o"]);
        assert!(crate::settle_until(async || !window.choices_shown().is_empty()).await);
        let save = support::button_labelled(&dialog(&window), "Save");
        support::click(&window, &save, 1);
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        assert_eq!(asked.borrow().len(), 1, "Save did not ask");
        assert!(
            window.toast_showing().is_none(),
            "a cancelled choice is not an error: {:?}",
            window.toast_showing()
        );
    });
}

/// A chip opens the chooser at its part, in the dialog and in the pane,
/// and asks the portal for nothing until Save is pressed.
pub fn a_click_on_a_chip_shows_the_chooser_at_that_part() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        for pane in [false, true] {
            let fixture = Fixture::empty().await;
            let into = tempfile::tempdir().expect("a directory");
            let target = into.path().join("report.pdf");
            fixture.file_with_kept_attachment("Invoice").await;
            let (window, _client) = fixture.open().await;
            if pane {
                window.set_focus_config(postio_config::FocusConfig {
                    reading: postio_config::Reading::Pane,
                    ..postio_config::FocusConfig::default()
                });
                crate::settle();
            }
            assert!(crate::settle_until(async || support::subjects(&window).len() == 1).await);
            let asked: Rc<RefCell<Vec<SavePick>>> = Rc::default();
            window.set_file_picker({
                let asked = Rc::clone(&asked);
                let target = target.clone();
                move |pick, answer| {
                    asked.borrow_mut().push(pick);
                    answer(Some(target.clone()));
                }
            });
            support::keys(&window, &["j"]);
            support::press(&window, "Return", gtk::gdk::ModifierType::empty());
            let reading = window.reading().expect("open");
            assert!(
                crate::settle_until(async || !reading.attachment_chips().is_empty()).await,
                "no chip (pane: {pane})"
            );
            let chip = reading.attachment_chips().remove(0);
            support::click(&window, &chip, 1);
            assert!(
                crate::settle_until(async || window
                    .choices_shown()
                    .iter()
                    .any(|(name, _)| name == "report.pdf"))
                .await,
                "the chip did not show the chooser (pane: {pane})"
            );
            assert_eq!(window.choice_focused().as_deref(), Some("report.pdf"));
            assert!(asked.borrow().is_empty(), "the portal opened on a click");
            let save = support::button_labelled(&window.visible_dialog().expect("up"), "Save");
            support::click(&window, &save, 1);
            assert_eq!(written(&target).await.as_deref(), Some(support::ATTACHED));
        }
    });
}
