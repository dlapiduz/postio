//! Focus's list: the model over the paged store, where its pages come from,
//! and the rows a person reads.

pub mod feed;
pub mod heading;
pub mod model;
pub mod pane;
pub mod row;

pub use feed::Feed;
pub use model::{FocusList, RowObject};
pub use pane::ListPane;
pub use postio_ui::focus_list::{Conversation, Digest, FocusRow};
pub use row::RowWidget;
