use adw::prelude::*;

use crate::support::{self, Fixture};

fn enter(window: &postio_gtk::window::FocusWindow) {
    support::press(window, "Return", gtk::gdk::ModifierType::empty());
}

pub(crate) async fn opened(fixture: &Fixture, rows: usize) -> postio_gtk::window::FocusWindow {
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == rows).await,
        "the inbox never reached the screen"
    );
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

pub(crate) fn scroller_of(widget: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    widget
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("inside a scroller")
}

fn column_child(view: &impl IsA<gtk::Widget>) -> gtk::Widget {
    scroller_of(view).child().expect("the column has a child")
}

async fn long_message(fixture: &Fixture) {
    let (message, _) = fixture
        .file(("Ada Moreno", "ada@example.com"), "Long", "A line.", 10)
        .await;
    let body = "A line long enough to scroll. ".repeat(600);
    fixture.write_body(message, &body).await;
}

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
        // The first document drawn can be the body before its last layout pass; the column's height
        // follows a frame later.
        let _ = crate::settle_until(async || {
            view.document()
                .is_some_and(|document| document.size.height > 4.0 * 512.0)
                && column.vadjustment().upper() > 2.0 * column.vadjustment().page_size()
        })
        .await;
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

pub fn the_column_draws_only_the_window_of_a_long_body() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        long_message(&fixture).await;
        let window = opened(&fixture, 1).await;
        let view = window.reading().expect("open").reader().view().clone();
        // The case is about the long body: wait past the first document, and for the tiles
        // rasterised a frame later.
        let _ = crate::settle_until(async || {
            view.document()
                .is_some_and(|document| document.size.height > 4.0 * 512.0)
                && view.tile_bytes() > 0
        })
        .await;
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
        let measure = f64::from(reading.reader().view().width());
        assert!(
            right > 0.9 * measure && right <= measure + 0.5,
            "the plain body's lines run to {right}px; the column is {measure}px"
        );

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.body_text().contains("A sale on now")).await,
            "the second message never arrived"
        );
        // A page of its own is drawn as sent on a sheet: its text sits 8px in, the default page
        // margin its sender did not override.
        let (left, _) = extent();
        assert!(
            left >= 8.0,
            "a message with a page of its own is not inset as a page: text at {left}px"
        );
    });
}

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

        let name = support::only(&card, "focus-open-sender-name");
        assert!(
            name.pango_context()
                .font_description()
                .is_some_and(|font| font.weight() >= gtk::pango::Weight::Semibold),
            "the sender's name is not bold"
        );
        let address = support::with_class(&card, "focus-open-address");
        assert!(!address.is_empty(), "the addresses are not marked");
        assert!(
            address.iter().any(|label| label
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == "lena@example.org")),
            "the sender's address is not drawn bare: {:?}",
            support::texts(&card)
        );
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
            .format("%-d %b %Y, %H:%M")
            .to_string();
        assert_eq!(date, full);
    });
}

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
        let face = |label: &gtk::Label| {
            label
                .pango_context()
                .font_description()
                .and_then(|font| font.family())
                .map(|family| family.to_lowercase())
                .unwrap_or_default()
        };
        assert!(
            !face(&name).contains("mono"),
            "the name is drawn in {:?}",
            face(&name)
        );
        assert!(
            face(&size).contains("mono"),
            "the size is drawn in {:?}",
            face(&size)
        );
        let y = |widget: &gtk::Widget| {
            widget
                .compute_bounds(&dialog.child().expect("content"))
                .expect("laid out")
                .y()
        };
        let middle = |widget: &gtk::Widget| y(widget) + widget.height() as f32 / 2.0;
        assert!(
            (middle(size.upcast_ref()) - middle(name.upcast_ref())).abs() <= 1.0,
            "the size is not beside the name"
        );
        assert_eq!(card.height(), 40, "the chip is not 40px tall");
        let view = reading.reader().view().clone();
        assert!(
            y(&card) >= y(view.upcast_ref()) + view.height() as f32 - 1.0,
            "the card is over the body"
        );
    });
}

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

