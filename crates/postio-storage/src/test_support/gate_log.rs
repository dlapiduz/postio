//! Records every gate request and grant, process-wide, so a test can assert by counting that a sync
//! pass honours the gate: between an interactive request and its grant there is no background
//! grant.

use std::sync::Mutex;

use crate::WritePriority;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Requested(WritePriority),
    Granted(WritePriority),
}

static LOG: Mutex<Vec<Event>> = Mutex::new(Vec::new());

pub fn requested(priority: WritePriority) {
    record(Event::Requested(priority));
    if priority == WritePriority::Interactive {
        INTERACTIVE_HERE.with(|count| count.set(count.get() + 1));
    }
}

thread_local! {
    static INTERACTIVE_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The log is process-wide and unit tests share a process; a current-thread test's requests are
/// made on its own thread, so this is exact for it.
pub fn interactive_requested_here() -> usize {
    INTERACTIVE_HERE.with(std::cell::Cell::get)
}

pub fn granted(priority: WritePriority) {
    record(Event::Granted(priority));
}

fn record(event: Event) {
    if let Ok(mut log) = LOG.lock() {
        log.push(event);
    }
}

pub fn reset() {
    if let Ok(mut log) = LOG.lock() {
        log.clear();
    }
}

pub fn events() -> Vec<Event> {
    LOG.lock().map(|log| log.clone()).unwrap_or_default()
}

/// Background grants between the first interactive request after `from` and the grant that answers
/// it; the gate promises zero. `None` when no interactive request was recorded after `from`.
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
