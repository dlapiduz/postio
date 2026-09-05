//! Render one view-layer surface straight out of GSK to a PNG.
//!
//! ```sh
//! cargo run -p postio-gtk --example surface -- /tmp/conversation.png conversation
//! cargo run -p postio-gtk --example surface -- /tmp/conversation.png conversation dark
//! cargo run -p postio-gtk --example surface -- /tmp/conversation.png conversation dark hc
//! cargo run -p postio-gtk --example surface -- /tmp/conversation.png conversation 900x700
//! cargo run -p postio-gtk --example surface -- /tmp/parts.png parts
//! ```
//!
//! # Why this is not `postio-app --example shot`
//!
//! `shot` renders the whole application from a seeded store, which is the
//! right way to check a screen against the canvas — and it lives in
//! `postio-app` because reading a store means `rusqlite`, which the view layer
//! may not link at any depth, dev-dependencies included.
//!
//! This is the other half of that trade. Some surfaces are reached by a
//! keystroke rather than by data — the conversation pane, the parts panel — and GTK4
//! offers no supported way to synthesize one, so `shot` cannot get to them
//! without knowing about them. These are also the surfaces that need no store
//! at all: every row here is a `crate::list::Row`, a type `postio-gtk` owns
//! and can build from nothing. So this stays in the view layer, brings no
//! database with it, and drives the surfaces by calling the same public
//! methods the window's own key handling calls.
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{app, capture, fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// Canvas 3a's own thread, six messages across four people.
///
/// Every address is a reserved domain, per CLAUDE.md.
fn conversation() -> Vec<Row> {
    let today = chrono::Local::now().date_naive();
    let at = |days: i64, hour: u32, minute: u32| {
        (today - chrono::Duration::days(days))
            .and_hms_opt(hour, minute, 0)
            .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
            .expect("a valid local time")
            .with_timezone(&chrono::Utc)
    };
    let message = |id: i64, name: &str, address: &str, subject: &str, when, seen| Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some(name), address)),
        subject: Some(subject.to_owned()),
        preview: None,
        received_at: when,
        seen,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    };

    vec![
        message(
            1,
            "Diogo Ferreira",
            "diogo@example.org",
            "index rebuild is O(n²)",
            at(4, 16, 20),
            true,
        ),
        message(
            2,
            "Sara Abadi",
            "sara@example.com",
            "Re: index rebuild is O(n²)",
            at(3, 11, 5),
            true,
        ),
        message(
            3,
            "buildbot",
            "buildbot@example.net",
            "[bench] rebuild 8m52s cold",
            at(2, 8, 52),
            true,
        ),
        message(
            4,
            "Diogo Ferreira",
            "diogo@example.org",
            "Re: index rebuild — profile attached",
            at(2, 14, 8),
            true,
        ),
        message(
            5,
            "Nadia Okafor",
            "nadia@example.org",
            "Re: index rebuild is O(n²)",
            at(0, 8, 2),
            false,
        ),
        message(
            6,
            "Lena Tomlin",
            "lena@example.com",
            "Re: index rebuild is O(n²)",
            at(0, 9, 14),
            false,
        ),
    ]
}

/// Canvas turn 8a: the conversation, stacked in the reading pane.
///
/// Through `Window::show_conversation`, which is what landing on a thread row
/// calls — so this renders the surface the application renders, minus the
/// keystroke that GTK will not let an example send.
fn show_conversation(window: &Window) {
    window.list().set_mailbox("Inbox", 12);
    window.show_conversation(conversation());
}

/// Canvas 3g's own message: four parts, one of them held back.
///
/// Built from `Attachment` metadata alone — a `part_id`, a type and a size —
/// which is exactly what `BODYSTRUCTURE` gives before a byte is transferred,
/// and the reason this needs no store to render.
fn show_parts(window: &Window) {
    use postio_model::Attachment;
    use postio_model::ids::AttachmentId;

    let message = MessageId::new(1);
    let part = |id: i64, path: &str, mime: &str, size: u64, filename: Option<&str>| {
        let mut part = Attachment::new(message, mime, size);
        part.id = AttachmentId::new(id);
        part.part_id = Some(path.to_owned());
        part.filename = filename.map(str::to_owned);
        part
    };

    window.open_parts(
        "multipart/mixed",
        &[
            part(1, "1", "text/plain", 2_100, None),
            part(2, "2", "text/html", 6 * 1024, None),
            part(3, "3", "text/x-diff", 11 * 1024, Some("0001-index.patch")),
            part(4, "4", "image/png", 1_100 * 1024, Some("cold.png")),
        ],
    );
    // The canvas draws the panel on the `text/html` part, held back.
    window.parts().set_held_back(3, 1);
    window.parts().next_part();
}

