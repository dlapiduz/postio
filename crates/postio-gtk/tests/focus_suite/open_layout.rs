use adw::prelude::*;
use gtk::gdk;
use postio_model::{Attachment, EmailAddress, MessageId};
use postio_storage::repository::MessageRepository;
use postio_ui::focus_dialog::{self, rhythm};

use crate::open_reading::scroller_of;
use crate::support::{self, Fixture};

const BODY: &str = "Hi all,\n\nUploaded v3 of the Harbor API draft with the \
    pagination changes from Monday's review. The main differences from v2:\n\n\
    - Cursor pagination on every list endpoint, replacing page and offset.\n\
    - Rate-limit headers are documented for every response.\n\
    - The exports endpoint moved under accounts, as Ben suggested.\n\n\
    Please leave comments by Wednesday; I'd like to freeze it on Thursday so \
    Ben can start on the client.\n\nThanks,\nLena";

async fn every_block(fixture: &Fixture) -> MessageId {
    let ids = fixture.thread_of("Harbor API draft v3", 2, 10).await;
    fixture.read_all_but_last(&ids).await;
    let latest = *ids.last().expect("a thread");
    dress(fixture, latest, true).await;
    fixture.write_body(latest, BODY).await;
    fixture.label_in(latest, "Harbor", "#3a7d44").await;
    fixture
        .ask(latest, "Please leave comments by Wednesday")
        .await;
    latest
}

const LIST: &str = "harbor.example.com";

async fn on_a_list(fixture: &Fixture, message: MessageId) {
    let connection = fixture.database.connect().await.expect("a connection");
    let repository = MessageRepository::new(&connection);
    let mut row = repository
        .get(message)
        .await
        .expect("the message reads")
        .expect("the message is there");
    row.list_id = Some(LIST.to_owned());
    repository.update(&mut row).await.expect("listed");
}

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
    row.list_id = Some(LIST.to_owned());
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

/// The suite's compositor maximises onto a 1280x800 monitor, so the handoff's 1440 is out of reach;
/// the pure functions are proven at 1440 and 1920 in `postio_ui::focus_dialog`.
const WIDE: Option<(i32, i32)> = None;
const NARROW: Option<(i32, i32)> = Some((1024, 700));

async fn restore(window: &postio_gtk::window::FocusWindow) {
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

async fn opened_at(
    fixture: &Fixture,
    rows: usize,
    size: Option<(i32, i32)>,
) -> postio_gtk::window::FocusWindow {
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
    assert!(
        crate::settle_until(async || window.reading().is_some()).await,
        "Enter opened the message"
    );
    let reading = window.reading().expect("Enter opened the message");
    assert!(
        crate::settle_until(async || reading.reader().view().document().is_some()
            && !reading.body_text().is_empty()
            && reading.reader().view().tiles_settled())
        .await,
        "the body was never drawn"
    );
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

/// #2125: the header stays in view though the marker's sentence is below the fold; the reveal stops
/// rather than scroll the header off.
pub fn the_window_opens_with_its_header_in_view_and_the_sentence_marked() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let view = reading.reader().view().clone();
        let scroller = shown(&dialog, "focus-open-scroller")
            .downcast::<gtk::ScrolledWindow>()
            .expect("the message's scroller");
        assert!(
            crate::settle_until(async || {
                let adjustment = scroller.vadjustment();
                view.highlighted().is_some() && adjustment.upper() > adjustment.page_size()
            })
            .await,
            "the sentence was never marked, or the column never grew past its page"
        );
        let look = || {
            let adjustment = scroller.vadjustment();
            (adjustment.value(), adjustment.upper())
        };
        let last = std::cell::RefCell::new(None);
        assert!(
            crate::settle_until(async || {
                let now = look();
                let still = *last.borrow() == Some(now);
                last.replace(Some(now));
                still
            })
            .await,
            "the column never came to rest: {:?}",
            look()
        );
        // In the column's own coordinates: an allocation read off the scroller can lag a scroll by
        // a frame.
        let column = shown(&dialog, "focus-open-column")
            .parent()
            .expect("the column's clamp");
        let (value, _) = look();
        let page = scroller.vadjustment().page_size();
        let subject = edges(&shown(&dialog, "focus-open-subject"), &column).1 as f64;
        let sender = edges(&shown(&dialog, "focus-open-header-card"), &column).3 as f64;
        assert!(
            value <= subject && sender <= value + page,
            "the header is scrolled out of view: the column is scrolled to \
             {value} of {:?}, the subject starts at {subject} and the sender \
             block ends at {sender}, over a {page}px page",
            look()
        );
        assert!(
            view.highlighted().is_some(),
            "the marker's sentence is no longer highlighted"
        );
        let sentence = view
            .highlight_rects()
            .iter()
            .map(|rect| rect.y1)
            .fold(0.0, f64::max);
        assert!(
            edges(&view, &column).1 as f64 + sentence > page,
            "the sentence is on the first screen, so staying at the top proves nothing"
        );
    });
}

