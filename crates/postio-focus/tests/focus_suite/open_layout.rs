//! The open-message dialog laid out as the maintainer's handoff draws it
//! (2026-10-01; T205-T209): its size from the window, one centred column
//! every block shares, the handoff's vertical rhythm, and its palette --
//! each measured on what GTK allocated and drew, not on what a layer was
//! handed.

use adw::prelude::*;
use gtk::gdk;
use postio_model::{Attachment, EmailAddress, MessageId};
use postio_storage::repository::MessageRepository;
use postio_ui::focus_dialog::{self, rhythm};

use crate::open_reading::scroller_of;
use crate::support::{self, Fixture};

/// A plain body with every kind of block the handoff draws in it:
/// paragraphs, a list, a sign-off.
const BODY: &str = "Hi all,\n\nUploaded v3 of the Harbor API draft with the \
    pagination changes from Monday's review. The main differences from v2:\n\n\
    - Cursor pagination on every list endpoint, replacing page and offset.\n\
    - Rate-limit headers are documented for every response.\n\
    - The exports endpoint moved under accounts, as Ben suggested.\n\n\
    Please leave comments by Wednesday; I'd like to freeze it on Thursday so \
    Ben can start on the client.\n\nThanks,\nLena";

/// The handoff's sample with every block present: the latest of a thread
/// of two, from Lena Park to two people with one copied, a label, a
/// question marker and two attachments. Answers the message.
async fn every_block(fixture: &Fixture) -> MessageId {
    let ids = fixture.thread_of("Harbor API draft v3", 2, 10).await;
    let latest = *ids.last().expect("a thread");
    dress(fixture, latest, true).await;
    fixture.write_body(latest, BODY).await;
    fixture.label_in(latest, "Harbor", "#3a7d44").await;
    fixture
        .ask(latest, "Please leave comments by Wednesday")
        .await;
    latest
}

/// Address `message` as the handoff's sample is, and attach its two files
/// when `attached`.
async fn dress(fixture: &Fixture, message: MessageId, attached: bool) {
    let connection = fixture.database.connect().await.expect("a connection");
    let repository = MessageRepository::new(&connection);
    let mut row = repository
        .get(message)
        .await
        .expect("the message reads")
        .expect("the message is there");
    row.from = vec![EmailAddress::new(Some("Lena Park"), "lena@example.com")];
    row.to = vec![
        EmailAddress::new(Some("Ada Moreno"), "ada@example.com"),
        EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com"),
    ];
    row.cc = vec![EmailAddress::new(Some("Grace Okafor"), "grace@example.com")];
    if attached {
        for (n, (name, size)) in [
            ("Harbor-API-v3.pdf", 212_000),
            ("harbor-openapi.yaml", 38_000),
        ]
        .into_iter()
        .enumerate()
        {
            let mut part = Attachment::new(MessageId::UNASSIGNED, "application/pdf", size);
            part.filename = Some(name.to_owned());
            part.part_id = Some((n + 2).to_string());
            row.attachments.push(part);
        }
    }
    repository.update(&mut row).await.expect("dressed");
}

/// The window as the suite's compositor gives it: its monitor is 1280x800,
/// and mutter maximises a Focus window onto it, so the handoff's 1440 is
/// out of reach here; the pure functions are proven at 1440 and 1920 in
/// `postio_ui::focus_dialog`, and these cases prove the wiring at the sizes
/// a window can be.
const WIDE: Option<(i32, i32)> = None;
/// A window narrow enough to fold the action row: 1024 wide, as the
/// handoff's screen 07.
const NARROW: Option<(i32, i32)> = Some((1024, 700));

/// Take `window` out of the maximised state the compositor may have put
/// it in, back to the size it asked for, and wait until it is there.
async fn restore(window: &postio_focus::window::FocusWindow) {
    window.unmaximize();
    let asked = window.default_size();
    assert!(
        crate::settle_until(async || !window.is_maximized()
            && window.width() == asked.0
            && window.height() == asked.1)
        .await,
        "the window never came back to {asked:?}: {}x{}",
        window.width(),
        window.height()
    );
}