/// A sidebar with as many folders as a real account has.
///
/// `postio-qhz.4`: the live run that produced the Adwaita height warning had
/// fifteen. The folder lists are `GtkListBox`es in a plain box, so their
/// height is however many folders there are.
fn show_folders(window: &Window) {
    use postio_model::ids::{AccountId, MailboxId};
    use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

    let account = AccountId::new(1);
    let folder = |id: i64, path: &str, role, unread| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total: 400,
            unread,
            flagged: 0,
            snoozed: 0,
        };
        mailbox
    };
    let mut folders = vec![
        folder(1, "INBOX", MailboxRole::Inbox, 12),
        folder(2, "Drafts", MailboxRole::Drafts, 2),
        folder(3, "Sent", MailboxRole::Sent, 0),
        folder(4, "Archive", MailboxRole::Archive, 0),
        folder(5, "Junk", MailboxRole::Junk, 3),
        folder(6, "Trash", MailboxRole::Trash, 0),
    ];
    for (index, name) in [
        "lkml",
        "wayland-devel",
        "gtk-devel",
        "rust-internals",
        "notmuch",
        "mutt-users",
        "postfix",
        "dovecot",
        "receipts",
        "travel",
        "family",
        "recruiters",
    ]
    .iter()
    .enumerate()
    {
        folders.push(folder(
            10 + index as i64,
            name,
            MailboxRole::Regular,
            (index as u32 * 7) % 40,
        ));
    }
    window.sidebar().set_account("lena@example.com");
    window.sidebar().set_mailboxes(&folders);
}

/// Canvas 3e's own first-run screen.
///
/// Put in as the window's content, which is how `postio-app` shows it: one
/// window, no new navigation level, and nothing behind it to go back to.
fn show_onboarding(window: &Window, manual: bool, failed: bool) {
    use postio_gtk::onboarding::{Onboarding, Server, Settings, Status};

    let screen = Onboarding::new();
    screen.set_address("lena@example.com");
    // An iCloud address, because it is the case the whole screen is shaped
    // around: the password that will not work unless you are told.
    screen.set_status(Status::Found(Settings {
        imap: Server {
            host: "imap.mail.me.com".to_owned(),
            port: 993,
            security: Default::default(),
        },
        smtp: Server {
            host: "smtp.mail.me.com".to_owned(),
            port: 465,
            security: Default::default(),
        },
        login: "lena@example.com".to_owned(),
        requires_app_password: true,
        note: Some(
            "iCloud does not accept your Apple ID password here. Make an \
             app-specific password and paste that instead."
                .to_owned(),
        ),
        help_url: Some("https://appleid.apple.com/account/manage".to_owned()),
        oauth_sign_in: false,
        source: "Postio's provider list".to_owned(),
    }));
    if failed {
        screen.set_status(Status::Failed(
            "imap.mail.me.com rejected that password. iCloud needs an \
             app-specific password, not your Apple ID password."
                .to_owned(),
        ));
    }
    screen.show_manual(manual);
    window.set_content(Some(&screen));
}

/// How many frames to let the window paint before the shot is taken.
const SETTLE_FRAMES: u32 = 8;

/// The ceiling on that wait, so a window that never paints reports rather
/// than hangs.
const SETTLE_MS: u64 = 5000;

/// Run the main loop until `window` has painted [`SETTLE_FRAMES`] frames.
///
/// Not a spin count: `MainContext::iteration(false)` returns immediately when
/// nothing is pending, so a fixed number of them is not a wait at all and the
/// frame clock may never tick inside it. The heartbeat guarantees the blocking
/// iteration returns.
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

/// What to say when the size a `WxH` argument asked for and the size the
/// compositor actually gave the window disagree (#933).
///
/// `set_default_size` is a hint: a toplevel larger than the monitor is
/// clamped, and the height is clamped further by whatever the session
/// reserves. `capture::png` reports the size it actually got, so this is the
/// one place that can compare it against what was asked for and say so --
/// silently handing over a smaller picture under a size that looks honoured
/// is the same shape as #599, an argument that looks applied and was not.
///
/// `None` when nothing was requested, or when the compositor gave back
/// exactly what was asked for -- the ordinary case, which must stay silent.
fn size_mismatch(requested: Option<(i32, i32)>, got: (i32, i32)) -> Option<String> {
    let requested = requested?;
    if requested == got {
        return None;
    }
    let (want_w, want_h) = requested;
    let (got_w, got_h) = got;
    Some(format!(
        "asked for {want_w}x{want_h} but the compositor gave {got_w}x{got_h} -- \
         the picture below is at the size it actually got, not the size named \
         on the command line"
    ))
}

