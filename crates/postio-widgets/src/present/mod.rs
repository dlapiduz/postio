//! Presenters both desktop apps would otherwise write twice (ADR 0043;
//! specs/007-postio-focus research R1): the joints between a widget and what
//! feeds it, with no window of either app in them.
//!
//! * [`config`] -- `config.toml` applied live: the watcher's thread to the
//!   main loop, one validated reload at a time.
//! * [`onboarding`] -- the account form joined to the store's host: its
//!   probe, its proof, its browser sign-in and its writes, and the
//!   add-account and credential dialogs built on them.
//! * [`export`] -- messages as `.eml` files for a drag out of either app's
//!   list, written only when a drop asks (T245).
//! * [`reading`] -- the reader's remote images, fetched off the main loop by
//!   whatever the app's composition root owns, and handed back on it; and its
//!   `cid:` images, resolved through `postio-client` scoped to the message on
//!   screen (specs/007-postio-focus T022).
//! * [`compose`] -- the composer's seams answered through `postio-client`
//!   (T022, T078): autosave and crash recovery, sending now or later,
//!   recipient completion, replying to a message, attaching a file, and an
//!   inline image's bytes.
//!
//! What an app does with a reload, which reader it feeds, or what a saved
//! account starts, stays in that app: these reach no window, no store and no
//! network of their own.

pub mod compose;
pub mod config;
pub mod export;
pub mod onboarding;
pub mod reading;
pub mod settings;
