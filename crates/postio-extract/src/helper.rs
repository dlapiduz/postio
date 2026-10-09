//! `postio-extract-helper`'s whole program (spec 010 D28).
//!
//! The indexer runs every extraction in a process of its own, so a file
//! that sends a reader into a loop or through the bottom of its stack
//! costs one process the indexer kills, never the application. This is
//! that process: one [request](crate::wire::read_request) on stdin, one
//! [reply](crate::wire::write_reply) on stdout, and nothing else. It opens
//! no file, no store and no socket; the bytes it reads are the only input
//! it has.
//!
//! The binary itself is one line in `postio-session` (`src/bin/`), so the
//! session's tests can find exactly the helper built from this source. The
//! program is here, in the pure leaf, so what the helper links is what
//! `check-crate-boundaries.py` already allows this crate.
//!
//! # It lowers its own ceiling
//!
//! Before it reads anything it gives up core files: a crash's memory is
//! the attachment's text, and it must not land on disk. Once it knows its
//! limits it takes a CPU-time ceiling a little past `Limits::max_time`, so
//! a helper whose parent died before it could kill it still ends by itself,
//! and on Linux an address-space ceiling, so a decompression bomb inside a
//! PDF stream runs out of room in the helper rather than in the machine.
//! The parent's wall-clock kill is still the deadline that matters; these
//! are what holds when nobody is left to enforce it.
//!
//! # Exit status
//!
//! `0` with a reply written; `2` when the request could not be read; `3`
//! when the reply could not be written. Anything else, or a signal, is a
//! crash, and the indexer records the attachment as failed either way.

use std::io::{self, BufReader};
use std::process::ExitCode;

use crate::wire;

/// The most address space the helper may map, on Linux: room for the
/// largest input (25 MB), everything a reader inflates from it, and a
/// 16 MB stack, with a wide margin; far short of what would hurt the
/// machine.
#[cfg(any(target_os = "linux", target_os = "android"))]
const ADDRESS_SPACE: u64 = 2 * 1024 * 1024 * 1024;

/// Run the helper: read the request on stdin, extract, write the reply on
/// stdout.
pub fn run() -> ExitCode {
    confine::no_core_files();
    let mut input = BufReader::new(io::stdin().lock());
    let request = match wire::read_request(&mut input) {
        Ok(request) => request,
        Err(_) => return ExitCode::from(2),
    };
    confine::for_limits(&request.limits);
    let extracted = crate::extract(
        &request.bytes,
        &request.mime_type,
        request.name.as_deref(),
        &request.limits,
    );
    drop(request);
    match wire::write_reply(&mut io::stdout().lock(), &extracted) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(3),
    }
}

/// The resource ceilings. Each is best effort: a platform that refuses one
/// still has the parent's kill, so a refusal is not a reason to stop.
#[cfg(unix)]
mod confine {
    use rustix::process::{Resource, Rlimit, setrlimit};

    use crate::Limits;

    pub(super) fn no_core_files() {
        let _ = setrlimit(
            Resource::Core,
            Rlimit {
                current: Some(0),
                maximum: Some(0),
            },
        );
    }

    pub(super) fn for_limits(limits: &Limits) {
        // Whole seconds, rounded up, plus one: the extraction's own
        // deadline stops it first, and the parent's kill shortly after.
        let seconds = limits.max_time.as_secs() + 2;
        let _ = setrlimit(
            Resource::Cpu,
            Rlimit {
                current: Some(seconds),
                maximum: Some(seconds + 1),
            },
        );
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let _ = setrlimit(
            Resource::As,
            Rlimit {
                current: Some(super::ADDRESS_SPACE),
                maximum: Some(super::ADDRESS_SPACE),
            },
        );
    }
}

#[cfg(not(unix))]
mod confine {
    use crate::Limits;

    pub(super) fn no_core_files() {}

    pub(super) fn for_limits(_: &Limits) {}
}
