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
/// the action card filled with the accent at 8% (light) and 12% (dark).
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
        // surface, accent: the handoff's SPEC.md section 6.
        let palettes = [
            (
                adw::ColorScheme::ForceLight,
                [0xfd, 0xfd, 0xfb],
                [0x0d, 0x70, 0x68],
                0.08,
            ),
            (
                adw::ColorScheme::ForceDark,
                [0x1c, 0x21, 0x20],
                [0x5f, 0xc4, 0xb5],
                0.12,
            ),
        ];
        for (scheme, surface, accent, share) in palettes {
            manager.set_color_scheme(scheme);
            crate::settle_for(std::time::Duration::from_millis(300)).await;
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
                (f64::from(surface[i]) * (1.0 - share) + f64::from(accent[i]) * share).round() as u8
            });
            assert!(
                same(ground, surface),
                "{scheme:?}: the surface is {ground:?}, not {surface:?}"
            );
            assert!(
                same(in_sender, surface),
                "{scheme:?}: the sender block is filled {in_sender:?}; it has no box"
            );
            assert!(
                same(in_card, filled),
                "{scheme:?}: the action card is {in_card:?}, not the accent at {share}: {filled:?}"
            );
        }
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
            [["Label", "l"], ["Move", "m"], ["Delete", "Delete"]]
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
