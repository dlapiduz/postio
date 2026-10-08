//! postio-render's integration tests, as one test binary.
//!
//! They were seventeen binaries, one per file, each statically linking the
//! whole renderer -- Blitz, stylo, vello, the fonts -- so every change to the
//! crate relinked all seventeen, and a tree carried seventeen copies of that
//! code on disk. The rest of the workspace made this move in #841; this crate
//! arrived after it.
//!
//! Nothing here needs a process of its own: no GTK (the renderer is
//! headless), no global allocator, no process-wide state beyond `support`'s
//! shared fonts, which are a `OnceLock` built to be shared. nextest still runs
//! every test in its own process.
//!
//! A file in this directory runs only if it is declared below, and
//! `scripts/checks/check-suite-modules.py` refuses one that is not.

mod support;

mod affordance_sweep;
mod contrast;
mod details;
mod egress;
mod engine_patches;
mod fidelity;
mod fonts;
mod hostile;
mod images;
mod layout;
mod presentation;
mod sharpness;
mod snapshot;
mod text_index;
mod thread;
mod thread_document;
mod treatment;
mod zoom;