/// A window over the fixture's mail, `size` big (or as the compositor gives
/// it), the first row open and its body drawn.
async fn opened_at(
    fixture: &Fixture,
    rows: usize,
    size: Option<(i32, i32)>,
) -> postio_focus::window::FocusWindow {
    let (window, _client) = fixture.open_sized(size).await;
    if size.is_some() {
        restore(&window).await;
    }
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == rows).await,
        "the inbox never reached the screen"
    );
    support::keys(&window, &["j"]);
    support::press(&window, "Return", gdk::ModifierType::empty());
    let reading = window.reading().expect("Enter opened the message");
    assert!(
        crate::settle_until(async || reading.reader().view().document().is_some()
            && !reading.body_text().is_empty()
            && reading.reader().view().tiles_settled())
        .await,
        "the body was never drawn"
    );
    // The dialog opens growing to its size: measure it once it is there.
    let content = reading.dialog().child().expect("the dialog's content");
    assert!(
        crate::settle_until(
            async || content
                .compute_bounds(&window)
                .is_some_and(|bounds| (bounds.width() - content.width() as f32).abs() < 0.5)
        )
        .await,
        "the dialog never finished opening"
    );
    crate::settle();
    window
}

/// The dialog's size for `window`'s, by the handoff's rule.
fn fitted(window: &postio_focus::window::FocusWindow) -> (i32, i32) {
    (
        focus_dialog::dialog_width(window.width()),
        focus_dialog::dialog_height(window.height()),
    )
}

/// `widget`'s place in `root`'s coordinates: left, top, right, bottom.
fn edges(widget: &impl IsA<gtk::Widget>, root: &impl IsA<gtk::Widget>) -> (f32, f32, f32, f32) {
    let bounds = widget
        .compute_bounds(root)
        .expect("laid out in the same tree");
    (
        bounds.x(),
        bounds.y(),
        bounds.x() + bounds.width(),
        bounds.y() + bounds.height(),
    )
}

/// The one widget wearing `class` under `root`, on screen.
fn shown(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Widget {
    let widget = support::only(root, class);
    assert!(widget.is_mapped(), "{class} is not on screen");
    widget
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0
}

/// T205: the dialog is as wide and as tall as the window says, centred, and
/// follows the window's resizes -- never the message: `j` keeps its size.
pub fn the_dialog_is_sized_by_the_window_and_never_by_the_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for minutes in [10, 20] {
            let (message, _) = fixture
                .file(("Ada Moreno", "ada@example.com"), "Sized", "x", minutes)
                .await;
            fixture.write_body(message, "A body.").await;
        }
        let window = opened_at(&fixture, 2, NARROW).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        // The dialog's own widget spans the window it is presented over;
        // its content is what is drawn as the dialog.
        let size = |dialog: &adw::Dialog| {
            let content = dialog.child().expect("content");
            (content.width(), content.height())
        };
        assert_eq!(size(&dialog), (655, 620), "in a 1024x700 window");
        let (left, top, right, _) = edges(&dialog.child().expect("content"), &window);
        assert!(
            // 655 is odd: half a pixel each side is as centred as it gets.
            (left - (1024.0 - 655.0) / 2.0).abs() <= 1.5
                && ((1024.0 - right) - left).abs() <= 1.5
                && near(top, 40.0),
            "the dialog is not centred 40px down: {left},{top} to {right}"
        );
        assert!(
            reading.folded(),
            "a 655px dialog folds Label, Move and Delete"
        );

        support::keys(&window, &["j"]);
        crate::settle();
        assert_eq!(size(&dialog), (655, 620), "j resized the dialog");

        // A resize either way is followed: maximised onto the monitor,
        // then restored to the 1024x700 it asked for.
        window.maximize();
        assert!(
            crate::settle_until(async || window.width() > 1100 && size(&dialog) == fitted(&window))
                .await,
            "in a {}x{} window the dialog is {:?}",
            window.width(),
            window.height(),
            size(&dialog)
        );
        assert!(
            !reading.folded(),
            "a {}px dialog has room for every verb",
            size(&dialog).0
        );
        restore(&window).await;
        assert!(
            crate::settle_until(async || size(&dialog) == (655, 620)).await,
            "restored, the dialog is {:?}",
            size(&dialog)
        );
        assert!(reading.folded(), "narrowed again, the dialog folds again");
    });
}

