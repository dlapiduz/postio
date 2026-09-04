//! Does the first-window cost transfer to a splash window?
//!
//! #636 measured that whichever window a process realizes **first** pays a
//! one-time cost — ~110 ms on that machine — which every later window in the
//! same process avoids: a bare `Sidebar` presented first took 110 ms and the
//! same widget presented second took 12 ms. GSK's GPU renderers compile their
//! shaders lazily, on the first window they ever realize.
//!
//! #790 decided to keep the GPU renderer and asks whether a splash screen can
//! cover that cost instead. A splash cannot hide a cost it pays itself, so
//! the question this example answers is narrower and empirical: **does a
//! trivial window pay the same toll as a real one?**
//!
//! ```sh
//! cargo run -p postio-gtk --example first_realize -- real
//! cargo run -p postio-gtk --example first_realize -- splash real
//! ```
//!
//! Each argument names a window to present, in the order given. Every window
//! is measured from `present()` to its own first frame, and the next one is
//! presented only once the previous has drawn — so the numbers are
//! sequential, never overlapping.
//!
//! Read it this way:
//!
//! * If `splash` costs ~110 ms and the `real` after it costs ~12 ms, the toll
//!   **transfers**: a splash pays it, so it appears late and hides nothing of
//!   the expensive part.
//! * If `splash` is cheap and the `real` after it still costs ~110 ms, the
//!   toll is about the window's *content*, and a splash would genuinely cover
//!   it.
//!
//! This is a development tool: examples are not built into the shipped
//! binary. Nothing here touches the network or a store.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gtk::glib;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, startup, style};

/// Which window to present.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// A window with a single label — as little as GTK will draw.
    Splash,
    /// Postio's own window, the whole widget tree.
    Real,
}

impl Kind {
    fn parse(name: &str) -> Option<Kind> {
        match name {
            "splash" => Some(Kind::Splash),
            "real" => Some(Kind::Real),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Splash => "splash",
            Kind::Real => "real",
        }
    }
}

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let kinds: Vec<Kind> = names
        .iter()
        .map(|name| {
            Kind::parse(name).unwrap_or_else(|| {
                eprintln!("unknown window kind {name:?}; expected `splash` or `real`");
                std::process::exit(2);
            })
        })
        .collect();
    if kinds.is_empty() {
        eprintln!("usage: first_realize <splash|real>...");
        std::process::exit(2);
    }

    adw::init().expect("libadwaita initialises");
    fonts::install().expect("the bundled fonts register");
    let display = gtk::gdk::Display::default().expect("a display");
    style::install(&display);
    app::install_icons(&display);

    let application = adw::Application::builder()
        .application_id("com.postio.FirstRealize")
        .build();

    let remaining = Rc::new(RefCell::new(kinds));
    application.connect_activate(move |application| {
        present_next(application, &remaining);
    });

    // No command line to parse -- argv here names windows, not files.
    application.run_with_args::<&str>(&[]);
}

/// Present the next window, and the one after it once this one has drawn.
fn present_next(application: &adw::Application, remaining: &Rc<RefCell<Vec<Kind>>>) {
    let mut queue = remaining.borrow_mut();
    if queue.is_empty() {
        application.quit();
        return;
    }
    let kind = queue.remove(0);
    drop(queue);

    let window: gtk::Window = match kind {
        Kind::Splash => adw::ApplicationWindow::builder()
            .application(application)
            .default_width(480)
            .default_height(320)
            .content(&gtk::Label::new(Some("Postio")))
            .build()
            .upcast(),
        Kind::Real => Window::new(application).upcast(),
    };

    let started = Instant::now();
    startup::on_first_frame(&window, {
        let application = application.clone();
        let remaining = Rc::clone(remaining);
        move || {
            println!("{:>6}  {:.1}ms", kind.label(), started.elapsed().as_secs_f64() * 1000.0);
            // Let this frame finish before the next window is built, so the
            // measurement that follows is not timing this one's tail.
            glib::idle_add_local_once({
                let application = application.clone();
                let remaining = Rc::clone(&remaining);
                move || present_next(&application, &remaining)
            });
        }
    });
    window.present();
}
