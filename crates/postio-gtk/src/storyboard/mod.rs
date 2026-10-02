//! The GTK half of storyboards (spec 008), shared by both GTK apps: how a
//! scripted key reaches a window, where typed text lands, whether the
//! keyboard is on something a person could use, when a frame has settled,
//! and the outlined frame a reviewer reads.
//!
//! Everything here works on a plain `gtk::Window` and knows nothing of
//! Classic or Focus.

pub mod deliver;
