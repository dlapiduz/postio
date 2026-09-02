//! One binary for `postio-gtk`'s tests that never open a window.
//!
//! Seven files, 52 tests, and not one of them calls `adw::init` or asks for a
//! `gdk::Display` — they exercise the keymap, the list model, the desktop
//! entry, the extension command table and the reader's tokenizer, all of
//! which are logic that happens to live in a GTK crate.
//!
//! They were seven separate integration binaries. Cargo gives every file in
//! `tests/` its own executable, and in this crate an executable statically
//! links GTK, libadwaita and WebKit: **43 MB each, ~2.3 GB across the
//! crate's 53 binaries**, and linking — not running — is where the suite's
//! time goes. Measured across the workspace: ~3,000 tests execute in 108s
//! inside a `cargo test` step that takes ~497s (#841).
//!
//! Unlike its neighbour `gtk_suite`, this one keeps libtest's ordinary
//! harness. That suite needs `harness = false` because GTK may be
//! initialized from one thread per process and libtest runs `#[test]`s on a
//! thread pool (#41, #355). Nothing here touches GTK, so nothing here has
//! that constraint: these cases can run in parallel, and do.
//!
//! **The test of whether a file belongs here is `adw::init`, not a feeling
//! about whether it is "unit-ish".** A file that grows a window later has to
//! move to `gtk_suite`, or it will race the others and the loser will return
//! through its own no-display guard and be reported as a pass — which is
//! exactly the failure #355 records.

mod desktop_entry;
mod drag_out;
mod gtk_extension_commands;
mod keymap_defaults;
mod keymap_live;
mod list_model;
mod reader_tokens;
