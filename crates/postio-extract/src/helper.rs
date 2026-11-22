//! `postio-extract-helper`'s whole program: one request on stdin, one reply
//! on stdout; it opens no file, store or socket, so a hostile file costs one process.
//! Exit status: `0` reply written, `2` request unreadable, `3` reply unwritable; else a crash.

use std::io::{self, BufReader};
use std::process::ExitCode;

use crate::wire;

/// Room for the largest input (25 MB), what a reader inflates from it and a 16 MB stack.
#[cfg(any(target_os = "linux", target_os = "android"))]
const ADDRESS_SPACE: u64 = 2 * 1024 * 1024 * 1024;

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

/// Each ceiling is best effort: a platform that refuses one still has the parent's kill.
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
        // Rounded up, plus one: the extraction's own deadline stops it first.
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
