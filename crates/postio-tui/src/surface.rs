//! What a click on one of Focus's surfaces lands on: a part of the surface
//! and its index. The surfaces are the Filtered view, the digest window, the
//! digest rules, the rule dialog and the capture frame; each draws its
//! parts and records them here, so the mouse does what the keys do.

/// A part of a surface that is one of many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A reason tab of Filtered, by its place.
    FilteredTab,
    /// A row of Filtered, by its place among the rows read.
    FilteredRow,
    /// The sweep's question: leave it be.
    SweepCancel,
    /// The sweep's question: sweep.
    SweepConfirm,
    /// The digest window's Summary or messages tab: 0 or 1.
    DigestTab,
    /// A message of the digest's plain list.
    DigestRow,
    /// A statement of the summary, by its place; its paragraph and its
    /// reference are the same click.
    DigestReference,
    /// The question over stopping a sender: leave it be.
    StopCancel,
    /// The question over stopping a sender: stop.
    StopConfirm,
}

/// What the surfaces hold between them.
#[derive(Debug, Default)]
pub struct Surfaces {
    /// The Filtered view, while it is the window's body.
    pub filtered: Option<crate::filtered::Filtered>,
    /// The sweep's question while it is up: how many it would move.
    pub sweep: Option<u32>,
    /// The digest window, while one is open.
    pub digest: Option<crate::digest::Window>,
}
