//! Storyboards: an interaction written down once, as steps with expectations, in the vocabulary
//! every Postio frontend shares (`specs/008-storyboards`). The pure half: it never draws, opens a
//! store or touches the network, and each app's runner calls in here for what is the same across
//! apps.

pub mod apply;
pub mod bundle;
pub mod check;
pub mod compare;
pub mod coverage;
pub mod format;
pub mod key;
pub mod lint;
pub mod page;
pub mod parity;
pub mod prompt;
pub mod run;
pub mod shard;
pub mod verdicts;

#[cfg(test)]
mod fixtures;
