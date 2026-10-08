//! The GTK half of storyboards (spec 008), which Postio's runner drives: how a
//! scripted key reaches a window, where typed text lands, whether the
//! keyboard is on something a person could use, when a frame has settled,
//! and the outlined frame a reviewer reads.
//!
//! Everything here works on a plain `gtk::Window` and knows nothing of the
//! app's own widgets.

pub mod deliver;
pub mod outline;
pub mod reach;
pub mod settle;
