//! Postio's GTK4 / libadwaita frontend.
//!
//! This crate holds widgets, CSS, the keymap and the palette. It must contain
//! no SQL and no IMAP: it talks to `postio-core` over commands and events
//! only. See `CLAUDE.md` for the architectural invariants CI enforces.
//!
//! What is here today is the foundation the rest of the UI is built on:
//!
//! - [`tokens`] — the Industry design system, parsed and turned into GTK CSS
//!   by `build.rs`. Retune `Design/_ds/industry-*/styles.css` and the app
//!   follows; nothing is retyped by hand.
//! - [`resources`] — the compiled GResource bundle carrying that stylesheet
//!   and the vendored OFL fonts.
//! - [`fonts`] — Barlow / Barlow Condensed / IBM Plex Mono, registered from
//!   the bundle so they resolve with nothing installed and nothing fetched.
//! - [`style`] — loads the tokens and follows the system light / dark /
//!   high-contrast preference.
//!
//! Startup order matters: register the fonts *before* the first widget is
//! built, then install the styles.
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

pub mod fonts;
pub mod resources;
pub mod style;
pub mod tokens;
