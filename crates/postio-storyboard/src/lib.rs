//! Storyboards: an interaction written down once, as steps with expectations,
//! in the vocabulary every Postio frontend shares (`specs/008-storyboards`).
//!
//! This crate is the pure half. It reads the storyboard files, decides which
//! apps a storyboard applies to, evaluates checks against an observation,
//! compares runs, and builds the review page. It never draws, never opens a
//! store and never touches the network -- each app's runner does the playing,
//! and calls in here for everything that is the same across apps, so two
//! runners cannot disagree about what a storyboard means.
//!
//! The file format is `contracts/storyboard-format.md`; the records are
//! `data-model.md`.

pub mod apply;
pub mod bundle;
pub mod check;
pub mod compare;
pub mod format;
pub mod key;
pub mod lint;
pub mod page;
pub mod parity;
pub mod prompt;
pub mod run;
pub mod verdicts;

#[cfg(test)]
mod fixtures;
