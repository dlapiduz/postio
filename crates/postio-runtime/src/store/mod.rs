//! Reads the local store off any thread, awaited, handing back `postio_model` types so a frontend
//! has no SQL in its ancestry.
//! The count travels with the page because separate reads can disagree when mail arrives between
//! them.

pub use postio_model::ListScope;

pub use postio_model::listing::{
    ListPage, ListRows, MessagePage, MessageSummary, PageRequest, StoreError, ThreadPage,
    ThreadSummary,
};

pub use postio_model::listing::{MailStore, Read};

mod local;
pub use local::LocalStore;
#[doc(hidden)]
pub use local::{focus_counted, folders_counted, last_thread_skip, unified_counted};
