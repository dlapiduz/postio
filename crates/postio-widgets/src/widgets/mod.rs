//! The small controls Postio draws, built once (ADR 0043). This is the drawing only: the rules
//! are in `postio_ui` and the classes are dressed by `widgets.css`.

pub mod action_bar;
pub mod button;
pub mod checkrow;
pub mod chrome;
pub mod field;
pub mod keycap;
pub mod keyhint;
pub mod notes;
pub mod notice;
pub mod pickers;
pub mod plate;
pub mod recipients;
pub mod segmented;
pub mod settings_group;
pub mod toast;

/// Spacing ramp in whole pixels, `S1` 3px to `S8` 27px, generated from the tokens behind
/// `--postio-space-N`.
pub mod space {
    include!("../../data/space.rs");
}

pub use action_bar::{Action, ActionBar};
pub use button::{Kind, Size, close_button, dress_icon, icon_button, icon_menu_button};
pub use checkrow::CheckRow;
pub use chrome::{kicker, stat_line};
pub use field::field;
pub use keycap::KeycapButton;
pub use keyhint::KeyLine;
pub use notes::{ListOrEmpty, callout, empty_note};
pub use notice::{NoticeBar, NoticeMenuItem};
pub use segmented::SegmentedControl;
pub use settings_group::SettingsGroup;
