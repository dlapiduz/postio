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
//! ```
//!
//! `demo` fills the panes with canvas 1b's own sample content, which is the
//! only way to check things like the selected row against the drawing before
//! there is a database to read. `settings` opens the canvas 3f panel over a
//! sample `config.toml` written to a scratch directory for the shot.
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network.

use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib, graphene};
use postio_core::ConnectionState;
use postio_gtk::sidebar::SyncStatus;
use postio_gtk::{app, fonts, style, window::Window};
use postio_model::ids::AccountId;
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// Canvas 1b's own sample account, so the drawing and the application can be
/// held up against each other.
fn populate(window: &Window) {
    let account = AccountId::new(1);
    let folder = |id: i64, path: &str, role, counts| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = postio_model::ids::MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = counts;
        mailbox
    };
    let counts = |total, unread, flagged| MailboxCounts {
        total,
        unread,
        flagged,
    };

    let sidebar = window.sidebar();
    // A reserved domain, per CLAUDE.md: the canvas' address is not ours to ship.
    sidebar.set_account("lena@example.com");
    sidebar.set_mailboxes(&[
        folder(1, "INBOX", MailboxRole::Inbox, counts(940, 12, 3)),
        folder(2, "Flagged", MailboxRole::Flagged, counts(940, 12, 3)),
        folder(3, "Drafts", MailboxRole::Drafts, counts(2, 0, 0)),
        folder(4, "Sent", MailboxRole::Sent, counts(4021, 0, 0)),
        folder(5, "Archive", MailboxRole::Archive, counts(38122, 0, 0)),
        folder(6, "lkml", MailboxRole::Regular, counts(9004, 204, 0)),
        folder(7, "wayland-devel", MailboxRole::Regular, counts(880, 37, 0)),
    ]);
    sidebar.select(postio_model::ids::MailboxId::new(1));
    sidebar.set_status(SyncStatus {
        state: ConnectionState::Online,
        last_sync: Instant::now().checked_sub(Duration::from_secs(12)),
        ..SyncStatus::default()
    });
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
    if flag("settings") {
        show_settings(&window);
    }
    if flag("compose") {
        show_composer(&window);
    }
    window.present();

    // Two frames: one to allocate, one to settle any size that depended on it.
    for _ in 0..200 {
        glib::MainContext::default().iteration(false);
    }

    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);

    let Some(node) = snapshot.to_node() else {
        eprintln!("shot: the window drew nothing");
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