/// One pixel of `texture`, as RGB bytes.
fn pixel(texture: &gdk::Texture, x: i32, y: i32) -> [u8; 3] {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let at = y as usize * stride + x as usize * 4;
    [bytes[at], bytes[at + 1], bytes[at + 2]]
}

/// Whether `seen` is `want`, give or take a step of rounding.
fn same(seen: [u8; 3], want: [u8; 3]) -> bool {
    seen.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2)
}

/// T205: the list behind the dialog is dimmed by black at 20% in light
/// and 45% in dark, measured on a pixel of the list beside the dialog.
pub fn the_list_behind_is_dimmed_by_black_at_20_and_45_percent() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Dimmed", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        for (scheme, kept) in [
            (adw::ColorScheme::ForceLight, 0.80),
            (adw::ColorScheme::ForceDark, 0.55),
        ] {
            manager.set_color_scheme(scheme);
            let (window, _client) = fixture.open().await;
            crate::settle_for(std::time::Duration::from_millis(200)).await;
            // Left of the dialog, near the foot, in the list's ground.
            let (x, y) = (40, window.height() - 30);
            let before = pixel(
                &postio_widgets::capture::texture(&window)
                    .expect("drawn")
                    .texture,
                x,
                y,
            );
            support::keys(&window, &["j"]);
            support::press(&window, "Return", gdk::ModifierType::empty());
            let reading = window.reading().expect("open");
            assert!(crate::settle_until(async || reading.dialog().is_mapped()).await);
            // The dimming fades in with the dialog: wait for where it ends.
            let want = before.map(|channel| (f64::from(channel) * kept).round() as u8);
            let seen = || {
                pixel(
                    &postio_widgets::capture::texture(&window)
                        .expect("drawn")
                        .texture,
                    x,
                    y,
                )
            };
            crate::settle_until(async || same(seen(), want)).await;
            let after = seen();
            assert!(
                same(after, want),
                "{scheme:?}: the list behind went from {before:?} to {after:?}, \
                 not {want:?} (black at {:.0}%)",
                (1.0 - kept) * 100.0
            );
            reading.close();
            window.close();
            crate::settle();
        }
        manager.set_color_scheme(adw::ColorScheme::Default);
    });
}

/// The blocks inside the message, top to bottom, by the class each wears.
const BLOCKS: [&str; 6] = [
    "focus-open-thread",
    "focus-open-subject",
    "focus-open-labels",
    "focus-open-header-card",
    "focus-marker-card",
    "postio-attachments",
];

/// T207: in a wide window and at 1024, the thread marker, subject, labels, sender
/// block, action card, body and attachments share both edges of one column
/// `min(480, dialog - 96)` wide, centred in the dialog.
pub fn every_block_shares_both_edges_of_one_centred_column() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        for size in [WIDE, NARROW] {
            let fixture = Fixture::empty().await;
            every_block(&fixture).await;
            let window = opened_at(&fixture, 1, size).await;
            let reading = window.reading().expect("open");
            let dialog = reading.dialog();
            assert!(
                crate::settle_until(
                    async || support::only(&dialog, "postio-attachments").is_mapped()
                )
                .await,
                "the attachments never showed"
            );
            let width = focus_dialog::dialog_width(window.width());
            let column =
                focus_dialog::column_width(width, postio_body::treatment::Treatment::AppColours);
            let content = dialog.child().expect("content");
            assert_eq!(content.width(), width, "the dialog's width");
            let left = (width - column) as f32 / 2.0;
            let right = left + column as f32;
            let mut blocks: Vec<(String, gtk::Widget)> = BLOCKS
                .iter()
                .map(|class| ((*class).to_owned(), shown(&dialog, class)))
                .collect();
            blocks.push(("body".into(), reading.reader().view().clone().upcast()));
            for (what, block) in blocks {
                let (l, _, r, _) = edges(&block, &content);
                assert!(
                    near(l, left) && near(r, right),
                    "{size:?}: the {what} runs {l} to {r}; the column is {left} to {right}"
                );
            }
            window.close();
            crate::settle();
        }
    });
}