fn fitted(window: &postio_gtk::window::FocusWindow) -> (i32, i32) {
    (
        focus_dialog::dialog_width(window.width()),
        focus_dialog::dialog_height(window.height()),
    )
}

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

fn shown(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Widget {
    let widget = support::only(root, class);
    assert!(widget.is_mapped(), "{class} is not on screen");
    widget
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0
}

fn the_notice(root: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let shown: Vec<gtk::Widget> = support::with_class(root, "postio-notice")
        .into_iter()
        .filter(gtk::Widget::is_mapped)
        .collect();
    assert_eq!(shown.len(), 1, "one notice on screen");
    shown[0].clone()
}

async fn without_the_notice(reading: &postio_gtk::open::OpenMessage) {
    let dialog = reading.dialog();
    let top = || edges(reading.reader().view(), &dialog).1;
    let was = top();
    reading.reader().set_unsubscribe(None);
    assert!(
        crate::settle_until(async || support::with_class(&dialog, "postio-notice")
            .iter()
            .all(|notice| !notice.is_mapped())
            && top() < was - 1.0)
        .await,
        "the notice never went, or the body never moved up into its place: {}px, was {was}px",
        top()
    );
    crate::settle();
}

pub fn the_dialog_is_sized_by_the_window_and_never_by_the_message() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        for minutes in [10, 20] {
            let (message, _) = fixture.file_from_ada("Sized", "x", minutes).await;
            fixture.write_body(message, "A body.").await;
        }
        let window = opened_at(&fixture, 2, NARROW).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let size = |dialog: &adw::Dialog| {
            let content = dialog.child().expect("content");
            (content.width(), content.height())
        };
        assert_eq!(size(&dialog), (655, 620), "in a 1024x700 window");
        let (left, top, right, _) = edges(&dialog.child().expect("content"), &window);
        assert!(
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

fn pixel(texture: &gdk::Texture, x: i32, y: i32) -> [u8; 3] {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let at = y as usize * stride + x as usize * 4;
    [bytes[at], bytes[at + 1], bytes[at + 2]]
}

fn same(seen: [u8; 3], want: [u8; 3]) -> bool {
    seen.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2)
}

pub fn recipient_addresses_keep_the_reader_centered() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let message = fixture
            .file_addressed(
                ("Ada", "ada@example.com"),
                &[
                    ("", "engineering.accounts.team@example.com"),
                    ("", "operations.accounts.team@example.org"),
                    ("", "procurement.accounts.team@example.net"),
                ],
                &[],
                "Addressed",
                postio_ui::clock::now().with_timezone(&chrono::Utc),
            )
            .await;
        fixture.write_body(message, "A short message.").await;
        let window = opened_at(&fixture, 1, NARROW).await;
        let reading = window.reading().expect("open");
        let content = reading.dialog().child().expect("reader content");
        assert_eq!(
            (content.width(), content.height()),
            fitted(&window),
            "recipient addresses expanded the reader past its window-based frame"
        );
        assert!(
            content.width() < window.width(),
            "the reader hides the whole inbox"
        );
    });
}

