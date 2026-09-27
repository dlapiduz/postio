//! Presenters both desktop apps would otherwise write twice (ADR 0043;
//! specs/007-postio-focus research R1): the joints between a widget and what
//! feeds it, with no window of either app in them.
//!
//! * [`config`] -- `config.toml` applied live: the watcher's thread to the
//!   main loop, one validated reload at a time.
//! * [`reading`] -- the reader's remote images, fetched off the main loop by
//!   whatever the app's composition root owns, and handed back on it.
//!
//! What an app does with a reload, or which reader it feeds, stays in that
//! app: these reach no window, no store and no network of their own.

pub mod config;
pub mod reading;
