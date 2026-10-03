//! The composer is the message dialog's family (T221; screens.md, "The
//! composer"): its size rule, a header of detach, title and the shared X, an
//! action row for its verbs with Send first and the only primary, short
//! keycaps, one column for the fields and the editor, and the editor drawn
//! on the dialog's own surface.
//!
//! Opened with `c` delivered as GTK delivers a key, and measured on the
//! screen: what a person sees is the allocated widgets, so that is what is
//! read.

use adw::prelude::*;
use gtk::gdk;
use postio_ui::focus_dialog;

use crate::support::{self, Fixture};

/// A window over one message with the composer open over it, `size` big
/// (or as the compositor gives it), once the dialog has finished opening.
async fn composing(size: Option<(i32, i32)>) -> (postio_gtk::window::FocusWindow, adw::Dialog) {
    let fixture = Fixture::empty().await;
    fixture
        .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
        .await;
    let (window, _client) = fixture.open_sized(size).await;
    support::keep(fixture);
    if size.is_some() {
        window.unmaximize();
        let asked = window.default_size();
        assert!(
            crate::settle_until(async || !window.is_maximized()
                && window.width() == asked.0
                && window.height() == asked.1)
            .await,
            "the window never came back to {asked:?}"
        );
    }
    assert!(
        crate::settle_until(async || window.composer().is_some()).await,
        "Focus mounted no composer"
    );
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 1).await,
        "the inbox never reached the screen"
    );
    assert!(support::deliver(&window, "c"), "nothing took c");
    assert!(
        crate::settle_until(async || window.compose_dialog().is_some()).await,
        "c opened no composer"
    );
    let dialog = window.compose_dialog().expect("the compose dialog");
    let content = dialog.child().expect("the dialog's content");
    assert!(
        crate::settle_until(async || content.width() > 0
            && content
                .compute_bounds(&window)
                .is_some_and(|bounds| (bounds.width() - content.width() as f32).abs() < 0.5))
        .await,
        "the dialog never finished opening"
    );
    crate::settle();
    (window, dialog)
}

