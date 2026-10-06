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
}

/// What the surfaces hold between them.
#[derive(Debug, Default)]
pub struct Surfaces {
    /// The Filtered view, while it is the window's body.
    pub filtered: Option<crate::filtered::Filtered>,
    /// The sweep's question while it is up: how many it would move.
    pub sweep: Option<u32>,
}
