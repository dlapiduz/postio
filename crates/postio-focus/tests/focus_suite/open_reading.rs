//! The open message read as one column (screen 04; T183 to T187): what the
//! dialog draws from its header down to the fold line, measured on the
//! screen -- where things are and how tall -- and not on what a layer was
//! handed.

use adw::prelude::*;
use gtk::gdk;

use crate::support::{self, Fixture};

fn enter(window: &postio_focus::window::FocusWindow) {
    let _ = window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
}

/// A window over the fixture's mail, the cursor on the first row, its
/// message open and its body drawn.
async fn opened(fixture: &Fixture, rows: usize) -> postio_focus::window::FocusWindow {
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == rows).await,
        "the inbox never reached the screen"
    );
    support::keys(&window, &["j"]);
    enter(&window);
    let reading = window.reading().expect("Enter opened the message");
    assert!(
        crate::settle_until(async || reading.reader().view().document().is_some()
            && !reading.body_text().is_empty())
        .await,
        "the body never arrived"
    );
    assert!(
        crate::settle_until(async || reading.reader().view().tiles_settled()).await,
        "the body was never drawn"
    );
    window
}

/// The scroller nearest above `widget`.
fn scroller_of(widget: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    widget
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("inside a scroller")
}

/// A message with a long plain body.
async fn long_message(fixture: &Fixture) {
    let (message, _) = fixture
        .file(("Ada Moreno", "ada@example.com"), "Long", "A line.", 10)
        .await;
    let body = "A line long enough to scroll. ".repeat(600);
    fixture.write_body(message, &body).await;
}

/// T183: the header, the body and the fold line scroll together in one
/// column; the body is not a scroller inside it, and is as tall as its
/// words.
pub fn the_open_message_is_one_scrolling_column() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        long_message(&fixture).await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();

        let view = reading.reader().view().clone();
        let column = scroller_of(&view);
        let subject = support::only(&dialog, "focus-open-subject");
        assert_eq!(
            scroller_of(&subject),
            column,
            "the subject and the body are in different scrollers"
        );
        let fold_line = support::only(&dialog, "focus-open-fold-line");
        assert_eq!(scroller_of(&fold_line), column, "the fold line is outside");
        let document = view.document().expect("drawn");
        assert!(
            f64::from(view.height()) >= document.size.height.floor(),
            "the body is {}px in a {}px document: a window of its own",
            view.height(),
            document.size.height
        );
        let adjustment = column.vadjustment();
        assert!(
            adjustment.upper() > 2.0 * adjustment.page_size(),
            "the column does not scroll: {} in {}",
            adjustment.upper(),
            adjustment.page_size()
        );
    });
}

/// T183: the column is still windowed -- only what the scroller shows is
/// rasterised, however tall the message.
pub fn the_column_draws_only_the_window_of_a_long_body() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        long_message(&fixture).await;
        let window = opened(&fixture, 1).await;
        let view = window.reading().expect("open").reader().view().clone();
        let document = view.document().expect("drawn");
        let whole = (document.size.width * document.size.height * 4.0) as usize;
        assert!(
            document.size.height > 4.0 * 512.0,
            "the body is too short to tell: {}px",
            document.size.height
        );
        assert!(
            view.tile_bytes() > 0 && view.tile_bytes() < whole / 2,
            "{} bytes of tiles held for a {whole} byte document",
            view.tile_bytes()
        );
    });
}

/// T183: correspondence is drawn flat on the column, at the reading
/// measure; a message that paints a page of its own keeps a frame.
pub fn a_plain_body_has_no_frame_and_a_page_of_its_own_keeps_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (plain, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Plain", "x", 10)
            .await;
        fixture
            .write_body(plain, &"Some words in a plain message. ".repeat(80))
            .await;
        let (page, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Paged", "x", 20)
            .await;
        fixture
            .write_html_body(
                page,
                "<html><body style=\"background:#fff4e0\"><p>A sale on now</p></body></html>",
            )
            .await;
        let window = opened(&fixture, 2).await;
        let reading = window.reading().expect("open");
        let extent = || {
            let document = reading.reader().view().document().expect("drawn");
            let left = document
                .text
                .clusters
                .iter()
                .map(|cluster| cluster.rect.x0)
                .fold(f64::MAX, f64::min);
            let right = document
                .text
                .clusters
                .iter()
                .map(|cluster| cluster.rect.x1)
                .fold(0.0, f64::max);
            (left, right)
        };
        assert_eq!(reading.title(), "Plain");
        let (left, right) = extent();
        assert!(left < 1.0, "the plain body is inset {left}px: a frame");
        assert!(
            (600.0..=700.5).contains(&right),
            "the plain body's lines run to {right}px, not the reading measure"
        );

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.body_text().contains("A sale on now")).await,
            "the second message never arrived"
        );
        let (left, _) = extent();
        assert!(
            left >= 16.0,
            "a message with a page of its own has no frame: text at {left}px"
        );
    });
}

