//! One binary for `postio-calendar`'s integration tests.
//!
//! Nothing here needs a display or a process of its own, so this keeps
//! libtest's ordinary harness, as `postio-body`'s suite does. A case that
//! grows a process-global has to move out.

mod calcard_spike;
