//! Render a Postio window straight out of GSK to a PNG.
//!
//! The canvas is the visual spec for this lane, and "matches the canvas" is
//! not something to check by squinting at a running app. This asks GTK for the
//! exact render node it would put on screen and writes it to a file, so a
//! change in spacing, weight or colour is something you can look at, diff and
//! attach to a review.
//!
//! ```sh
//! cargo run -p postio-gtk --example shot -- /tmp/plate.png             # light
//! cargo run -p postio-gtk --example shot -- /tmp/plate.png dark
//! cargo run -p postio-gtk --example shot -- /tmp/plate.png dark hc
//! cargo run -p postio-gtk --example shot -- /tmp/narrow.png 900x700
//! cargo run -p postio-gtk --example shot -- /tmp/plate.png demo
//! cargo run -p postio-gtk --example shot -- /tmp/settings.png settings
//! cargo run -p postio-gtk --example shot -- /tmp/compose.png demo compose
//! cargo run -p postio-gtk --example shot -- /tmp/tight.png demo compact
//! cargo run -p postio-gtk --example shot -- /tmp/large.png demo text2
//! cargo run -p postio-gtk --example shot -- /tmp/box.png demo command
//! ```
//!
//! `demo` fills the panes with canvas 1b's own sample content, which is the
//! only way to check things like the selected row against the drawing before
//! there is a database to read. `settings` opens the canvas 3f panel over a
//! sample `config.toml` written to a scratch directory for the shot.
//!
//! `comfortable` and `compact` render the other two row densities — a design
//! that only works at one of them is unfinished. `text2` is GNOME's
//! text-scaling setting at 200%, which is how a partially sighted user
//! actually reads this application, and the only way to see that the type
//! scale moves with them rather than ignoring them.
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib, graphene};
use postio_core::ConnectionState;
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::{app, fonts, style, window::Window};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// Canvas 1b's own account, fed through the seams the application uses.
///
/// Not set on the widgets directly: `Window::install_feeds` is the whole of
/// the wiring a running Postio does, so the shot goes through it too. What
/// this renders is therefore what the application renders, minus a database.
/// Every address is a reserved domain, per CLAUDE.md.
fn populate(window: &Window) {
    let account = AccountId::new(1);
    let sample = Rc::new(Sample::new(account));

    let feeds = window.install_feeds(account, "lena@example.com", sample.clone(), sample);
    // A connection that is up and has just finished a sync, so the status
    // line reads `idle · imap` / `last sync 12s` as the canvas draws it.
    feeds.apply(&postio_core::Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });

    // Leaked on purpose: the shot renders one window and exits, and feeds
    // dropped here would stop answering before the first page arrived.
    Box::leak(Box::new(feeds));
}

/// Canvas 1b's folders and its six messages, answering the two source
/// traits the runtime will answer.
struct Sample {
    account: AccountId,
    rows: Vec<postio_gtk::list::Row>,
}

impl Sample {
    fn new(account: AccountId) -> Self {
        use postio_gtk::list::Row;
        use postio_model::EmailAddress;
        use postio_model::ids::{MessageId, ThreadId};

        let today = chrono::Local::now().date_naive();
        let at = |hour: u32, minute: u32, days: i64| {
            (today - chrono::Duration::days(days))
                .and_hms_opt(hour, minute, 0)
                .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
                .expect("a valid local time")
                .with_timezone(&chrono::Utc)
        };
        #[allow(clippy::too_many_arguments)]
        fn message(
            id: i64,
            name: &str,
            address: &str,
            subject: &str,
            preview: &str,
            when: chrono::DateTime<chrono::Utc>,
            seen: bool,
            thread_count: u32,
            has_attachments: bool,
        ) -> Row {
            Row {
                id: MessageId::new(id),
                thread: Some(ThreadId::new(id)),
                from: Some(EmailAddress::new(Some(name), address)),
                subject: Some(subject.to_owned()),
                preview: Some(preview.to_owned()),
                received_at: when,
                seen,
                flagged: false,
                answered: false,
                draft: false,
                has_attachments,
                thread_count,
            }
        }

        Sample {
            account,
            rows: vec![
                message(
                    1,
                    "Lena Tomlin",
                    "lena@example.com",
                    "Re: maildir index rebuild is O(n²)",
                    "Confirmed on 0.4.1 — the rebuild walks every…",
                    at(9, 14, 0),
                    false,
                    6,
                    true,
                ),
                message(
                    2,
                    "buildbot",
                    "buildbot@example.net",
                    "[FAIL] main · imap-idle · 3 tests",
                    "3 failing, 1 flaky. Full log attached…",
                    at(8, 52, 0),
                    false,
                    1,
                    true,
                ),
                message(
                    3,
                    "Nadia Okafor",
                    "nadia@example.org",
                    "Notes from the sync — attaching the deck",
                    "Short one. Decisions at the top, owners…",
                    at(8, 30, 0),
                    true,
                    1,
                    true,
                ),
                message(
                    4,
                    "lkml",
                    "lkml@example.org",
                    "[PATCH v3 2/7] sched: fix EEVDF lag accounting",
                    "Peter, Vincent — the lag decay was applied…",
                    at(7, 41, 0),
                    false,
                    14,
                    false,
                ),
                message(
                    5,
                    "Diogo Ferreira",
                    "diogo@example.org",
                    "Can you review the mbox importer today?",
                    "Small diff, mostly the folder walker…",
                    at(16, 20, 3),
                    false,
                    1,
                    false,
                ),
                message(
                    6,
                    "Sara Abadi",
                    "sara@example.com",
                    "Re: Re: keyring unlock on wayland",
                    "gnome-keyring works, kwallet needs the…",
                    at(11, 5, 4),
                    true,
                    3,
                    false,
                ),
            ],
        }
    }
}

