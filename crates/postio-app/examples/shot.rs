//! Render a Postio window straight out of GSK to a PNG.
//!
//! The canvas is the visual spec for this lane, and "matches the canvas" is
//! not something to check by squinting at a running app. This asks GTK for the
//! exact render node it would put on screen and writes it to a file, so a
//! change in spacing, weight or colour is something you can look at, diff and
//! attach to a review.
//!
//! ```sh
//! cargo run -p postio-app --example shot -- /tmp/plate.png             # light
//! cargo run -p postio-app --example shot -- /tmp/plate.png dark
//! cargo run -p postio-app --example shot -- /tmp/plate.png dark hc
//! cargo run -p postio-app --example shot -- /tmp/narrow.png 900x700
//! cargo run -p postio-app --example shot -- /tmp/plate.png demo
//! cargo run -p postio-app --example shot -- /tmp/settings.png settings
//! cargo run -p postio-app --example shot -- /tmp/compose.png demo compose
//! cargo run -p postio-app --example shot -- /tmp/popout.png demo compose detached
//! cargo run -p postio-app --example shot -- /tmp/tight.png demo compact
//! cargo run -p postio-app --example shot -- /tmp/large.png demo text2
//! cargo run -p postio-app --example shot -- /tmp/box.png demo command
//! cargo run -p postio-app --example shot -- /tmp/who.png demo contact
//! cargo run -p postio-app --example shot -- /tmp/selected.png demo selected
//! ```
//!
//! `demo` fills the panes from `postio_storage::seed` — a migrated in-memory
//! database with a real folder tree, corpus-derived messages, flags and
//! threading — read back through the same `SqliteStore` the running
//! application reads through. A shot of hand-written rows can only prove that
//! the *drawing* is right; this one is also about the content the store
//! actually produces. `settings` opens the canvas 3f panel over a sample
//! `config.toml` written to a scratch directory for the shot.
//!
//! # Why this lives in `postio-app`
//!
//! Because of that seed. `postio-gtk` may not depend on `rusqlite`, and
//! `scripts/checks/check-crate-boundaries.py` counts a crate's own dev-dependencies —
//! an example is built from that graph — so a `shot` that reads a store cannot
//! live beside the widgets it renders. `postio-app` is the crate that already
//! knows both halves exist, which is what a shot of the real application over
//! a real store is.
//!
//! `comfortable` and `compact` render the other two row densities — a design
//! that only works at one of them is unfinished. `text2` is GNOME's
//! text-scaling setting at 200%, which is how a partially sighted user
//! actually reads this application, and the only way to see that the type
//! scale moves with them rather than ignoring them.
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network — the
//! database it reads is created, seeded and thrown away in process.

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
use postio_model::mailbox::{Mailbox, MailboxRole};