/// T208: every gap the handoff gives, from the blocks' real allocations,
/// with every block present.
pub fn the_blocks_keep_the_handoffs_rhythm() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        assert!(
            crate::settle_until(async || support::only(&dialog, "postio-attachments").is_mapped())
                .await
        );
        let at = |class: &str| edges(&shown(&dialog, class), &dialog);
        let toolbar = at("focus-open-toolbar");
        let header = at("focus-open-header");
        let thread = at("focus-open-thread");
        let subject = at("focus-open-subject");
        let labels = at("focus-open-labels");
        let sender = at("focus-open-header-card");
        let card = at("focus-marker-card");
        let body = edges(reading.reader().view(), &dialog);
        let files = at("postio-attachments");
        let gap = |above: (f32, f32, f32, f32), below: (f32, f32, f32, f32)| below.1 - above.3;
        let height = |block: (f32, f32, f32, f32)| block.3 - block.1;
        let checks = [
            ("header bar height", height(header), 52),
            ("action row height", height(toolbar), 44),
            (
                "action row -> thread marker",
                gap(toolbar, thread),
                rhythm::TOP,
            ),
            (
                "thread marker -> subject",
                gap(thread, subject),
                rhythm::MARKER_TO_SUBJECT,
            ),
            (
                "subject -> labels",
                gap(subject, labels),
                rhythm::SUBJECT_TO_LABELS,
            ),
            ("labels height", height(labels), 24),
            (
                "labels -> sender block",
                gap(labels, sender),
                rhythm::LABELS_TO_SENDER,
            ),
            (
                "sender block -> action card",
                gap(sender, card),
                rhythm::SENDER_TO_CARD,
            ),
            ("action card -> body", gap(card, body), rhythm::CARD_TO_BODY),
            (
                "body -> attachments",
                gap(body, files),
                rhythm::BODY_TO_ATTACHMENTS,
            ),
        ];
        let wrong: Vec<String> = checks
            .iter()
            .filter(|(_, seen, want)| !near(*seen, *want as f32))
            .map(|(what, seen, want)| format!("{what}: {seen}, not {want}"))
            .collect();
        assert!(
            wrong.is_empty(),
            "off the handoff's rhythm:\n  {}",
            wrong.join("\n  ")
        );
        assert!(
            height(card) >= 48.0,
            "the action card is {}px tall",
            height(card)
        );

        // Inside the sender block: 12px to the first row and from the last,
        // rows 22px tall and 2px apart.
        let rows: Vec<(f32, f32, f32, f32)> = support::with_class(&dialog, "focus-open-field")
            .iter()
            .filter(|field| field.is_mapped())
            .map(|field| edges(field, &dialog))
            .collect();
        assert_eq!(rows.len(), 3, "From, To and Cc");
        assert!(
            near(rows[0].1 - sender.1, rhythm::SENDER_PADDING as f32),
            "{rows:?} in {sender:?}"
        );
        assert!(
            near(sender.3 - rows[2].3, rhythm::SENDER_PADDING as f32),
            "{rows:?} in {sender:?}"
        );
        // Rows on a shared baseline land on a fractional pixel: a row is
        // its 22px, give or take the one the baseline rounds into.
        for row in &rows {
            assert!(
                (height(*row) - rhythm::SENDER_ROW as f32).abs() <= 1.5,
                "a row is {}px",
                height(*row)
            );
        }
        assert!(near(
            rows[1].1 - rows[0].3,
            focus_dialog::SENDER_ROW_GAP as f32
        ));

        // The attachments: 16px under their hairline, chips 40px tall and
        // 8px apart.
        let chips: Vec<(f32, f32, f32, f32)> =
            support::with_class(&dialog, "postio-attachment-card")
                .iter()
                .map(|chip| edges(chip, &dialog))
                .collect();
        assert_eq!(chips.len(), 2, "two attachments");
        assert!(
            near(
                chips[0].1 - files.1,
                rhythm::ATTACHMENTS_RULE_TO_CHIPS as f32
            ),
            "the chips start {}px under the rule",
            chips[0].1 - files.1
        );
        assert!(
            near(height(chips[0]), 40.0),
            "a chip is {}px tall",
            height(chips[0])
        );
        assert!(near(chips[1].0 - chips[0].2, rhythm::CHIP_GAP as f32));

        // Under the last block, the column's 32px: what the scroller can
        // reach below the attachments.
        let scroller = scroller_of(reading.reader().view());
        let reach = scroller
            .child()
            .expect("the column's holder")
            .measure(gtk::Orientation::Vertical, scroller.width())
            .1 as f32;
        let last = edges(
            &shown(&dialog, "postio-attachments"),
            &scroller.child().expect("held"),
        )
        .3;
        assert!(
            near(reach - last, rhythm::BOTTOM as f32),
            "the column ends {}px under the attachments",
            reach - last
        );
    });
}

