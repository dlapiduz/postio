//! The small controls both desktop apps draw, built once (ADR 0043).
//!
//! A button with its key beside it, a row of them, a one-line notice, the
//! chips, the key hints and the undo toast. Postio had three or four
//! hand-rolled copies of each; copies drift, and only one of the three
//! keycap implementations read the live keymap, so the other two claimed
//! keys a rebind had already moved (#1002).
//!
//! What lives here is the *drawing*. The rules these draw -- which key a
//! hint names, which participants fit on a line -- are in `postio_ui`,
//! where they can be proven without a display. The classes they wear are
//! dressed by this crate's `widgets.css`.

pub mod action_bar;
pub mod button;
pub mod chip;
pub mod keycap;
pub mod keyhint;
pub mod notice;
pub mod toast;

/// The design system's spacing ramp in whole pixels -- `S1` 3px to `S8`
/// 27px -- generated from the same tokens as `--postio-space-N` in
/// `metrics.css`, so a margin set in code and a padding in a stylesheet are
/// one number.
pub mod space {
    include!("../../data/space.rs");
}

pub use action_bar::{Action, ActionBar};
pub use button::{Kind, Size, icon_button};
pub use chip::{chip_button, filter_chip};
pub use keycap::KeycapButton;
pub use keyhint::KeyLine;
pub use notice::{NoticeBar, NoticeMenuItem};
