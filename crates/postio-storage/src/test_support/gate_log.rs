//! What the write gate did, in order — the instrument behind the
//! interaction-under-load gate.
//!
//! [`WriteGate`](crate::WriteGate) promises that a background writer never
//! *begins* a write while an interactive writer is waiting, so a keystroke's
//! write waits for at most the one background unit already in progress. The
//! storage suite proves the rule on a bare gate; what it cannot prove is that
//! the sync pass honours it — that a pass re-takes the permit per write unit
//! rather than once per batch, and that a unit is a size a person can wait
//! for. That is a property of the callers, and it is asserted by counting
//! what the gate saw while a real pass ran, not by timing it.
//!
//! Every request and every grant is recorded here, process-wide, as they
//! happen. A test resets the log, runs its load, and reads the sequence back:
//! between an interactive request and its grant there must be no background
//! grant at all.

use std::sync::Mutex;

use crate::WritePriority;

/// One thing the gate did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A caller asked for the writer at this priority.
    Requested(WritePriority),
    /// A caller was handed the writer at this priority.
    Granted(WritePriority),
}

static LOG: Mutex<Vec<Event>> = Mutex::new(Vec::new());

/// Called by the gate when a caller asks for the writer.
pub fn requested(priority: WritePriority) {
    record(Event::Requested(priority));
}

/// Called by the gate when a caller is handed the writer.
pub fn granted(priority: WritePriority) {
    record(Event::Granted(priority));
}

fn record(event: Event) {
    if let Ok(mut log) = LOG.lock() {
        log.push(event);
    }
}

/// Forget everything recorded so far. A test calls this before its load.
pub fn reset() {
    if let Ok(mut log) = LOG.lock() {
        log.clear();
    }
}

/// Everything the gate did since the last [`reset`], in order.
pub fn events() -> Vec<Event> {
    LOG.lock().map(|log| log.clone()).unwrap_or_default()
}

/// How many background grants sit between the first interactive request
/// after `from` and the grant that answers it — the number the gate promises
/// is zero. `None` when no interactive request was recorded after `from`.
pub fn background_grants_while_interactive_waited(from: usize) -> Option<usize> {
    let log = events();
    let asked = log
        .iter()
        .skip(from)
        .position(|event| *event == Event::Requested(WritePriority::Interactive))?
        + from;
    let mut background = 0;
    for event in &log[asked + 1..] {
        match event {
            Event::Granted(WritePriority::Interactive) => return Some(background),
            Event::Granted(WritePriority::Background) => background += 1,
            Event::Requested(_) => {}
        }
    }
    None
}