/// The one widget wearing `class` under `root`, on screen.
fn shown(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Widget {
    let widget = support::only(root, class);
    assert!(widget.is_mapped(), "{class} is not on screen");
    widget
}

/// `widget`'s bounds in `root`'s coordinates.
fn bounds(widget: &impl IsA<gtk::Widget>, root: &impl IsA<gtk::Widget>) -> gtk::graphene::Rect {
    widget
        .compute_bounds(root)
        .expect("laid out in the same tree")
}

/// Whether `widget` is `ancestor` or inside it.
fn within(widget: &gtk::Widget, ancestor: &gtk::Widget) -> bool {
    widget == ancestor || widget.is_ancestor(ancestor)
}

/// The text of every keycap under `root` that is on screen.
fn caps(root: &impl IsA<gtk::Widget>) -> Vec<String> {
    support::with_class(root, "postio-keyhint")
        .into_iter()
        .chain(support::with_class(root, "postio-key"))
        .filter(gtk::Widget::is_mapped)
        .filter_map(|cap| cap.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

/// The header holds three things -- Detach at the left, the title and its
/// subtitle centred on the dialog, the shared X at the right -- and the
/// verbs are a row of their own under it, Send first, as Reply is first in
/// the message dialog's. There is no footer.
pub fn the_header_is_detach_title_close_and_the_verbs_have_a_row_of_their_own() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_window, dialog) = composing(None).await;
        let header = shown(&dialog, "focus-compose-header")
            .downcast::<gtk::CenterBox>()
            .expect("the header is a centre box");
        let start = header.start_widget().expect("something at the left");
        let detach = shown(&dialog, "focus-compose-detach");
        assert!(
            within(&detach, &start),
            "Detach is not at the header's left"
        );
        assert!(
            detach.has_css_class("postio-icon-button"),
            "Detach is not the shared icon button"
        );
        let centre = header.center_widget().expect("something centred");
        let title = shown(&dialog, "focus-compose-title");
        assert!(within(&title, &centre), "the title is not the centre");
        assert_eq!(
            title.downcast_ref::<gtk::Label>().map(|label| label.text()),
            Some("New message".into())
        );
        let subtitle = shown(&dialog, "focus-compose-subtitle");
        assert!(
            within(&subtitle, &centre),
            "the subtitle is not under the title"
        );
        assert!(
            crate::settle_until(async || support::texts(&subtitle)
                .iter()
                .any(|text| text.starts_with("Plain text \u{b7} 0 words")))
            .await,
            "the subtitle does not say what will be sent: {:?}",
            support::texts(&subtitle)
        );
        let end = header.end_widget().expect("something at the right");
        let close = shown(&dialog, "focus-compose-close");
        assert!(within(&close, &end), "the X is not at the header's right");
        // Nothing else: no verb and no keycap in the header.
        for class in [
            "focus-compose-send",
            "focus-compose-send-later",
            "focus-compose-attach",
            "focus-compose-remind",
        ] {
            assert!(
                !within(&shown(&dialog, class), header.upcast_ref()),
                "{class} is in the header"
            );
        }
        assert!(
            caps(&header).is_empty(),
            "the header draws keycaps: {:?}",
            caps(&header)
        );
        // The title is centred on the dialog, whatever is beside it.
        let content = dialog.child().expect("content");
        let middle = bounds(&title, &content);
        let centre_x = middle.x() + middle.width() / 2.0;
        assert!(
            (centre_x - content.width() as f32 / 2.0).abs() <= 1.0,
            "the title's centre is at {centre_x}, the dialog's at {}",
            content.width() as f32 / 2.0
        );

        // The action row: under the header, the four verbs in order.
        let row = shown(&dialog, "focus-compose-actions");
        assert!(
            bounds(&row, &content).y()
                >= bounds(&header, &content).y() + bounds(&header, &content).height() - 0.5,
            "the action row is not under the header"
        );
        let xs: Vec<f32> = [
            "focus-compose-send",
            "focus-compose-send-later",
            "focus-compose-attach",
            "focus-compose-remind",
        ]
        .iter()
        .map(|class| {
            let button = shown(&dialog, class);
            assert!(within(&button, &row), "{class} is not in the action row");
            bounds(&button, &content).x()
        })
        .collect();
        assert!(
            xs.windows(2).all(|pair| pair[0] < pair[1]),
            "the verbs are not Send, Send later, Attach, Remind left to right: {xs:?}"
        );
        assert!(
            support::with_class(&dialog, "focus-compose-footer")
                .iter()
                .all(|footer| !footer.is_mapped()),
            "a footer is still drawn"
        );
    });
}

/// Send is the dialog's one primary button, and in Focus a primary is a
/// plain raised button (FR-091): nothing in the composer wears libadwaita's
/// accent-filled `suggested-action`.
pub fn send_is_the_one_primary_and_nothing_wears_the_accent() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_window, dialog) = composing(None).await;
        let send = shown(&dialog, "focus-compose-send");
        let primaries: Vec<gtk::Widget> = support::with_class(&dialog, "postio-button-primary")
            .into_iter()
            .filter(gtk::Widget::is_mapped)
            .collect();
        assert_eq!(
            primaries,
            vec![send.clone()],
            "the primaries on screen are {:?}",
            primaries.iter().map(support::texts).collect::<Vec<_>>()
        );
        let accented: Vec<Vec<String>> = support::descendants(&dialog)
            .iter()
            .filter(|widget| widget.is_mapped() && widget.has_css_class("suggested-action"))
            .map(support::texts)
            .collect();
        assert!(accented.is_empty(), "{accented:?} wear suggested-action");
        for class in [
            "focus-compose-send-later",
            "focus-compose-attach",
            "focus-compose-remind",
        ] {
            assert!(
                !shown(&dialog, class).has_css_class("postio-button-primary"),
                "{class} is drawn as a primary"
            );
        }
    });
}