/// T208: an absent block takes its gap with it, and the next gap stays as
/// listed: no thread, the subject is 28px under the action row; no action
/// card, the body is 24px under the sender block.
pub fn an_absent_block_takes_its_gap_with_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.com"), "Alone", "x", 10)
            .await;
        dress(&fixture, message, false).await;
        fixture.write_body(message, BODY).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        assert!(!support::only(&dialog, "focus-open-thread").is_visible());
        assert!(support::with_class(&dialog, "focus-marker-card").is_empty());
        let at = |class: &str| edges(&shown(&dialog, class), &dialog);
        let toolbar = at("focus-open-toolbar");
        let subject = at("focus-open-subject");
        let sender = at("focus-open-header-card");
        let body = edges(reading.reader().view(), &dialog);
        assert!(
            near(subject.1 - toolbar.3, rhythm::TOP as f32),
            "with no thread the subject is {}px under the action row",
            subject.1 - toolbar.3
        );
        assert!(
            near(body.1 - sender.3, rhythm::CARD_TO_BODY as f32),
            "with no card the body is {}px under the sender block",
            body.1 - sender.3
        );
    });
}

/// T209: the dialog wears the handoff's palette, light and dark: its
/// surface, the sender block on the surface with no box of its own, and
/// the action card filled with the system's accent at 8% (light) and 12%
/// (dark). The accent is libadwaita's, never the handoff's teal (spec C26):
/// set another and the card follows it.
pub fn the_dialog_wears_its_palette_in_light_and_dark() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let manager = adw::StyleManager::default();
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        // The handoff's surface (SPEC.md section 6); the accent is the
        // system's, as libadwaita resolves it for the scheme.
        let palettes = [
            (adw::ColorScheme::ForceLight, [0xfd, 0xfd, 0xfb], 0.08),
            (adw::ColorScheme::ForceDark, [0x1c, 0x21, 0x20], 0.12),
        ];
        // The system's own accent, then another one set the way libadwaita
        // sets it -- its named colour, which every `--accent-*` derives
        // from -- so a card that kept any fixed colour cannot pass both.
        let other = adw::AccentColor::Red;
        let display = gdk::Display::default().expect("a display");
        let setting = gtk::CssProvider::new();
        for accent in [None, Some(other)] {
            if let Some(accent) = accent {
                let rgba = accent.to_rgba();
                setting.load_from_string(&format!(
                    "@define-color accent_bg_color rgb({}, {}, {});",
                    (rgba.red() * 255.0).round(),
                    (rgba.green() * 255.0).round(),
                    (rgba.blue() * 255.0).round(),
                ));
                gtk::style_context_add_provider_for_display(
                    &display,
                    &setting,
                    gtk::STYLE_PROVIDER_PRIORITY_USER,
                );
            }
            for (scheme, surface, share) in palettes {
                manager.set_color_scheme(scheme);
                crate::settle_for(std::time::Duration::from_millis(300)).await;
                let dark = manager.is_dark();
                let standalone = accent
                    .unwrap_or_else(|| manager.accent_color())
                    .to_standalone_rgba(dark);
                let accent_rgb = [standalone.red(), standalone.green(), standalone.blue()]
                    .map(|channel| (channel * 255.0).round() as u8);
                let picture = postio_widgets::capture::texture(&dialog).expect("drawn");
                let sender = edges(&shown(&dialog, "focus-open-header-card"), &dialog);
                let card = edges(&shown(&dialog, "focus-marker-card"), &dialog);
                // In the dialog's margin left of the column, and inside the
                // sender block between its rows' words and the date.
                let inside = edges(&dialog.child().expect("content"), &dialog).0 as i32;
                let ground = pixel(&picture.texture, inside + 20, sender.1 as i32 + 30);
                let in_sender = pixel(
                    &picture.texture,
                    sender.2 as i32 - 140,
                    sender.1 as i32 + 40,
                );
                let in_card = pixel(&picture.texture, card.0 as i32 + 4, card.1 as i32 + 4);
                let filled = std::array::from_fn(|i| {
                    (f64::from(surface[i]) * (1.0 - share) + f64::from(accent_rgb[i]) * share)
                        .round() as u8
                });
                // libadwaita's own standalone colour is clamped to sRGB; the
                // stylesheet mixes the unclamped oklab one, which for a
                // saturated accent in dark lands a few steps off it. The
                // handoff's teal is further than this from red, in both the
                // card and the links, so a fixed teal still fails.
                let close = |seen: [u8; 3], want: [u8; 3]| {
                    seen.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 10)
                };
                // The body's links: the accent the column hands the body.
                let link = support::only(&dialog, "postio-flow-accent").color();
                let link = [link.red(), link.green(), link.blue()]
                    .map(|channel| (channel * 255.0).round() as u8);
                assert!(
                    close(link, accent_rgb),
                    "{scheme:?}, accent {accent:?}: the body's links are {link:?}, not the \
                     system's accent {accent_rgb:?}"
                );
                assert!(
                    same(ground, surface),
                    "{scheme:?}: the surface is {ground:?}, not {surface:?}"
                );
                assert!(
                    same(in_sender, surface),
                    "{scheme:?}: the sender block is filled {in_sender:?}; it has no box"
                );
                assert!(
                    close(in_card, filled),
                    "{scheme:?}, accent {accent:?}: the action card is {in_card:?}, not the \
                     system's accent {accent_rgb:?} at {share}: {filled:?}"
                );
            }
        }
        gtk::style_context_remove_provider_for_display(&display, &setting);
        manager.set_color_scheme(adw::ColorScheme::Default);
    });
}

