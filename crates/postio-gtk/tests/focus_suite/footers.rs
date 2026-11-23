use gtk::prelude::*;
use postio_model::DraftState;

use crate::support::{self, Fixture};

fn drawn(window: &postio_gtk::window::FocusWindow) -> Vec<(String, Vec<String>)> {
    support::with_class(window, "focus-scope-footer")
        .into_iter()
        .filter(|footer| footer.is_mapped())
        .map(|footer| {
            let title = support::with_class(&footer, "focus-scope-footer-title")
                .first()
                .and_then(|title| title.downcast_ref::<gtk::Label>().map(|l| l.text()))
                .map(|text| text.to_string())
                .unwrap_or_default();
            // The pairs are the second child: a keycap, then its words.
            let mut labels = Vec::new();
            let mut pair = footer
                .first_child()
                .and_then(|title| title.next_sibling())
                .and_then(|keys| keys.first_child());
            while let Some(widget) = pair {
                if let Some(label) = widget
                    .last_child()
                    .and_then(|label| label.downcast::<gtk::Label>().ok())
                {
                    labels.push(label.text().to_string());
                }
                pair = widget.next_sibling();
            }
            (title, labels)
        })
        .collect()
}

async fn shows(window: &postio_gtk::window::FocusWindow, labels: &[&str]) -> String {
    assert!(
        crate::settle_until(async || {
            let drawn = drawn(window);
            drawn.len() == 1 && drawn[0].1 == labels
        })
        .await,
        "the footer drawn is not {labels:?}: {:?}",
        drawn(window)
    );
    drawn(window).remove(0).0
}

async fn shows_none(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || drawn(window).is_empty()).await,
        "a footer is drawn: {:?}",
        drawn(window)
    );
}

const ROW: &[&str] = &["move", "open", "reply", "forward", "archive"];

async fn two_alone() -> (Fixture, postio_gtk::window::FocusWindow) {
    let fixture = Fixture::empty().await;
    fixture
        .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
        .await;
    fixture
        .file(("Ben Okafor", "ben@example.com"), "Harbor", "Plans.", 10)
        .await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 2).await,
        "the inbox never reached the screen"
    );
    assert!(
        crate::settle_until(async || window.composer().is_some()).await,
        "Focus mounted no composer"
    );
    (fixture, window)
}

pub fn a_message_alone_has_its_own_footer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = two_alone().await;
        let row = shows(&window, ROW).await;
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|reading| reading.is_open()))
                .await,
            "Return opened no message"
        );
        let title = shows(&window, &["reply", "forward", "archive"]).await;
        assert_eq!(title, row, "the footer names the message shown");
        support::deliver(&window, "Escape");
        assert_eq!(shows(&window, ROW).await, row, "the row's footer is back");
    });
}

pub fn a_waiting_send_offers_what_settles_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _draft) =
            crate::sending_states::with_a_draft(DraftState::Queued).await;
        crate::sending_states::go_to(&window, "Outbox").await;
        crate::sending_states::open_the_draft(&window).await;
        let _reading = crate::sending_states::reading(&window).await;
        let title = shows(&window, &["cancel send", "edit"]).await;
        assert_eq!(title, crate::sending_states::SUBJECT);
    });
}

pub fn the_composer_has_no_footer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = two_alone().await;
        let _ = shows(&window, ROW).await;
        assert!(support::deliver(&window, "c"), "nothing took c");
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        shows_none(&window).await;
        support::deliver(&window, "Escape");
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Escape left the composer open"
        );
        let _ = shows(&window, ROW).await;

        support::deliver(&window, "Return");
        let _ = shows(&window, &["reply", "forward", "archive"]).await;
        support::deliver(&window, "r");
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "r opened no reply"
        );
        shows_none(&window).await;
        support::deliver(&window, "Escape");
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Escape left the reply open"
        );
        let _ = shows(&window, &["reply", "forward", "archive"]).await;
    });
}

pub fn settings_has_no_footer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = two_alone().await;
        let _ = shows(&window, ROW).await;
        support::deliver_with(&window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settings::settings_shown(&window).await.is_some(),
            "Settings opened"
        );
        shows_none(&window).await;
        support::deliver(&window, "Escape");
        let _ = shows(&window, ROW).await;
    });
}

pub fn the_capture_sheet_has_no_footer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let vault = crate::capture::TempVault::new();
        let fixture = Fixture::empty().await;
        crate::capture::harbor_to_do(&fixture).await;
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt(
            &window,
            fixture.host(),
            &vault.config(),
        ));
        assert!(crate::settle_until(async || !support::subjects(&window).is_empty()).await);
        support::keys(&window, &["j", "Return"]);
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|reading| reading.is_open()))
                .await,
            "Return opened no message"
        );
        let message = shows(&window, &["reply", "forward", "archive"]).await;
        support::keys(&window, &["T"]);
        assert!(
            crate::settle_until(async || window.capture().is_some_and(|sheet| sheet.is_shown()))
                .await,
            "T did not show capture"
        );
        shows_none(&window).await;
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.capture().is_some_and(|sheet| !sheet.is_open()))
                .await,
            "Escape left capture open"
        );
        assert_eq!(
            shows(&window, &["reply", "forward", "archive"]).await,
            message,
            "the message's footer is back"
        );
    });
}