impl MailboxSource for Sample {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let folder = |id: i64, path: &str, role, counts| {
            let mut mailbox = Mailbox::new(self.account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = counts;
            mailbox.last_synced_at = Some(chrono::Utc::now() - chrono::Duration::seconds(12));
            mailbox
        };
        let counts = |total, unread, flagged| MailboxCounts {
            total,
            unread,
            flagged,
        };
        let folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox, counts(940, 12, 3)),
            folder(2, "Flagged", MailboxRole::Flagged, counts(940, 12, 3)),
            folder(3, "Drafts", MailboxRole::Drafts, counts(2, 2, 0)),
            folder(4, "Sent", MailboxRole::Sent, counts(4021, 0, 0)),
            folder(5, "Archive", MailboxRole::Archive, counts(38122, 0, 0)),
            folder(6, "lkml", MailboxRole::Regular, counts(9004, 204, 0)),
            folder(7, "wayland-devel", MailboxRole::Regular, counts(880, 37, 0)),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Sample {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = self.rows.len() as u32;
        let start = (request.offset as usize).min(self.rows.len());
        let end = (start + request.limit as usize).min(self.rows.len());
        let rows = self.rows[start..end].to_vec();
        Box::pin(async move { Ok(Page { total, rows }) })
    }
}

/// Canvas 3f's own sample file, so the shot can be held up against the
/// drawing.
fn show_settings(window: &Window) {
    let path =
        std::env::temp_dir().join(format!("postio-shot-settings-{}.toml", std::process::id()));
    std::fs::write(
        &path,
        "# edits here and in the panel are the same file\n\
         [ui]\n\
         density = \"compact\"\n\
         theme = \"system\"\n\
         show_hover_actions = true\n\
         thread_drill = true\n\n\
         [keys]\n\
         archive = \"a\"\n\
         archive_thread = \"A\"\n\
         undo = \"u\"\n",
    )
    .expect("a scratch config.toml for the shot");
    window.settings().load(&path);
    window.open_settings();
}

/// Canvas 2a's own reply, so the composer can be held up against the drawing.
///
/// The canvas' addresses are not ours to ship: every one here is a reserved
/// domain, per CLAUDE.md.
fn show_composer(window: &Window) {
    let account = AccountId::new(1);
    let identity = |name: &str, address: &str, default| postio_model::Identity {
        display_name: name.to_owned(),
        is_default: default,
        signature: Some(postio_model::Signature {
            text: format!("{name} · postio.example.com"),
            html: None,
        }),
        ..postio_model::Identity::new(
            account,
            postio_model::EmailAddress::new(Some(name), address),
        )
    };

    let composer = postio_gtk::composer::install(window);
    composer.set_identities(vec![
        identity("Lena Tomlin", "lena@example.com", true),
        identity("Lena Tomlin", "lena@example.net", false),
    ]);

    let mut draft = postio_model::Draft::new(account);
    draft.kind = postio_model::DraftKind::Reply;
    draft.to = vec![postio_model::EmailAddress::new(
        Some("Diogo Ferreira"),
        "diogo@example.org",
    )];
    draft.subject = "Re: mbox importer review".to_owned();
    draft.body = postio_model::MessageBody {
        text: Some(
            "Looking now. The folder walker reads right, but I'd key the dedupe on \
             the maildir filename so it matches the index patch — otherwise the two \
             disagree on re-imported mail.\n\n\
             > Small diff, mostly the folder walker and a\n\
             > dedupe pass keyed on Message-ID.\n"
                .to_owned(),
        ),
        html: None,
    };
    composer.open(draft);
}

/// How many frames to let the window paint before the shot is taken.
///
/// One to allocate, one to settle any size that depended on it, and the
/// rest for work that only starts once there is a viewport to fill — the
/// message list asks for its first page from inside its first layout, so
/// the rows are a frame or two behind the panes around them.
const SETTLE_FRAMES: u32 = 8;