/// T206: below 760px the action row folds Label, Move and Delete into More
/// (`.`), whose menu holds the three with their keys; choosing one runs
/// its command, as its button would.
pub fn a_narrow_dialog_folds_label_move_and_delete_into_more() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Narrow", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        let window = opened_at(&fixture, 1, NARROW).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        for class in ["focus-open-label", "focus-open-move", "focus-open-delete"] {
            assert!(
                !support::only(&dialog, class).is_visible(),
                "{class} is still in a 655px action row"
            );
        }
        let more = shown(&dialog, "focus-open-more");
        assert!(
            support::texts(&more).iter().any(|text| text == "."),
            "More shows its key: {:?}",
            support::texts(&more)
        );

        support::press(&window, "period", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.more_open()).await,
            "`.` did not open More"
        );
        let menu = reading.more_menu();
        let items = support::with_class(&menu, "focus-open-more-item");
        let said: Vec<Vec<String>> = items.iter().map(support::texts).collect();
        assert_eq!(
            said,
            [["Label", "l"], ["Move", "m"], ["Delete", "Del"]]
                .map(|item| item.map(str::to_owned).to_vec()),
            "More's menu"
        );
        support::click(&window, &items[0], 1);
        assert!(
            crate::settle_until(async || !reading.more_open()).await,
            "choosing Label left More open"
        );
        assert!(
            crate::settle_until(async || window.open_picker().is_some()).await,
            "choosing Label did not run Label"
        );
    });
}