/// A seeded account, fed through the seams the application uses.
///
/// Not set on the widgets directly: `Window::install_feeds` is the whole of
/// the wiring a running Postio does, so the shot goes through it too. What
/// this renders is therefore what the application renders.
///
/// The content is `postio_storage::seed`'s — a migrated in-memory database
/// with a real folder tree, corpus-derived messages, flags and threading, read
/// back through the same `SqliteStore` the running application reads through.
/// That is the point of it: a shot of hand-written rows can only ever prove
/// that the *drawing* is right, and this lane keeps finding that the drawing
/// was right about content the store does not actually produce.
fn populate(window: &Window) {
    let sample = Rc::new(Sample::from_seed());
    let (account, address) = (sample.account, sample.address.clone());

    let feeds = window.install_feeds(account, &address, sample.clone(), sample);
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

/// A seeded account's folders and the newest page of its inbox, answering the
/// two source traits the runtime answers.
///
/// Read once, up front, rather than kept as a live store: the shot renders one
/// window and exits, and a source that answered from SQLite on every page
/// request would make the example own a connection for the length of a render
/// it does not otherwise need one for.
struct Sample {
    account: AccountId,
    /// What the sidebar puts above the folder list.
    address: String,
    mailboxes: Vec<Mailbox>,
    rows: Vec<postio_gtk::list::Row>,
}

/// How many rows the demo reads. More than fills the pane at any density, and
/// far less than the mailbox holds.
const DEMO_ROWS: u32 = 60;

impl Sample {
    /// Seed a throwaway database and read it back through `SqliteStore`.
    fn from_seed() -> Self {
        use postio_runtime::store::MailStore;

        let database = postio_storage::test_support::memory();
        let report = postio_storage::seed::seed_small(&database, 11);
        let store = postio_runtime::store::SqliteStore::new(&database);

        // The store's reads are async because the application's are — they
        // cross onto a worker so the GTK thread never waits on a scan. Here
        // there is no GTK thread yet and nothing else to do, so they are
        // simply run to completion.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to read the seeded store with");

        let account = report.account.id;
        let inbox = report
            .mailbox(MailboxRole::Inbox)
            .expect("the seed has an inbox")
            .id;

        let mailboxes = runtime
            .block_on(store.mailboxes(account))
            .expect("the seeded folders");
        let page = runtime
            .block_on(store.message_page(postio_runtime::store::PageRequest {
                scope: postio_runtime::store::ListScope::Mailbox(inbox),
                offset: 0,
                limit: DEMO_ROWS,
            }))
            .expect("the seeded inbox");

        Sample {
            account,
            address: report.account.address.address.clone(),
            mailboxes,
            rows: page.rows.into_iter().map(row).collect(),
        }
    }
}

/// One row, as the list draws it.
///
/// Field for field, the same conversion `postio-app`'s own `feed` module
/// makes. It is repeated here rather than shared because `postio-app` is a
/// binary crate with no library target, so an example cannot link against its
/// modules — see the note in `compose`'s tests.
fn row(summary: postio_runtime::store::MessageSummary) -> postio_gtk::list::Row {
    postio_gtk::list::Row {
        id: summary.id,
        thread: summary.thread,
        from: summary.from,
        subject: summary.subject,
        preview: summary.preview,
        received_at: summary.received_at,
        seen: summary.seen,
        flagged: summary.flagged,
        answered: summary.answered,
        draft: summary.draft,
        has_attachments: summary.has_attachments,
        thread_count: summary.thread_count,
    }
}

impl MailboxSource for Sample {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        // Stamped as just-synced so the status line reads `last sync 12s` the
        // way the canvas draws it. The seed writes no sync state — it has
        // never talked to a server — and a sidebar saying "never synced" would
        // be a shot of the empty state rather than of the folder list.
        let synced = chrono::Utc::now() - chrono::Duration::seconds(12);
        let folders = self
            .mailboxes
            .iter()
            .cloned()
            .map(|mut mailbox| {
                mailbox.last_synced_at = Some(synced);
                mailbox
            })
            .collect();
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

/// Correspondents for the `@` mode, in the canvas' own cast.
///
/// Every address is a reserved domain, per CLAUDE.md.
fn sample_contacts() -> Vec<postio_model::Contact> {
    let person = |name: &str, address: &str, seen: u32| {
        let mut contact =
            postio_model::Contact::new(postio_model::EmailAddress::new(Some(name), address));
        contact.times_seen = seen;
        contact
    };
    vec![
        person("Lena Tomlin", "lena@example.com", 412),
        person("Nadia Okafor", "nadia@example.org", 96),
        person("Diogo Ferreira", "diogo@example.org", 54),
        person("Sara Abadi", "sara@example.com", 31),
        person("buildbot", "buildbot@example.net", 1204),
    ]
}

/// Canvas 2b's left column, over the artboard's own numbers.
///
/// Mounted through `search::View::attach`, which is the one call a running
/// Postio makes — so what this renders is what the application renders once
/// something answers with facets.
fn show_search_panels(window: &Window) {
    use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};

    let view = postio_gtk::search::View::attach(&window.shell(), &window.finder());
    let count = |scope, hits| ScopeCount { scope, hits };
    let refinement = |token: &str, hits| Refinement {
        token: token.to_owned(),
        hits,
    };
    view.set_facets(
        &Facets {
            scopes: vec![
                count(Scope::AllMail, 14),
                count(Scope::Inbox, 6),
                count(Scope::Lists, 8),
            ],
            refinements: vec![
                refinement("is:unread", 9),
                refinement("larger:1M", 5),
                refinement("is:flagged", 2),
                refinement("in:lkml", 8),
            ],
        },
        14,
    );
    view.set_searching(true);

    // Canvas 2b's own focused result, snippet and all. The markers are what
    // FTS5 puts around a match, so this is the shape a real hit arrives in.
    let marked = |text: &str| {
        text.replace('[', &postio_search::highlight::MATCH_START.to_string())
            .replace(']', &postio_search::highlight::MATCH_END.to_string())
    };
    view.set_focused(Some(&postio_search::SearchHit {
        message_id: postio_model::ids::MessageId::new(1),
        thread_id: Some(postio_model::ids::ThreadId::new(1)),
        mailbox_id: MailboxId::new(1),
        subject: Some("Re: maildir index rebuild is O(n²)".to_owned()),
        from: Some(postio_model::EmailAddress::new(
            Some("Lena Tomlin"),
            "lena@example.com",
        )),
        received_at: chrono::Utc::now(),
        snippet: marked(
            "…the rebuild walks every message once per folder rather than once per \
             store, so a 40k-message [maildir] takes about nine minutes on a cold \
             cache. The patch keys the header cache on the [maildir] filename…",
        ),
        score: -3.5,
    }));

    // And the body itself, so the match tint the reader stylesheet paints is
    // something that can be looked at rather than only asserted on.
    view.preview().set_body(
        postio_model::ids::MessageId::new(1),
        &postio_model::MessageBody {
            text: Some(
                "Confirmed on 0.4.1 — the rebuild walks every message once per folder \
                 rather than once per store, so a 40k-message maildir takes about nine \
                 minutes on a cold cache.\n\n\
                 The patch moves the header cache to a single pass and keys it on the \
                 maildir filename instead of Message-ID.\n"
                    .to_owned(),
            ),
            html: None,
        },
        Some("lena@example.com"),
    );

    // Leaked for the same reason `populate` leaks its feeds: the shot renders
    // one window and exits, and a view dropped here would unwire itself.
    Box::leak(Box::new(view));
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
            id: Default::default(),
            name: String::new(),
            text: format!("{name} · postio.example.com"),
            html: None,
        }),
        ..postio_model::Identity::new(
            account,
            postio_model::EmailAddress::new(Some(name), address),
        )
    };

    // `Window::composer`, not `composer::install`: the window caches the one
    // it mounted, and installing a second means the shot renders one composer
    // while `detached` below pops out another.
    let composer = window.composer();
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
fn settle(window: &impl IsA<gtk::Widget>) {
    let left = Rc::new(Cell::new(SETTLE_FRAMES));
    window.as_ref().add_tick_callback(glib::clone!(
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
    if flag("contact") {
        window.finder().set_contacts(&sample_contacts());
        window.open_finder(postio_gtk::finder::Mode::Contact);
    }
    if flag("search") {
        window.open_finder(postio_gtk::finder::Mode::Search);
        window.finder().set_query(postio_gtk::finder::Query {
            mode: postio_gtk::finder::Mode::Search,
            text: "maildir from:lena has:attach after:aug1".into(),
        });
        // Canvas 2b's own readout. Delivered through the same pacing the
        // application uses — `flush` asks the question the debounce was
        // about to ask, and the answer comes back under its sequence number,
        // so what is rendered is what a real answer would look like.
        if let Some(live) = window.finder().live() {
            live.flush();
            live.deliver(
                live.outstanding(),
                postio_gtk::search::Outcome {
                    hits: 14,
                    capped: false,
                    elapsed: Duration::from_millis(11),
                },
            );
        }
        show_search_panels(&window);
    }
    if flag("settings") {
        show_settings(&window);
    }
    if flag("compose") {
        show_composer(&window);
    }
    window.present();

    settle(&window);

    // The pop-out, rendered as its own window rather than as a state of this
    // one — because that is what it is. A surface nobody can render is a
    // surface nobody checks against the canvas, and this one has chrome of
    // its own (`AdwWindow` draws none unless the content provides it) that a
    // widget test cannot look at.
    let target: gtk::Window = match flag("detached").then(|| window.composer()) {
        Some(composer) => {
            composer.toggle_detached();
            let host = composer
                .detached_window()
                .expect("`detached` needs `compose`: there is nothing to pop out");
            settle(&host);
            host.upcast()
        }
        None => window.clone().upcast(),
    };

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

    // A selection is a *second* state on top of the focused row, and the two
    // have to be told apart at a glance (`postio-qhz.1`). This is the only way
    // to look at them together before there is a mailbox to select in.
    if flag("selected") {
        let list = window.list();
        list.first_row();
        list.toggle_cursor_row();
        list.extend_down();
        list.extend_down();
        // Leave the keyboard one row below the selection, so the shot shows a
        // cursor row that is *not* selected next to selected rows that are
        // not the cursor.
        list.next_row();
        settle(&window);
    }

    let (width, height) = (target.width(), target.height());
    let paintable = gtk::WidgetPaintable::new(Some(&target));
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
    let renderer = target
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
