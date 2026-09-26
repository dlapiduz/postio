//! The reading renderer: message bodies drawn in-process, with no network
//! crate and no C anywhere on the path message content takes (spec 006
//! FR-001, FR-023a).
//!
//! **A skeleton.** Spec 006 chooses its engine by evaluation (research R0):
//! Blitz, here, against WebKit, the reader today. Until the maintainer
//! decides (T029) this crate holds only arm B's harness, in `examples/`. If
//! Blitz is chosen it grows into the renderer the plan describes; if not, it
//! is deleted.