/// An office mail's HTML, from the corpus.
fn office_html() -> String {
    postio_model::mime::parse(
        postio_model::test_corpus::load("html-work-black-text").bytes(),
    )
    .body
    .html
    .expect("an HTML part")
}

/// T208: over an HTML body the render-mode line sits 24px under the action
/// card and 12px over the body -- the card's 24 goes to the line, and the
/// line keeps 12 of its own -- and with no card, 24 under the sender block.
pub fn the_render_mode_line_sits_24_under_the_card_and_12_over_the_body() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        for card in [true, false] {
            let fixture = Fixture::empty().await;
            let (message, _) = fixture
                .file(("Dana Whitfield", "dana@example.com"), "Building access", "x", 10)
                .await;
            fixture.write_html_body(message, &office_html()).await;
            if card {
                fixture.ask(message, "Building access").await;
            }
            let window = opened_at(&fixture, 1, WIDE).await;
            let reading = window.reading().expect("open");
            let dialog = reading.dialog();
            let line = reading
                .reader()
                .render_mode_line()
                .expect("Focus draws the line");
            assert!(
                crate::settle_until(async || line.is_shown()).await,
                "no line over the HTML body"
            );
            crate::settle();
            let above = if card {
                edges(&shown(&dialog, "focus-marker-card"), &dialog)
            } else {
                edges(&shown(&dialog, "focus-open-header-card"), &dialog)
            };
            let line = edges(&line.widget(), &dialog);
            let body = edges(reading.reader().view(), &dialog);
            assert!(
                near(line.1 - above.3, rhythm::CARD_TO_BODY as f32),
                "card {card}: the line is {}px under the block above it, not 24",
                line.1 - above.3
            );
            assert!(
                near(body.1 - line.3, rhythm::MODE_LINE_TO_BODY as f32),
                "card {card}: the body is {}px under the line, not 12",
                body.1 - line.3
            );
            window.close();
            crate::settle();
        }
    });
}

/// SPEC section 5: the action card's sentence wraps -- in italics, onto a
/// second line when it needs one -- and is never cut short with an
/// ellipsis.
pub fn the_action_cards_sentence_wraps_and_is_never_cut() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        const LONG: &str = "Could you please leave your comments on the pagination \
            section and the rate-limit headers by Wednesday afternoon";
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.com"), "Harbor", "x", 10)
            .await;
        fixture
            .write_body(message, &format!("Hi all,\n\n{LONG}, so I can freeze it.\n"))
            .await;
        fixture.ask(message, LONG).await;
        let window = opened_at(&fixture, 1, NARROW).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let quote = shown(&dialog, "focus-marker-quote")
            .downcast::<gtk::Label>()
            .expect("a label");
        assert!(
            quote.text().contains(LONG),
            "the card quotes {:?}",
            quote.text()
        );
        let layout = quote.layout();
        assert!(!layout.is_ellipsized(), "the sentence is cut short");
        assert!(
            layout.line_count() >= 2,
            "a sentence this long wraps in a 480px card: {} line(s)",
            layout.line_count()
        );
    });
}

/// SPEC section 2: Delete's cap reads `Del` in the action row, as the
/// handoff draws it; the binding keeps its name.
pub fn delete_s_cap_reads_del_in_the_action_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Wide", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let delete = shown(&reading.dialog(), "focus-open-delete");
        assert_eq!(support::texts(&delete), ["Delete", "Del"]);
        assert_eq!(
            postio_core::Keymap::defaults().binding(postio_core::CommandId::Delete),
            Some("Delete"),
            "the binding keeps its name"
        );
    });
}