/// The keycaps are the keymap's spelling, compacted as the message
/// dialog's `Del` is: `Return` is `↵` and `shift` is `⇧`, so no cap is the
/// widest thing in its row.
pub fn the_keycaps_are_short() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_window, dialog) = composing(None).await;
        for (class, cap) in [
            ("focus-compose-send", "ctrl+\u{21b5}"),
            ("focus-compose-send-later", "ctrl+\u{21e7}+\u{21b5}"),
            ("focus-compose-attach", "ctrl+\u{21e7}+a"),
            ("focus-compose-remind", "ctrl+h"),
        ] {
            let button = shown(&dialog, class);
            assert!(
                crate::settle_until(async || !caps(&button).is_empty()).await,
                "{class} has no keycap"
            );
            assert_eq!(caps(&button), vec![cap.to_owned()], "{class}'s keycap");
        }
        for cap in caps(&dialog) {
            assert!(
                cap.chars().count() <= 8,
                "{cap:?} is a long keycap ({} characters)",
                cap.chars().count()
            );
        }
    });
}

/// The field rows share one column with the editor: one left edge and one
/// right edge for every row, the toolbar and the editor; one label column,
/// so every value starts at one x; and the column is the message dialog's
/// app-colours column, centred.
pub fn the_field_rows_share_their_edges() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, dialog) = composing(None).await;
        let content = dialog.child().expect("content");
        let rows: Vec<gtk::Widget> = support::with_class(&dialog, "postio-compose-row")
            .into_iter()
            .filter(gtk::Widget::is_mapped)
            .collect();
        assert!(
            rows.len() >= 4,
            "To, From, Subject and Labels are not all on screen: {}",
            rows.len()
        );
        let column = focus_dialog::column_width(
            focus_dialog::dialog_width(window.width()),
            postio_body::treatment::Treatment::AppColours,
        ) as f32;
        let left = (content.width() as f32 - column) / 2.0;
        let mut label_xs = Vec::new();
        let mut value_xs = Vec::new();
        for row in &rows {
            let at = bounds(row, &content);
            assert!(
                (at.x() - left).abs() <= 1.0 && (at.width() - column).abs() <= 1.0,
                "{:?} runs {}..{}, not the column {left}..{}",
                support::texts(row),
                at.x(),
                at.x() + at.width(),
                left + column
            );
            let mut children = Vec::new();
            let mut child = row.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if widget.is_visible() {
                    children.push(widget);
                }
            }
            let label = children
                .iter()
                .find(|child| child.has_css_class("postio-compose-label"))
                .unwrap_or_else(|| panic!("{:?} has no name", support::texts(row)));
            let value = children
                .iter()
                .skip_while(|child| *child != label)
                .nth(1)
                .unwrap_or_else(|| panic!("{:?} has no value", support::texts(row)));
            label_xs.push(bounds(label, &content).x());
            value_xs.push(bounds(value, &content).x());
        }
        let same = |xs: &[f32]| xs.iter().all(|x| (x - xs[0]).abs() <= 1.0);
        assert!(same(&label_xs), "the names start at {label_xs:?}");
        assert!(same(&value_xs), "the values start at {value_xs:?}");

        for class in ["postio-compose-toolbar", "postio-compose-body"] {
            let at = bounds(&shown(&dialog, class), &content);
            assert!(
                (at.x() - left).abs() <= 1.0 && (at.x() + at.width() - left - column).abs() <= 1.0,
                "{class} runs {}..{}, not the column {left}..{}",
                at.x(),
                at.x() + at.width(),
                left + column
            );
        }
    });
}

/// `colour` as the CSS `rgb(r, g, b)` a document computes.
fn css(colour: [u8; 3]) -> String {
    format!("rgb({}, {}, {})", colour[0], colour[1], colour[2])
}

/// The pixel at (`x`, `y`) of `texture`.
fn pixel(texture: &gdk::Texture, x: i32, y: i32) -> [u8; 3] {
    let (width, height) = (texture.width(), texture.height());
    assert!(
        (0..width).contains(&x) && (0..height).contains(&y),
        "({x}, {y}) is outside the {width}x{height} picture"
    );
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let at = y as usize * stride + x as usize * 4;
    [bytes[at], bytes[at + 1], bytes[at + 2]]
}

