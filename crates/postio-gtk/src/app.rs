//! The application object, and the startup order everything else depends on.
//!
//! [`run`] is the whole of `main`. The order it works in is not arbitrary:
//!
//! 1. Start the [`Timeline`] — the budget is measured from process start.
//! 2. `adw::init()`, so there is a display to fail loudly about.
//! 3. Register the embedded fonts **before the first widget exists**. A
//!    `PangoContext` keeps the family it has already resolved, so a face added
//!    later never reaches a label that already exists.
//! 4. Install the generated tokens and the bundled icon theme on the display.
//! 5. Build the application, and open a [`Window`] on `activate`.
//!
//! # Configuration
//!
//! The window is built on the registry's default bindings so that it can be
//! constructed without touching the disk — that is what lets the widget tests
//! run hermetically. `activate` then hands it to [`crate::config::install`],
//! which lays the user's `[keys]` over them and keeps doing so as the file
//! changes.

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::startup::{self, Phase, Timeline};
use crate::window::Window;
use crate::{fonts, resources, style};

/// The application ID: the D-Bus name, the desktop entry's basename, the
/// Wayland `app_id` the compositor matches a window to its entry by, and the
/// name of the bundled icon. All four have to agree.
pub const APP_ID: &str = "dev.postio.Postio";

/// The name of the binary that lands on `PATH`, as the desktop entry's `Exec`
/// spells it. The crate is `postio-gtk`; what a user types is `postio`.
pub const BINARY: &str = "postio";

/// Run Postio. This is `main`.
pub fn run() -> glib::ExitCode {
    let timeline = Timeline::start();

    if adw::init().is_err() {
        eprintln!("postio: no display; the UI needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    timeline.mark(Phase::Init);

    // Fonts first, before any widget: see the module docs.
    if let Err(error) = fonts::install() {
        // Recoverable: the design degrades to system fallbacks, which is
        // ugly but usable, and refusing to start over a font would be worse.
        eprintln!("postio: {error}");
    }
    timeline.mark(Phase::Fonts);

    if let Some(display) = gdk::Display::default() {
        style::install(&display);
        install_icons(&display);
    }
    timeline.mark(Phase::Styles);

    build_with(timeline).run()
}

/// The application object, with no windows open yet.
///
/// This builds and wires; it does **not** perform the startup order above.
/// [`run`] does that, and anything else driving this object — a test, a bench
/// — has to do the same three steps first: `adw::init`, [`fonts::install`],
/// then [`style::install`] and [`install_icons`].
pub fn build() -> adw::Application {
    build_with(Timeline::start())
}

/// As [`build`], recording into a timeline the caller already owns.
pub fn build_with(timeline: Timeline) -> adw::Application {
    resources::register();

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path(resources::PREFIX)
        .build();

    app.connect_activate(move |app| {
        // Launching Postio a second time raises the window that is already
        // open rather than opening another one, and must not overwrite the
        // startup that was actually measured.
        if let Some(open) = app.active_window() {
            open.present();
            return;
        }

        let window = Window::new(app);
        crate::config::install(&window);
        timeline.mark(Phase::Window);
        report_first_frame(&window, app, &timeline);
        window.present();
    });

    app
}

/// Make the bundled icon resolvable by name, and adopt it as the default.
pub fn install_icons(display: &gdk::Display) {
    resources::register();
    gtk::IconTheme::for_display(display).add_resource_path(resources::ICONS);
    gtk::Window::set_default_icon_name(APP_ID);
}

/// Close the timeline once the window is actually on screen, and act on the
/// benchmarking switches documented in [`crate::startup`].
fn report_first_frame(window: &Window, app: &adw::Application, timeline: &Timeline) {
    let timeline = timeline.clone();
    let app = app.clone();
    startup::on_first_frame(window, move || {
        timeline.mark(Phase::FirstFrame);
        if startup::enabled(startup::TRACE_ENV) {
            eprintln!("postio: {}", timeline.report());
        }
        if startup::enabled(startup::EXIT_ENV) {
            app.quit();
        }
    });
}