/// T207: a page on paper takes the paper column, `min(640, dialog - 48)`,
/// and a layout wider than that is zoomed to fit it -- a 640px newsletter
/// in a 607px column at 0.95, with nothing to scroll sideways -- down to
/// 0.85; a layout that would need less is drawn at 0.85 and scrolls
/// sideways, which a sideways scroll on the body does.
pub fn a_page_on_paper_is_zoomed_to_its_column_and_scrolls_sideways_below_the_floor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let newsletter = postio_model::mime::parse(
            postio_model::test_corpus::load("html-newsletter-own-page").bytes(),
        )
        .body
        .html
        .expect("an HTML part");
        assert!(newsletter.contains("width=\"640\""), "the fixture's page is 640 wide");
        let floor = postio_render::render::PAPER_FIT_FLOOR;
        for (html, wide) in [
            (newsletter.clone(), 640.0),
            (newsletter.replace("width=\"640\"", "width=\"900\""), 900.0),
        ] {
            let fixture = Fixture::empty().await;
            let (message, _) = fixture
                .file(("Field Notes", "news@example.com"), "Issue 48", "x", 10)
                .await;
            fixture.write_html_body(message, &html).await;
            let window = opened_at(&fixture, 1, NARROW).await;
            let reading = window.reading().expect("open");
            let reader = reading.reader();
            assert!(
                crate::settle_until(async || reader.treatment()
                    == postio_body::treatment::Treatment::Paper
                    && reader.view().tiles_settled())
                .await,
                "the newsletter never went on paper"
            );
            let dialog = focus_dialog::dialog_width(window.width());
            let column =
                focus_dialog::column_width(dialog, postio_body::treatment::Treatment::Paper);
            assert!(
                crate::settle_until(async || reader.view().width() == column).await,
                "a {wide}px page: the body is {}px, not the {column}px paper column",
                reader.view().width()
            );
            let fit = reader.paper_fit();
            let want = (f64::from(column) / wide).clamp(floor, 1.0);
            assert!(
                (fit - want).abs() < 0.01,
                "a {wide}px page in a {column}px column is zoomed by {fit}, not {want}"
            );
            let view = reader.view().clone();
            let document = view.document().expect("drawn");
            let sideways = view.hadjustment().expect("the view scrolls sideways");
            if want > floor {
                assert!(
                    document.size.width <= f64::from(column) + 0.5,
                    "zoomed to fit, the page is still {}px",
                    document.size.width
                );
            } else {
                assert!(
                    sideways.upper() - sideways.page_size() > 1.0,
                    "at the floor the {}px page does not scroll sideways in {column}px",
                    document.size.width
                );
                let scroll = view
                    .observe_controllers()
                    .into_iter()
                    .filter_map(|controller| {
                        controller.ok()?.downcast::<gtk::EventControllerScroll>().ok()
                    })
                    .find(|scroll| {
                        scroll
                            .flags()
                            .contains(gtk::EventControllerScrollFlags::HORIZONTAL)
                    })
                    .expect("the body takes a sideways scroll");
                scroll.emit_by_name::<bool>("scroll", &[&3.0f64, &0.0f64]);
                assert!(
                    sideways.value() > 0.0,
                    "a sideways scroll did not move the page"
                );
            }
            window.close();
            crate::settle();
        }
    });
}

/// Spec C25: the dialog's chrome keeps the system's faces -- Adwaita Sans,
/// and Adwaita Mono for keys, dates and addresses -- at the handoff's sizes,
/// where the handoff draws Barlow and IBM Plex Mono. Only the body, drawn
/// by the renderer, is set in Barlow.
pub fn the_dialogs_chrome_is_set_in_the_system_faces() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let mut faces = std::collections::BTreeSet::new();
        let mut stack = vec![reading.dialog().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && label.is_mapped()
                && !label.text().is_empty()
            {
                let family = label
                    .pango_context()
                    .font_description()
                    .and_then(|font| font.family())
                    .map(|family| family.to_string())
                    .unwrap_or_default();
                faces.insert(family);
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
        }
        assert!(
            !faces.is_empty()
                && faces
                    .iter()
                    .all(|family| family.starts_with("Adwaita Sans")
                        || family.starts_with("Adwaita Mono")),
            "the chrome is set in {faces:?}"
        );
    });
}