pub fn the_list_behind_is_dimmed_by_black_at_20_and_45_percent() {
    crate::gtk_case(async {
        let manager = adw::StyleManager::default();
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Dimmed", "x", 10).await;
        fixture.write_body(message, "A body.").await;
        for (scheme, kept) in [
            (adw::ColorScheme::ForceLight, 0.80),
            (adw::ColorScheme::ForceDark, 0.55),
        ] {
            manager.set_color_scheme(scheme);
            let (window, _client) = fixture.open().await;
            crate::settle_for(std::time::Duration::from_millis(200)).await;
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

const BLOCKS: [&str; 6] = [
    "focus-open-thread",
    "focus-open-subject",
    "focus-open-labels",
    "focus-open-header-card",
    "focus-marker-card",
    "postio-attachments",
];

pub fn every_block_shares_both_edges_of_one_centred_column() {
    crate::gtk_case(async {
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
            let notice = the_notice(&dialog);
            let icon = edges(&support::only(&notice, "postio-notice-icon"), &content);
            let action = edges(
                &support::only(&notice, "postio-unsubscribe-banner-action"),
                &content,
            );
            assert!(
                near(icon.0, left) && near(action.2, right),
                "{size:?}: the notice's icon starts at {} and its button ends at {}; \
                 the column is {left} to {right}",
                icon.0,
                action.2
            );
            window.close();
            crate::settle();
        }
    });
}

pub fn the_blocks_keep_the_handoffs_rhythm() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        assert!(
            crate::settle_until(async || support::only(&dialog, "postio-attachments").is_mapped())
                .await
        );
        assert!(
            crate::settle_until(
                async || !reading.thread_chip_said().is_empty() && reading.thread_bar_state().1 > 0
            )
            .await,
            "the thread's bar never arrived"
        );
        // The bar's arrival lays the column out again, over more than one pass on a slow machine:
        // measure once two looks agree.
        let blocks = || {
            [
                "focus-thread-bar",
                "focus-open-thread",
                "focus-open-subject",
                "focus-open-header-card",
                "postio-attachments",
            ]
            .map(|class| edges(&shown(&dialog, class), &dialog))
        };
        let last = std::cell::RefCell::new(None);
        assert!(
            crate::settle_until(async || {
                let now = blocks();
                let still = last.borrow().as_ref() == Some(&now);
                last.replace(Some(now));
                still
            })
            .await,
            "the column never came to rest: {:?}",
            blocks()
        );
        let at = |class: &str| edges(&shown(&dialog, class), &dialog);
        let toolbar = at("focus-open-toolbar");
        let header = at("focus-open-header");
        let bar = at("focus-thread-bar");
        let thread = at("focus-open-thread");
        let subject = at("focus-open-subject");
        let labels = at("focus-open-labels");
        let sender = at("focus-open-header-card");
        let card = at("focus-marker-card");
        let notice = edges(&the_notice(&dialog), &dialog);
        let body = edges(reading.reader().view(), &dialog);
        let files = at("postio-attachments");
        let gap = |above: (f32, f32, f32, f32), below: (f32, f32, f32, f32)| below.1 - above.3;
        let height = |block: (f32, f32, f32, f32)| block.3 - block.1;
        let checks = [
            ("header bar height", height(header), 52),
            ("action row height", height(toolbar), 44),
            ("thread bar height", height(bar), 60),
            ("action row -> thread bar", gap(toolbar, bar), 0),
            ("thread bar -> thread marker", gap(bar, thread), rhythm::TOP),
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
            (
                "action card -> notice",
                gap(card, notice),
                rhythm::CARD_TO_NOTICE,
            ),
            ("notice -> body", gap(notice, body), rhythm::CARD_TO_BODY),
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
        // Rows on a shared baseline land on a fractional pixel, so a row is 22px give or take one.
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

        without_the_notice(&reading).await;
        let card = at("focus-marker-card");
        let body = edges(reading.reader().view(), &dialog);
        assert!(
            near(gap(card, body), rhythm::CARD_TO_BODY as f32),
            "with no notice the body is {}px under the action card",
            gap(card, body)
        );
    });
}

pub fn an_absent_block_takes_its_gap_with_it() {
    crate::gtk_case(async {
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
        let notice = edges(&the_notice(&dialog), &dialog);
        let body = edges(reading.reader().view(), &dialog);
        assert!(
            near(subject.1 - toolbar.3, rhythm::TOP as f32),
            "with no thread the subject is {}px under the action row",
            subject.1 - toolbar.3
        );
        assert!(
            near(notice.1 - sender.3, rhythm::CARD_TO_NOTICE as f32),
            "with no card the notice is {}px under the sender block",
            notice.1 - sender.3
        );
        assert!(
            near(body.1 - notice.3, rhythm::CARD_TO_BODY as f32),
            "the body is {}px under the notice",
            body.1 - notice.3
        );
        without_the_notice(&reading).await;
        let sender = at("focus-open-header-card");
        let body = edges(reading.reader().view(), &dialog);
        assert!(
            near(body.1 - sender.3, rhythm::CARD_TO_BODY as f32),
            "with no card and no notice the body is {}px under the sender block",
            body.1 - sender.3
        );
    });
}

/// The accent is libadwaita's, never the handoff's teal: set another and the card follows it.
pub fn the_dialog_wears_its_palette_in_light_and_dark() {
    crate::gtk_case(async {
        let manager = adw::StyleManager::default();
        let fixture = Fixture::empty().await;
        every_block(&fixture).await;
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let palettes = [
            (adw::ColorScheme::ForceLight, [0xfd, 0xfd, 0xfb], 0.08),
            (adw::ColorScheme::ForceDark, [0x1c, 0x21, 0x20], 0.12),
        ];
        // Set another accent the way libadwaita does, so a card that kept any fixed colour cannot
        // pass both.
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
                // libadwaita's standalone colour is clamped to sRGB while the stylesheet mixes
                // unclamped oklab, so a saturated dark accent lands a few steps off;
                // the handoff's teal is further than this from red.
                let close = |seen: [u8; 3], want: [u8; 3]| {
                    seen.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 10)
                };
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

pub fn a_narrow_dialog_folds_label_move_and_delete_into_more() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Narrow", "x", 10).await;
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
            [["Label", "l"], ["Move", "v"], ["Delete", "#"]]
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

fn office_html() -> String {
    postio_model::mime::parse(postio_model::test_corpus::load("html-work-black-text").bytes())
        .body
        .html
        .expect("an HTML part")
}

pub fn the_render_mode_line_sits_24_under_the_card_and_12_over_the_body() {
    crate::gtk_case(async {
        for card in [true, false] {
            let fixture = Fixture::empty().await;
            let (message, _) = fixture
                .file(
                    ("Dana Whitfield", "dana@example.com"),
                    "Building access",
                    "x",
                    10,
                )
                .await;
            fixture.write_html_body(message, &office_html()).await;
            on_a_list(&fixture, message).await;
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
            let above = || {
                if card {
                    edges(&shown(&dialog, "focus-marker-card"), &dialog)
                } else {
                    edges(&shown(&dialog, "focus-open-header-card"), &dialog)
                }
            };
            let notice = edges(&the_notice(&dialog), &dialog);
            assert!(
                near(notice.1 - above().3, rhythm::CARD_TO_NOTICE as f32),
                "card {card}: the notice is {}px under the block above it, not 12",
                notice.1 - above().3
            );
            for noticed in [true, false] {
                if !noticed {
                    without_the_notice(&reading).await;
                }
                let above = if noticed { notice } else { above() };
                let line = edges(&line.widget(), &dialog);
                let body = edges(reading.reader().view(), &dialog);
                assert!(
                    near(line.1 - above.3, rhythm::CARD_TO_BODY as f32),
                    "card {card}, notice {noticed}: the line is {}px under the block above it, not 24",
                    line.1 - above.3
                );
                assert!(
                    near(body.1 - line.3, rhythm::MODE_LINE_TO_BODY as f32),
                    "card {card}, notice {noticed}: the body is {}px under the line, not 12",
                    body.1 - line.3
                );
            }
            window.close();
            crate::settle();
        }
    });
}

pub fn the_action_cards_sentence_wraps_and_is_never_cut() {
    crate::gtk_case(async {
        const LONG: &str = "Could you please leave your comments on the pagination \
            section and the rate-limit headers by Wednesday afternoon";
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Lena Park", "lena@example.com"), "Harbor", "x", 10)
            .await;
        fixture
            .write_body(
                message,
                &format!("Hi all,\n\n{LONG}, so I can freeze it.\n"),
            )
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

pub fn delete_s_cap_reads_its_gmail_key_in_the_action_row() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Wide", "x", 10).await;
        fixture.write_body(message, "A body.").await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let delete = shown(&reading.dialog(), "focus-open-delete");
        assert_eq!(support::texts(&delete), ["Delete", "#"]);
        assert!(
            postio_core::Keymap::defaults()
                .bindings(postio_core::CommandId::Delete)
                .iter()
                .any(|binding| binding == "Delete"),
            "the Delete key still deletes"
        );
    });
}

pub fn a_page_on_paper_is_zoomed_to_its_column_and_scrolls_sideways_below_the_floor() {
    crate::gtk_case(async {
        let newsletter = postio_model::mime::parse(
            postio_model::test_corpus::load("html-newsletter-own-page").bytes(),
        )
        .body
        .html
        .expect("an HTML part");
        assert!(
            newsletter.contains("width=\"640\""),
            "the fixture's page is 640 wide"
        );
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
                        controller
                            .ok()?
                            .downcast::<gtk::EventControllerScroll>()
                            .ok()
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

pub fn the_dialogs_chrome_is_set_in_the_system_faces() {
    crate::gtk_case(async {
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
                && faces.iter().all(|family| family.starts_with("Adwaita Sans")
                    || family.starts_with("Adwaita Mono")),
            "the chrome is set in {faces:?}"
        );
    });
}

pub fn paper_fit_multiplies_under_the_zoom() {
    crate::gtk_case(async {
        let newsletter = postio_model::mime::parse(
            postio_model::test_corpus::load("html-newsletter-own-page").bytes(),
        )
        .body
        .html
        .expect("an HTML part")
        .replace("width=\"640\"", "width=\"900\"");
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Field Notes", "news@example.com"), "Issue 48", "x", 10)
            .await;
        fixture.write_html_body(message, &newsletter).await;
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
        let fit = reader.paper_fit();
        assert!(fit < 1.0, "the 900px page is not fitted: {fit}");
        let before = reader.view().document().expect("drawn").size.width;
        support::press(&window, "plus", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || reader.zoom() == 110
                && reader.view().tiles_settled()
                && reader
                    .view()
                    .document()
                    .is_some_and(|document| { (document.size.width / before - 1.1).abs() < 0.03 }))
            .await,
            "zoomed to {}, the page is {} wide, not 1.1 x {before}",
            reader.zoom(),
            reader
                .view()
                .document()
                .map_or(0.0, |document| document.size.width)
        );
        assert!(
            (reader.paper_fit() - fit).abs() < 0.01,
            "zooming moved the fit to the column: {fit} -> {}",
            reader.paper_fit()
        );
    });
}

pub fn the_senders_address_is_never_cut_while_there_is_room() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Norwood", "ada.norwood.accounts@example.com"),
                "Harbor",
                "x",
                60 * 24 * 20,
            )
            .await;
        fixture.write_body(message, "A body.").await;
        let window = opened_at(&fixture, 1, NARROW).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let address = shown(&dialog, "focus-open-address")
            .downcast::<gtk::Label>()
            .expect("a label");
        assert_eq!(address.text(), "ada.norwood.accounts@example.com");
        assert!(
            !address.layout().is_ellipsized(),
            "the address is cut short: {:?}",
            address.layout().text()
        );
        let date = shown(&dialog, "focus-open-date")
            .downcast::<gtk::Label>()
            .expect("a label");
        assert!(
            !date.text().contains(" at "),
            "the date is worded unlike today's: {:?}",
            date.text()
        );
    });
}