/// The editor is drawn on the dialog's own surface, in light and in dark:
/// the document's ground is the colour the dialog paints beside it, and the text starts at the column's edge, under the
/// field names, with no inset of its own.
pub fn the_editor_is_drawn_on_the_dialogs_surface() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let (window, dialog) = composing(None).await;
        let composer = window.composer().expect("the composer");
        let body = shown(&dialog, "postio-compose-body");
        let mut seen = Vec::new();
        for scheme in [adw::ColorScheme::ForceLight, adw::ColorScheme::ForceDark] {
            manager.set_color_scheme(scheme);
            crate::settle_for(std::time::Duration::from_millis(300)).await;
            let picture = postio_widgets::capture::texture(&dialog).expect("the dialog was drawn");
            let at = bounds(&body, &dialog);
            // In the dialog's gutter, beside the editor.
            let around = pixel(
                &picture.texture,
                (at.x() - 12.0) as i32,
                (at.y() + at.height() / 2.0) as i32,
            );
            let want = css(around);
            assert!(
                crate::settle_until(async || composer
                    .test_body_eval("getComputedStyle(document.body).backgroundColor")
                    == want)
                .await,
                "{scheme:?}: the document's ground is {}, the dialog's {want}; its sheet ends {}",
                composer.test_body_eval("getComputedStyle(document.body).backgroundColor"),
                composer.test_body_eval("document.querySelector('style').textContent.slice(-300)")
            );
            seen.push(around);
        }
        manager.set_color_scheme(adw::ColorScheme::Default);
        assert_ne!(seen[0], seen[1], "the dialog did not go dark");
        assert_eq!(
            composer.test_body_eval("getComputedStyle(document.body).paddingLeft"),
            "0px",
            "the text is inset from the column's edge"
        );
    });
}

/// The close is the shared X, at the header's right end, and a click on it
/// -- delivered as GTK delivers one -- closes the composer, keeping the
/// draft.
pub fn the_close_is_the_shared_x() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, dialog) = composing(None).await;
        let close = shown(&dialog, "focus-compose-close");
        assert!(
            close.has_css_class("postio-close-button"),
            "the composer's close is not the shared X"
        );
        let header = shown(&dialog, "focus-compose-header");
        let right = bounds(&close, &header);
        for widget in support::descendants(&header) {
            if widget.is_mapped() && !within(&widget, &close) && !close.is_ancestor(&widget) {
                let at = bounds(&widget, &header);
                assert!(
                    at.x() + at.width() <= right.x() + 0.5 || widget.width() == 0,
                    "{} stands right of the X",
                    widget.type_().name()
                );
            }
        }
        let composer = window.composer().expect("the composer");
        composer.test_set_subject("Kept for later");
        support::click(&window, &close, 1);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "the X did not close the composer"
        );
    });
}

/// The dialog takes the message dialog's size rule from the window, not a
/// size of its own: at 1024 wide it is 655 across and the window less 80
/// tall.
pub fn the_dialog_follows_the_message_dialogs_size_rule() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, dialog) = composing(Some((1024, 700))).await;
        let want = (
            focus_dialog::dialog_width(window.width()),
            focus_dialog::dialog_height(window.height()),
        );
        let content = dialog.child().expect("content");
        assert!(
            crate::settle_until(async || (content.width(), content.height()) == want).await,
            "the composer is {}x{} in a {}x{} window, not {want:?}",
            content.width(),
            content.height(),
            window.width(),
            window.height()
        );
        // And the verbs fit their row at that width, with their keys --
        // with a reminder's day on Remind too.
        let row = shown(&dialog, "focus-compose-actions");
        let fits = |when: &str| {
            let (_, natural, _, _) = row.measure(gtk::Orientation::Horizontal, -1);
            assert!(
                natural <= want.0,
                "{when}, the action row wants {natural}px in a {}px dialog: {:?}",
                want.0,
                support::texts(&row)
            );
        };
        fits("with no reminder");
        let composer = window.composer().expect("the composer");
        composer.set_remind_at(Some(chrono::Utc::now() + chrono::Duration::days(3)));
        crate::settle();
        fits("with a reminder chosen");
        assert_eq!(
            (content.width(), content.height()),
            want,
            "the reminder's day widened the dialog"
        );
    });
}