#[cfg(test)]
mod size_mismatch_tests {
    use super::*;

    #[test]
    fn nothing_requested_is_silent() {
        assert_eq!(size_mismatch(None, (1280, 800)), None);
    }

    #[test]
    fn the_size_asked_for_is_silent() {
        assert_eq!(size_mismatch(Some((1280, 800)), (1280, 800)), None);
    }

    #[test]
    fn a_clamped_size_is_reported() {
        let message = size_mismatch(Some((1600, 900)), (1280, 800))
            .expect("a clamped size should be reported");
        assert!(message.contains("1600x900"), "{message}");
        assert!(message.contains("1280x800"), "{message}");
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "surface.png".to_string());
    let flag = |name: &str| args.iter().skip(1).any(|a| a == name);
    let size = args.iter().skip(1).find_map(|a| {
        let (w, h) = a.split_once('x')?;
        Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
    });
    let scheme = if flag("dark") {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    };

    if adw::init().is_err() {
        eprintln!("surface: no display; rendering needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    // Same order as `app::run`: fonts before the first widget, or a
    // `PangoContext` caches the fallback family for the whole session.
    fonts::install().expect("the embedded fonts should install");
    let display = gdk::Display::default().expect("a display");
    style::install(&display);
    app::install_icons(&display);
    adw::StyleManager::default().set_color_scheme(scheme);

    let window = Window::default();
    if flag("hc") {
        window.add_css_class(style::HIGH_CONTRAST_CLASS);
    }
    if let Some((width, height)) = size {
        window.set_default_size(width, height);
    }
    for (name, density) in [
        ("comfortable", postio_config::Density::Comfortable),
        ("compact", postio_config::Density::Compact),
    ] {
        if flag(name) {
            window.list().set_density(density);
        }
    }
    if flag("conversation") {
        show_conversation(&window);
    }
    if flag("parts") {
        show_parts(&window);
    }
    if flag("folders") {
        show_folders(&window);
    }
    if flag("onboarding") {
        show_onboarding(&window, flag("manual"), flag("failed"));
    }
    // The list pane's fourth named state. `long` is the one worth looking at:
    // a query is user-typed and unbounded, and it is the only thing any of
    // these plates interpolates that the application does not control the
    // length of.
    if flag("nomatches") {
        window.set_searching(Some(if flag("long") {
            "from:ada subject:\"quarterly invoice\" has:attachment after:2026-01-01 before:2026-06-30"
        } else {
            "from:ada invoice"
        }));
    }
    window.present();
    settle(&window);
    // Same reason, and the point of the shot: the folder list's focus ring is
    // what says the keyboard is in this pane, and it cannot be looked at
    // without putting it there. `folders` on its own renders the pane at
    // rest, which is the other half worth seeing.
    if flag("folders") && flag("focused") {
        window
            .sidebar()
            .select(postio_model::ids::MailboxId::new(3));
        window.sidebar().focus_folders();
        settle(&window);
    }

    // The picture, and the wait for it, both belong to `postio_gtk::capture`
    // -- it turns the main loop until the window is actually drawable rather
    // than until a fixed number of frames has gone past, and it writes no
    // file when it cannot, so a non-zero exit here means exactly "there is
    // nothing to look at" (#809).
    match capture::png(&window, std::path::Path::new(&path)) {
        Ok(written) => {
            let (width, height) = (written.width, written.height);
            println!("surface: {width}x{height} -> {path}");
            if let Some(message) = size_mismatch(size, (width, height)) {
                eprintln!("surface: {message}");
            }
            if written.stalled {
                // Said out loud because the picture is misleading in one
                // specific way, and silently handing it over is how a
                // compositor problem gets read as an application one (#809).
                eprintln!(
                    "surface: the compositor was not presenting this window -- a blanked \
                     or locked screen -- so the layout was done here. The widgets \
                     are drawn correctly, but anything composited by another \
                     process, the reader's web view above all, will be blank."
                );
            }
            glib::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("surface: {error}");
            eprintln!("surface: NO IMAGE WAS WRITTEN to {path}");
            glib::ExitCode::FAILURE
        }
    }
}