pub fn close_is_the_header_s_square_and_the_verbs_are_30px() {
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
        assert_eq!(
            (close.width(), close.height()),
            (32, 32),
            "Close is not the header's 32px square"
        );
        assert_eq!(verb.height(), 30, "a verb is not 30px tall");
    });
}

pub fn close_is_an_x_icon_at_the_right_and_the_steps_at_the_left() {
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
        assert!(
            crate::settle_until(async || support::only(&dialog, "focus-open-close").width() > 0)
                .await
        );
        let close = support::only(&dialog, "focus-open-close");
        let close = close.downcast_ref::<gtk::Button>().expect("a button");
        assert_eq!(close.icon_name().as_deref(), Some("window-close-symbolic"));
        assert_eq!(close.tooltip_text().as_deref(), Some("Close"));
        let dialog_widget: gtk::Widget = dialog.clone().upcast();
        let x_of =
            |widget: &gtk::Widget| widget.compute_bounds(&dialog_widget).expect("laid out").x();
        let steps = support::with_class(&dialog, "focus-open-step");
        assert_eq!(steps.len(), 2, "the two step buttons");
        for step in &steps {
            assert!(
                x_of(step) < x_of(close.upcast_ref()),
                "a step button is left of Close"
            );
        }
        let title = support::only(&dialog, "focus-open-title");
        assert!(
            x_of(&steps[0]) < x_of(&title),
            "the steps are left of the title"
        );
        assert!(x_of(&title) < x_of(close.upcast_ref()));
    });
}

pub fn the_steps_carry_their_keys_inside_and_stay_compact() {
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
        assert!(
            crate::settle_until(async || support::only(&dialog, "focus-open-close").width() > 0)
                .await
        );
        let steps = support::with_class(&dialog, "focus-open-step");
        assert_eq!(steps.len(), 2, "the two steps, each one control");
        for (step, key, name) in [
            (&steps[0], "k", "Previous message"),
            (&steps[1], "j", "Next message"),
        ] {
            assert!(step.is::<gtk::Button>(), "{name} is not a button");
            assert_eq!(step.tooltip_text().as_deref(), Some(name));
            let caps: Vec<String> = support::descendants(step)
                .into_iter()
                .filter(|w| w.has_css_class("postio-keyhint") && w.is_visible())
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .map(|label| label.text().to_string())
                .collect();
            assert_eq!(caps, vec![key.to_owned()], "{name}'s key is not inside it");
        }
        let pair = steps[0].parent().expect("the steps sit together");
        assert_eq!(
            steps[1].parent().as_ref(),
            Some(&pair),
            "the steps are not one pair"
        );
        assert!(
            pair.width() <= 88,
            "the steps take {}px of the header",
            pair.width()
        );
    });
}

const QUOTE_WORDS: [&str; 12] = [
    "zephyr", "quartz", "lantern", "marble", "orchid", "saffron", "tundra", "walnut", "yarrow",
    "ember", "falcon", "garnet",
];

/// Done with the keyboard alone (Tab to the fold line, Return), as a person without a pointer
/// would: the Mac's WKWebView body once opened its fold and kept its old height (#1897).
pub fn opening_the_quoted_fold_grows_the_body_and_shows_the_quote() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Re: Plan", "Agreed.", 10)
            .await;
        let quoted: String = QUOTE_WORDS
            .iter()
            .cycle()
            .take(60)
            .map(|word| format!("> {word} {word} {word}\n"))
            .collect();
        fixture
            .write_body(
                message,
                &format!("Agreed, thanks.\n\nOn Monday, Ben wrote:\n{quoted}"),
            )
            .await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view().clone();
        let column = scroller_of(&view);
        let fold_line = support::only(&reading.dialog(), "focus-open-fold-line");
        assert!(
            crate::settle_until(async || fold_line.is_mapped()).await,
            "a reply with quoted history shows no fold line"
        );
        let drawn = |word: &str| {
            view.document()
                .is_some_and(|document| !document.text.find(word).is_empty())
        };
        assert!(!drawn("zephyr"), "the quote is drawn while folded");
        let before = (
            view.document().expect("drawn").size.height,
            view.height(),
            column.vadjustment().upper(),
        );

        fold_line.grab_focus();
        assert!(
            fold_line.is_focus(),
            "the fold line cannot take the keyboard"
        );
        // Return on a focused button is its `activate` action; the harness's key delivery runs key
        // controllers only.
        assert!(fold_line.activate(), "the fold line is not activatable");
        assert!(
            crate::settle_until(async || drawn("zephyr") && drawn("garnet")).await,
            "Activating the fold line left the quoted words undrawn"
        );
        assert!(
            crate::settle_until(async || view.tiles_settled()).await,
            "the opened quote was never drawn"
        );
        let document = view.document().expect("drawn");
        assert!(
            document.size.height > before.0 + 100.0,
            "the document is {} tall, was {}: the fold opened but nothing grew",
            document.size.height,
            before.0
        );
        assert!(
            crate::settle_until(async || f64::from(view.height()) >= document.size.height.floor())
                .await,
            "the body widget is {}px in a {}px document: the quote is clipped (was {}px)",
            view.height(),
            document.size.height,
            before.1
        );
        assert!(
            crate::settle_until(async || column.vadjustment().upper() > before.2 + 100.0).await,
            "the column scrolls {} as before ({}): the quote cannot be reached",
            column.vadjustment().upper(),
            before.2
        );
        assert!(
            !fold_line.is_visible(),
            "the fold line still offers the fold it just opened"
        );
    });
}