/// T184: the header card names who a message is from, who it went to and
/// who was copied, with the sender's name apart from their address, and
/// dates today's mail relatively.
pub fn the_header_card_names_from_to_and_cc_and_dates_today_relatively() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let today = chrono::Utc::now() - chrono::Duration::minutes(1);
        fixture
            .file_addressed(
                ("Lena Park", "lena@example.org"),
                &[
                    ("Ben Adeyemi", "ben@example.org"),
                    ("Grace Oyelaran", "grace@example.org"),
                ],
                &[("Harbor API", "harbor-api@example.org")],
                "Copied",
                today,
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let card = support::only(&dialog, "focus-open-header-card");
        let said = support::texts(&card);
        for expected in ["From", "To", "Cc", "Lena Park", "Ben Adeyemi", "Harbor API"] {
            assert!(
                said.iter().any(|text| text.contains(expected)),
                "no {expected:?} in the card: {said:?}"
            );
        }
        let date = support::only(&card, "focus-open-date");
        let date = date.downcast::<gtk::Label>().expect("a label").text();
        let time = today
            .with_timezone(&chrono::Local)
            .format("%H:%M")
            .to_string();
        assert_eq!(date, format!("Today, {time}"));

        // The sender's name is bold and their address is dim and mono.
        let name = support::only(&card, "focus-open-sender-name");
        assert!(
            name.pango_context()
                .font_description()
                .is_some_and(|font| font.weight() >= gtk::pango::Weight::Semibold),
            "the sender's name is not bold"
        );
        let address = support::with_class(&card, "focus-open-address");
        assert!(!address.is_empty(), "the addresses are not marked");
        for label in &address {
            let family = label
                .pango_context()
                .font_description()
                .and_then(|font| font.family())
                .map(|family| family.to_string())
                .unwrap_or_default();
            assert!(
                family.to_lowercase().contains("mono"),
                "an address is drawn in {family:?}, not mono"
            );
        }
    });
}

/// T184: a message nobody was copied on shows no Cc line, and one from the
/// past is dated in full.
pub fn the_header_card_has_no_cc_line_without_cc_and_dates_the_past_in_full() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file_addressed(
                ("Lena Park", "lena@example.org"),
                &[("Ben Adeyemi", "ben@example.org")],
                &[],
                "Not copied",
                support::now(),
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let card = support::only(&reading.dialog(), "focus-open-header-card");
        let said = support::texts(&card);
        assert!(said.iter().any(|text| text == "To"), "no To: {said:?}");
        assert!(
            !said.iter().any(|text| text == "Cc"),
            "a Cc line with nobody copied: {said:?}"
        );
        let date = support::only(&card, "focus-open-date")
            .downcast::<gtk::Label>()
            .expect("a label")
            .text();
        let full = support::now()
            .with_timezone(&chrono::Local)
            .format("%a, %-d %b %Y at %H:%M")
            .to_string();
        assert_eq!(date, full);
    });
}

/// T185: an attachment is a card -- an icon, the name in mono and the size
/// beneath it -- in a row under the body, not a pill over it.
pub fn an_attachment_is_a_card_under_the_body() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let message = fixture
            .file_with_attachment("Attached", "Harbor-API-v3.pdf")
            .await;
        fixture.write_body(message, "See the attached file.").await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let card = support::only(&dialog, "postio-attachment-card");
        assert!(
            crate::settle_until(async || card.is_mapped() && card.height() > 1).await,
            "the card was never drawn"
        );
        let widgets = support::descendants(&card);
        assert!(
            widgets.iter().any(|widget| widget.is::<gtk::Image>()),
            "the card has no file icon"
        );
        let name = support::only(&card, "postio-attachment-name")
            .downcast::<gtk::Label>()
            .expect("a label");
        let size = support::only(&card, "postio-attachment-size")
            .downcast::<gtk::Label>()
            .expect("a label");
        assert_eq!(name.text(), "Harbor-API-v3.pdf");
        assert_eq!(size.text(), "47 KB");
        let family = name
            .pango_context()
            .font_description()
            .and_then(|font| font.family())
            .map(|family| family.to_lowercase())
            .unwrap_or_default();
        assert!(family.contains("mono"), "the name is drawn in {family:?}");
        let y = |widget: &gtk::Widget| {
            widget
                .compute_bounds(&dialog.child().expect("content"))
                .expect("laid out")
                .y()
        };
        assert!(
            y(size.upcast_ref()) > y(name.upcast_ref()),
            "the size is not beneath the name"
        );
        // Under the body, not over it.
        let view = reading.reader().view().clone();
        assert!(
            y(&card) >= y(view.upcast_ref()) + view.height() as f32 - 1.0,
            "the card is over the body"
        );
    });
}

/// T186: a label's pill carries its colour dot, in the label's own colour.
pub fn a_label_pill_carries_its_colour_dot() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Labelled", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        fixture.label_in(message, "Harbor", "#3a7d44").await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let pill = support::only(&reading.dialog(), "focus-open-label-pill");
        let dots = support::with_class(&pill, "focus-open-label-dot");
        assert_eq!(dots.len(), 1, "the pill has {} dots", dots.len());
        assert!(
            dots[0].width() > 0 && dots[0].height() > 0,
            "the dot has no size"
        );
        assert_eq!(
            reading.label_dots(),
            vec![("Harbor".to_owned(), "#3a7d44".to_owned())],
            "the dot is not the label's colour"
        );
    });
}

/// T187: Close is the same compact pill as the toolbar's buttons.
pub fn close_is_as_compact_as_the_toolbar_s_buttons() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let close = support::only(&dialog, "focus-open-close");
        let verb = support::only(&dialog, "focus-open-archive");
        assert!(
            close.height() <= verb.height(),
            "Close is {}px tall beside a toolbar button's {}px",
            close.height(),
            verb.height()
        );
    });
}
