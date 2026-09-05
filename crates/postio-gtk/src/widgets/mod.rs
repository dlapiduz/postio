//! The controls every surface draws, built once.
//!
//! Postio had three of these in three or four hand-rolled copies each — a
//! button with its key beside it, a row of them, a one-line notice. Copies
//! drift: only one of the three keycap implementations read the live keymap,
//! so the other two claimed keys a rebind had already moved.
//!
//! What lives here is the *drawing*. The rules these draw — which
//! participants fit on a line, for one — are in `postio_ui::conversation`,
//! where they can be proven without a display and reached by a second
//! frontend.
//!
//! The fourth control the canvas asks for, the mark that says which of four
//! states a row is in (turn 8b), is **not here yet**: both surfaces that
//! draw it are single-`snapshot()` widgets and neither is built, and a
//! mechanism wired to nothing is what `check-uncalled-pub-fn` exists to
//! catch. It lands with the collapsed conversation row it belongs to.

pub mod action_bar;
pub mod keycap;
pub mod notice;

pub use action_bar::{Action, ActionBar};
pub use keycap::KeycapButton;
pub use notice::{NoticeBar, NoticeMenuItem};