pub fn ctrl_c_copies_what_was_dragged_across_the_body() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Directions",
                "Gate code",
                10,
            )
            .await;
        // Far enough down a long body that the column must scroll to reach it: taking the selection
        // must not move the page.
        let body = format!(
            "{}The gate code is 4417 and the door sticks. {}",
            "A line long enough to scroll. ".repeat(300),
            "A line long enough to scroll. ".repeat(300),
        );
        fixture.write_body(message, &body).await;
        let window = opened(&fixture, 1).await;
        let reading = window.reading().expect("open");
        let view = reading.reader().view();
        assert!(
            crate::settle_until(async || view
                .document()
                .is_some_and(|document| !document.text.find("4417").is_empty()))
            .await,
            "the body never laid out as far as the gate code"
        );
        let document = view.document().expect("a snapshot");
        let code = document.text.find("4417")[0].clone();
        let rect = document.text.rects(code)[0];
        let column = scroller_of(view).vadjustment();
        let from_top = view
            .compute_point(&column_child(view), &gtk::graphene::Point::zero())
            .map_or(0.0, |at| f64::from(at.y()));
        column.set_value(from_top + rect.y0 - column.page_size() / 2.0);
        let last = std::cell::Cell::new(column.value());
        assert!(
            crate::settle_until(async || {
                crate::settle();
                let now = column.value();
                now == last.replace(now)
            })
            .await,
            "the page never came to rest"
        );
        let settled = column.value();
        let scrolled = |adjustment: Option<gtk::Adjustment>| adjustment.map_or(0.0, |a| a.value());
        let y = (rect.center().y - scrolled(view.vadjustment())) as f32;
        let x = |at: f64| (at - scrolled(view.hadjustment())) as f32;
        view.drag_select(
            gtk::graphene::Point::new(x(rect.x0 + 0.5), y),
            gtk::graphene::Point::new(x(rect.x1 - 0.5), y),
        );
        let selected = view.selection().expect("the drag selected something");
        assert_eq!(document.text.slice(selected), "4417");

        let clipboard = gtk::gdk::Display::default().expect("a display").clipboard();
        clipboard.set_text("something else");
        support::press(&window, "c", gtk::gdk::ModifierType::CONTROL_MASK);
        let text = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = text.clone();
        clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
            *slot.borrow_mut() = Some(result.ok().flatten().map(|s| s.to_string()));
        });
        assert!(crate::settle_until(async || text.borrow().is_some()).await);
        assert_eq!(
            text.borrow().clone().flatten().as_deref(),
            Some("4417"),
            "Ctrl+C did not copy the selection; the keyboard was on a {}",
            gtk::prelude::GtkWindowExt::focus(&window)
                .map(|widget| widget.type_().name().to_owned())
                .unwrap_or_default()
        );
        crate::settle();
        assert_eq!(column.value(), settled, "selecting moved the page");
    });
}
