//! Presenters both desktop apps would otherwise write twice (ADR 0043;
//! specs/007-postio-focus research R1): the joints between a widget and what
//! feeds it, with no window of either app in them.
//!
//! * [`config`] -- `config.toml` applied live: the watcher's thread to the
//!   main loop, one validated reload at a time.
//! * [`onboarding`] -- the account form joined to the store's host: its
//!   probe, its proof, its browser sign-in and its writes, and the
//!   add-account and credential dialogs built on them.
//! * [`reading`] -- the reader's remote images, fetched off the main loop by
//!   whatever the app's composition root owns, and handed back on it.
//!
//! What an app does with a reload, which reader it feeds, or what a saved
//! account starts, stays in that app: these reach no window, no store and no
//! network of their own.

pub mod config;
pub mod onboarding;
pub mod reading;