/// The ceiling on that wait, so a window that never paints reports rather
/// than hangs.
const SETTLE_MS: u64 = 5000;

/// Run the main loop until `window` has painted [`SETTLE_FRAMES`] frames.
///
/// Not a spin count. `MainContext::iteration(false)` returns immediately
/// when nothing is pending, so a fixed number of them is not a wait at all
/// and the frame clock may never tick inside it — which is how this example
/// came to render an empty message list while the running application drew
/// it correctly. Counting actual frames is the thing that was meant all
/// along. The heartbeat guarantees the blocking iteration returns.
fn settle(window: &Window) {
    let left = Rc::new(Cell::new(SETTLE_FRAMES));
    window.add_tick_callback(glib::clone!(
        #[strong]
        left,
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    ));

    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(SETTLE_MS);
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "postio.png".to_string());
    let flag = |name: &str| args.iter().skip(1).any(|a| a == name);
    // A `WxH` argument forces the window size, which is how the adaptive
    // modes get rendered without a compositor in the loop.
    let size = args.iter().skip(1).find_map(|a| {
        let (w, h) = a.split_once('x')?;
        Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
    });
    let scheme = if flag("dark") {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    };
    let high_contrast = flag("hc");

    if adw::init().is_err() {
        eprintln!("shot: no display; rendering needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    // Same order as `app::run`: fonts before the first widget.
    fonts::install().expect("the embedded fonts should install");
    let display = gdk::Display::default().expect("a display");
    style::install(&display);
    app::install_icons(&display);
    adw::StyleManager::default().set_color_scheme(scheme);

    // GNOME's "Large Text" works by moving `gtk-xft-dpi`, so this is what
    // a text-scaling user actually sees.
    if let Some(factor) = args.iter().skip(1).find_map(|a| a.strip_prefix("text")) {
        let factor: f64 = factor.parse().unwrap_or(2.0);
        if let Some(settings) = gtk::Settings::default() {
            let base = settings.gtk_xft_dpi();
            settings.set_gtk_xft_dpi((base as f64 * factor) as i32);
        }
    }

    let window = Window::default();
    if high_contrast {
        window.add_css_class(style::HIGH_CONTRAST_CLASS);
    }
    if let Some((width, height)) = size {
        window.set_default_size(width, height);
    }
    if flag("demo") {
        populate(&window);
    }
    // The list has three row heights and a design that only works at one of
    // them is unfinished, so the shot can render any of them.
    for (name, density) in [
        ("comfortable", postio_config::Density::Comfortable),
        ("compact", postio_config::Density::Compact),
    ] {
        if flag(name) {
            window.list().set_density(density);
        }
    }
    // The one box, in the mode a prefix puts it in. `postio-cfd.1` folded
    // the palette and the query bar into this; a surface nobody can render
    // is a surface nobody checks against the canvas.
    if flag("command") {
        window.open_finder(postio_gtk::finder::Mode::Command);
    }
    if flag("folder") {
        window.open_finder(postio_gtk::finder::Mode::Mailbox);
    }
    if flag("search") {
        window.open_finder(postio_gtk::finder::Mode::Search);
        window.finder().set_query(postio_gtk::finder::Query {
            mode: postio_gtk::finder::Mode::Search,
            text: "from:lena has:attach after:aug1".into(),
        });
    }
    if flag("settings") {
        show_settings(&window);
    }
    if flag("compose") {
        show_composer(&window);
    }
    window.present();

    settle(&window);

    // The canvas draws its key hints on the first row, which means the list
    // has the keyboard — and a shot without them is a shot of a different
    // state. Focused here rather than in `populate` because the rows
    // arrive a frame or two later and an empty list has no row to focus.
    // The canvas draws its key hints on the first row, which means the list
    // has the keyboard — and a shot without them is a shot of a different
    // state. Focused here rather than in `populate` because the rows arrive
    // a frame or two later and an empty list has no row to focus.
    if flag("demo") {
        window.list().grab_focus();
        settle(&window);
    }

    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);
    let Some(node) = snapshot.to_node() else {
        // Almost always the compositor rather than the widgets: it stops
        // delivering frame callbacks to a window nobody can see, and the
        // commonest reason on a developer's machine is that the screen
        // blanked part-way through. Worth saying, because "the window drew
        // nothing" reads like a bug in the thing being rendered.
        eprintln!(
            "shot: no frame after {SETTLE_MS}ms — is the screen blanked or the \
             window occluded? Nothing is painted to a surface the compositor \
             is not showing."
        );
        return glib::ExitCode::FAILURE;
    };
    let renderer = window
        .native()
        .and_then(|native| native.renderer())
        .expect("a realized window has a renderer");
    let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));

    match texture.save_to_png(&path) {
        Ok(()) => {
            println!("shot: {width}x{height} -> {path}");
            glib::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("shot: cannot write {path}: {error}");
            glib::ExitCode::FAILURE
        }
    }
}
