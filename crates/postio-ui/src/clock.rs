//! The clock seam, which lives in `postio-model` so that what the model
//! stamps -- a draft's date, a queued send's -- reads the same clock as
//! what the interface draws. Re-exported here, where presentation code has
//! always found it.

pub use postio_model::clock::*;
