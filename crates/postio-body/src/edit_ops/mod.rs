//! The editing algebra: every change a person makes to a draft, as a pure function of the
//! document, the selection and an intent. Nothing else mutates a composer's text (ADR 0039).

pub(crate) mod flat;

pub use flat::MarkSet;