pub fn every_band_starts_at_the_messages_left_edge() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor API draft v3", 2, 10).await;
        fixture.read_all_but_last(&ids).await;
        let latest = *ids.last().expect("a thread");
        dress(&fixture, latest, false).await;
        fixture.write_body(latest, BODY).await;
        let window = opened_at(&fixture, 1, WIDE).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        let content = dialog.child().expect("content");
        let left = focus_dialog::GUTTER as f32;
        let words = |class: &str| -> f32 {
            let label = first_label(&shown(&dialog, class));
            edges(&label, &content).0 + label.layout_offsets().0 as f32
        };
        for (what, x) in [
            ("the thread bar's title", words("focus-thread-bar-title")),
            ("the action row's first verb", words("focus-open-toolbar")),
            ("the subject", words("focus-open-subject")),
            ("+ Label", words("focus-open-add-label")),
        ] {
            assert!(
                near(x, left),
                "{what} starts at {x}; the dialog's bands start at {left}"
            );
        }
        let card = shown(&dialog, "focus-open-header-card")
            .downcast::<gtk::Grid>()
            .expect("the sender block is a grid");
        for (row, field) in [(0, "From"), (1, "To")] {
            let name = card
                .child_at(0, row)
                .and_downcast::<gtk::Label>()
                .expect("a field's name");
            assert_eq!(name.text(), field);
            let value = first_label(&card.child_at(1, row).expect("a field's value"));
            let (at, should) = (baseline(&name, &content), baseline(&value, &content));
            // Two sizes centred on one 22px line share a baseline only within their fonts'
            // rounding, which differs between machines: 1.5px, not 1.
            assert!(
                (at - should).abs() <= 1.5,
                "{field} sits on {at}; its value on {should}"
            );
        }
        window.close();
        crate::settle();
    });
}

