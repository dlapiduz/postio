//! Render one view-layer surface straight out of GSK to a PNG.
//!
//! ```sh
//! cargo run -p postio-gtk --example surface -- /tmp/thread.png thread
//! cargo run -p postio-gtk --example surface -- /tmp/thread.png thread dark
//! cargo run -p postio-gtk --example surface -- /tmp/thread.png thread dark hc
//! cargo run -p postio-gtk --example surface -- /tmp/thread.png thread 900x700
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
//! keystroke rather than by data — thread drill-in, the parts panel — and GTK4
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
use gtk::{gdk, glib, graphene};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
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

/// Canvas 3a: the list column, drilled in.
///
/// Through `ThreadView::open`, which is what `Window::open_thread` calls when
/// `t` resolves — so this renders the surface the application renders, minus
/// the keystroke that GTK will not let an example send.
fn show_thread(window: &Window) {
    let rows = conversation();
    let subject = "maildir index rebuild is O(n²)";
    window.list().set_mailbox("Inbox", 12);
    window.show_thread(ThreadId::new(1), Some(subject), rows, 6);
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
    if flag("thread") {
        show_thread(&window);
    }
    window.present();
    settle(&window);
    // The thread column asks for the keyboard on the way in, and the focus
    // only actually lands once there is a surface to land on.
    if flag("thread") {
        window.thread().focus_rows();
        settle(&window);
    }

    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);
    let Some(node) = snapshot.to_node() else {
        eprintln!(
            "surface: no frame after {SETTLE_MS}ms — is the screen blanked or the \
             window occluded? Nothing is painted to a surface the compositor is \
             not showing."
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
            println!("surface: {width}x{height} -> {path}");
            glib::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("surface: cannot write {path}: {error}");
            glib::ExitCode::FAILURE
        }
    }
}
