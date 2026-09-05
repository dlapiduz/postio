//! Postio's GTK4 / libadwaita frontend.
//!
//! This crate holds widgets, CSS, the keymap and the palette. It must contain
//! no SQL and no IMAP: it talks to `postio-core` over commands and events
//! only. See `CLAUDE.md` for the architectural invariants CI enforces.
//!
//! The design is PLATE, option 1b on the canvas: an airy native desktop with
//! the Industry identity — Barlow Condensed over Barlow, the steel accent,
//! hairline dividers — inside real Adwaita window chrome.
//!
//! The foundation:
//!
//! - `postio_ui::tokens` — the Industry design system, parsed and turned
//!   into GTK CSS by `build.rs` (a build-dependency, not a module here:
//!   nothing at runtime needs it, only the build script and the drift
//!   tests do — see #569). Retune `Design/_ds/industry-*/styles.css` and
//!   the app follows; nothing is retyped by hand.
//! - [`resources`] — the compiled GResource bundle carrying the stylesheets,
//!   the icon and the vendored OFL fonts.
//! - [`fonts`] — Barlow / Barlow Condensed / IBM Plex Mono, registered from
//!   the bundle so they resolve with nothing installed and nothing fetched.
//! - [`style`] — loads the generated tokens and Postio's own widget styles,
//!   and follows the system light / dark / high-contrast preference.
//!
//! The application:
//!
//! - [`app`] — the application object and the startup order, [`startup`] the
//!   instrumentation that proves it stays inside the budget.
//! - [`window`] — the window: chrome, breakpoints, and the [`state`] that
//!   survives a restart.
//! - [`header`] — the header bar the canvas draws.
//! - [`shell`] — the three panes, and the rule for how many of them fit.
//! - [`sidebar`] — the folders, their counts, and the sync status line.
//! - [`list`] — the message list's model, windowed over paged storage so a
//!   mailbox is never loaded into memory.
//! - [`keymap`] — the resolver behind every key press: sequences like `g g`,
//!   per-context meanings for `Esc`, and the rule that typing always wins.
//! - [`reader`] — the reading pane: a hardened `WebView` for message bodies.
//! - [`palette`] — the `Ctrl+K` overlay, generated from the command registry
//!   so every command is reachable without memorizing a key.
//! - [`cheatsheet`] — the `?` overlay, generated from the same table, so the
//!   key it prints is the key that is bound.
//! - [`config`] — `config.toml` applied live: the bridge from the watcher's
//!   own thread to the main context, where the widgets are.
//! - [`search`] — the `/` query bar: operators drawn as chips over the query
//!   they were parsed from, and Backspace that pops one whole.
//! - [`settings`] — the settings panel: canvas 3f, `config.toml` edited in
//!   place, with a validity line instead of a save button.
//! - [`composer`] — canvas 2a: compose takes over the reading pane, so the
//!   list never moves while you write.
//!
//! Startup order matters: register the fonts *before* the first widget is
//! built, then install the styles. [`app::run`] does it in that order, and is
//! the whole of `main`.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! adw::init()?;
//! postio_gtk::fonts::install()?;
//! let app = adw::Application::builder().application_id("dev.postio.Postio").build();
//! postio_gtk::style::install_for_application(&app);
//! # Ok(())
//! # }
//! ```

pub mod app;
pub mod autoscroll;
pub mod capture;
pub mod cheatsheet;
pub mod composer;
pub mod config;
pub mod conversation;
pub mod drag_out;
pub mod editor;
pub mod feed;
pub mod finder;
pub mod fonts;
pub mod header;
pub mod keymap;
pub mod list;
pub mod list_state;
pub mod list_view;
pub mod onboarding;
pub mod orientation;
pub mod palette;
pub mod parts;
pub mod reader;
pub mod resources;
pub mod row;
pub mod search;
// Moved to postio-ui (#566, ADR 0019): the selection model has no toolkit
// in it, and re-exporting keeps every call site and test resolving here.
pub use postio_ui::selection;
pub mod settings;
pub mod shell;
pub mod sidebar;
pub mod startup;
pub mod state;
pub mod style;
pub mod thread_row;
pub mod toast;
pub mod unavailable;
pub mod widgets;
pub mod window;
