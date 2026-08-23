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
//! - [`tokens`] — the Industry design system, parsed and turned into GTK CSS
//!   by `build.rs`. Retune `Design/_ds/industry-*/styles.css` and the app
//!   follows; nothing is retyped by hand.
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
//! - [`palette`] — the `Ctrl+K` overlay, generated from the command registry
//!   so every command is reachable without memorizing a key.
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
pub mod fonts;
pub mod header;
pub mod keymap;
pub mod list;
pub mod palette;
pub mod resources;
pub mod shell;
pub mod sidebar;
pub mod startup;
pub mod state;
pub mod style;
pub mod tokens;
pub mod window;