fn first_label(root: &gtk::Widget) -> gtk::Label {
    support::descendants(root)
        .into_iter()
        .chain(std::iter::once(root.clone()))
        .find_map(|widget| widget.downcast::<gtk::Label>().ok())
        .expect("a label")
}

fn baseline(label: &gtk::Label, root: &impl IsA<gtk::Widget>) -> f32 {
    let top = edges(label, root).1 + label.layout_offsets().1 as f32;
    top + label.layout().baseline() as f32 / gtk::pango::SCALE as f32
}

pub fn a_folded_thread_bar_keeps_its_height_at_a_narrow_window() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor API draft v3", 6, 10).await;
        fixture.read_all_but_last(&ids).await;
        let window = opened_at(&fixture, 1, Some((900, 700))).await;
        let reading = window.reading().expect("open");
        let dialog = reading.dialog();
        assert!(
            crate::settle_until(
                async || !reading.thread_chip_said().is_empty() && reading.thread_bar_state().1 > 0
            )
            .await,
            "the thread's bar never arrived"
        );
        assert!(
            crate::settle_until(
                async || support::with_class(&dialog, "focus-thread-chip-more")
                    .iter()
                    .any(gtk::Widget::is_mapped)
            )
            .await,
            "six messages fold into a more chip at 900 wide: {} chips",
            reading.thread_bar_state().1
        );
        let bar = || edges(&shown(&dialog, "focus-thread-bar"), &dialog);
        let last = std::cell::RefCell::new(None);
        assert!(
            crate::settle_until(async || {
                let now = bar();
                let still = last.borrow().as_ref() == Some(&now);
                last.replace(Some(now));
                still
            })
            .await,
            "the bar never came to rest: {:?}",
            bar()
        );
        let (_, top, _, bottom) = bar();
        assert!(
            near(bottom - top, 60.0),
            "the thread bar is {} tall at 900x700, not 60",
            bottom - top
        );
        window.close();
        crate::settle();
    });
}
