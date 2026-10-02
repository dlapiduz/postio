//! The small controls both desktop apps draw, built once (ADR 0043).
//!
//! A button with its key beside it, a row of them, a one-line notice, the
//! chips, the key hints and the undo toast; and the pieces of the account
//! form, which both apps open: the plate it floats on, its labelled fields,
//! the segmented control, the callout and the kicker. Postio had three or four
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
pub mod checkrow;
pub mod chip;
pub mod chrome;
pub mod field;
pub mod keycap;
pub mod keyhint;
pub mod nav_row;
pub mod notes;
pub mod notice;
pub mod pickers;
pub mod plate;
pub mod recipients;
pub mod screen;
pub mod segmented;
pub mod settings_group;
pub mod toast;

/// The design system's spacing ramp in whole pixels -- `S1` 3px to `S8`
/// 27px -- generated from the same tokens as `--postio-space-N` in
/// `metrics.css`, so a margin set in code and a padding in a stylesheet are
/// one number.
pub mod space {
    include!("../../data/space.rs");
}

pub use action_bar::{Action, ActionBar};
pub use button::{Kind, Size, close_button, dress_icon, icon_button, icon_menu_button};
pub use checkrow::CheckRow;
pub use chip::{chip_button, filter_chip};
pub use chrome::{kicker, stat_line};
pub use field::field;
pub use keycap::KeycapButton;
pub use keyhint::KeyLine;
pub use nav_row::{nav_count, nav_name, nav_row};
pub use notes::{ListOrEmpty, callout, empty_note};
pub use notice::{NoticeBar, NoticeMenuItem};
pub use screen::under_window_chrome;
pub use segmented::SegmentedControl;
pub use settings_group::SettingsGroup;
