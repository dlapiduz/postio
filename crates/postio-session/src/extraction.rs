//! Attachment text, read in a process the indexer can kill (spec 010 D28).
//!
//! An attachment is bytes a stranger chose, and `pdf-extract` was not
//! written for that: T121 found a file that overflows its stack and one
//! that loops it for ever. `postio-extract` refuses both before it starts,
//! but the next one nobody has found yet would cost a thread spinning for
//! the life of the application, or the application. So no attachment is
//! read here. Each one goes to `postio-extract-helper` -- a process of its
//! own, one per attachment -- as one request on its stdin, and comes back
//! as one reply on its stdout, in `postio_extract::wire`'s frame.
//!
//! # How it can end
//!
//! - The helper answers: its reply is the extraction ([`Via::Helper`]).
//! - It is still going at `Limits::max_time` plus [`HELPER_GRACE`]: it is
//!   killed and the attachment is `Truncated(Time)` with nothing kept
//!   ([`Via::Killed`]).
//! - It dies, by a signal or a non-zero status: `Failed` ([`Via::Crashed`]).
//! - It answers something that is not a reply, or more than any reply
//!   could be: `Failed` ([`Via::Garbled`]).
//! - There is no helper, or it answers for another extractor version (a
//!   build left behind): a PDF is **not** read here and is recorded
//!   `Skipped(Unavailable)` ([`Via::Refused`]); OOXML and plain text, read
//!   by walkers `postio-extract` wrote itself -- iterative, guarded and
//!   deadline-checked -- are read in this process ([`Via::InProcess`]), so
//!   a build without the helper still finds words in a spreadsheet.
//!
//! # One process per attachment
//!
//! Starting one costs a few milliseconds against an extraction of tens of
//! milliseconds to seconds and the indexer's own breather between batches.
//! In exchange a kill costs exactly one attachment, and nothing one
//! hostile file leaves behind -- a heap a bomb inflated, a thread a loop
//! still holds -- reaches the next.
//!
//! # The pipe that cannot kill the application
//!
//! The request goes in over one end of a socket pair rather than a pipe.
//! A write into a pipe whose reader has died raises `SIGPIPE`, which ends
//! the process that wrote; Rust's own `main` ignores it, but the Mac app's
//! `main` is Swift's, and a helper killed mid-request would take the app
//! with it. A socket can say "no signal" for itself: `SO_NOSIGPIPE` on
//! Apple, `MSG_NOSIGNAL` on each write elsewhere.

use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use postio_extract::{Extracted, Limit, Limits, Outcome, Skip, wire};

/// The environment variable that names the helper, ahead of every place it
/// is otherwise looked for.
pub const HELPER_ENV: &str = "POSTIO_EXTRACT_HELPER";

/// The helper's file name, beside the application's executable.
pub const HELPER_NAME: &str = "postio-extract-helper";

/// How long past `Limits::max_time` a helper may take before it is killed:
/// starting the process, reading a request of up to `max_input`, and
/// writing its reply, on a machine that may be busy. The extraction inside
/// stops itself at `max_time` and answers what it read; this is the margin
/// for an honest helper to say so.
pub const HELPER_GRACE: Duration = Duration::from_secs(1);

/// How the indexer reads attachments: which helper, and under what limits.
/// Cheap to clone; one indexer keeps one for its whole life, so it warns
/// once about a missing helper rather than once per file.
#[derive(Debug, Clone)]
pub struct Extractor {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    helper: PathBuf,
    limits: Limits,
    warned: AtomicBool,
}

/// The path an extraction took, for the log and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The helper answered.
    Helper,
    /// The helper was killed at its deadline.
    Killed,
    /// The helper died: a signal, or a status that is not success.
    Crashed,
    /// The helper answered something that is not a reply.
    Garbled,
    /// No usable helper, and the format is one this process may read.
    InProcess,
    /// No usable helper, and the format is a PDF: not read.
    Refused,
}

impl Via {
    /// One word for the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helper => "helper",
            Self::Killed => "killed",
            Self::Crashed => "crashed",
            Self::Garbled => "garbled",
            Self::InProcess => "in-process",
            Self::Refused => "refused",
        }
    }

    /// Whether this path means the helper could not be used at all.
    pub fn helper_unavailable(self) -> bool {
        matches!(self, Self::InProcess | Self::Refused)
    }
}

/// One attachment's extraction and the path it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// What was read.
    pub extracted: Extracted,
    /// How.
    pub via: Via,
}

impl Extractor {
    /// The helper this application ships: `$POSTIO_EXTRACT_HELPER`, else
    /// [`HELPER_NAME`] beside the running executable (`Contents/MacOS` in
    /// the .app, `/app/bin` in the flatpak, `target/<profile>` in a
    /// build), else in the directory above a test binary's `deps/`. When
    /// none exists the first candidate is kept, and each extraction finds
    /// it missing.
    pub fn locate() -> Self {
        if let Some(path) = std::env::var_os(HELPER_ENV).filter(|path| !path.is_empty()) {
            return Self::with_helper(path);
        }
        let name = format!("{HELPER_NAME}{}", std::env::consts::EXE_SUFFIX);
        let mut candidates = Vec::new();
        if let Some(directory) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            candidates.push(directory.join(&name));
            if directory.file_name().is_some_and(|last| last == "deps")
                && let Some(above) = directory.parent()
            {
                candidates.push(above.join(&name));
            }
        }
        let helper = candidates
            .iter()
            .find(|candidate| candidate.is_file())
            .or(candidates.first())
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&name));
        Self::with_helper(helper)
    }

    /// The helper at `path`, under the default limits.
    pub fn with_helper(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Limits::default())
    }

    /// The helper at `path`, under `limits`.
    pub fn new(path: impl Into<PathBuf>, limits: Limits) -> Self {
        Self {
            inner: Arc::new(Inner {
                helper: path.into(),
                limits,
                warned: AtomicBool::new(false),
            }),
        }
    }

    /// Where the helper is looked for.
    pub fn helper(&self) -> &Path {
        &self.inner.helper
    }

    /// Whether there is a file where the helper is looked for: what lets
    /// the indexer try again the PDFs it skipped while there was not. A
    /// file from another build passes this and is refused at the first
    /// extraction, which records those PDFs `unavailable` again.
    pub(crate) fn helper_present(&self) -> bool {
        self.inner.helper.is_file()
    }

    /// The limits every extraction runs under.
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// True the first time it is asked after the helper was found
    /// unusable, false after: the indexer's one warning.
    pub(crate) fn first_warning(&self) -> bool {
        !self.inner.warned.swap(true, Ordering::Relaxed)
    }

    /// Read one attachment. Blocking: it waits up to `max_time` plus
    /// [`HELPER_GRACE`] for the helper, so call it on the blocking pool.
    pub fn extract(&self, bytes: Vec<u8>, mime_type: &str, name: Option<&str>) -> Extraction {
        let limits = &self.inner.limits;
        let bytes = Arc::new(bytes);
        match run_helper(&self.inner.helper, &bytes, mime_type, name, limits) {
            Ok(extraction) => extraction,
            Err(Unusable) if postio_extract::is_pdf(mime_type, name, &bytes) => Extraction {
                extracted: Extracted {
                    units: Vec::new(),
                    outcome: Outcome::Skipped(Skip::Unavailable),
                },
                via: Via::Refused,
            },
            Err(Unusable) => Extraction {
                extracted: postio_extract::extract(&bytes, mime_type, name, limits),
                via: Via::InProcess,
            },
        }
    }
}

/// The helper is missing, cannot be started, or belongs to another build.
struct Unusable;

/// The most a reply may be: every byte of text the limits allow, generous
/// room for each unit's location, and the frame. More is not a reply.
fn reply_cap(limits: &Limits) -> u64 {
    limits.max_text as u64 + limits.max_units as u64 * 1024 + 1024 * 1024
}

fn run_helper(
    helper: &Path,
    bytes: &Arc<Vec<u8>>,
    mime_type: &str,
    name: Option<&str>,
    limits: &Limits,
) -> Result<Extraction, Unusable> {
    let deadline = Instant::now() + limits.max_time + HELPER_GRACE;
    let (ours, theirs) = request_channel().map_err(|_| Unusable)?;
    let mut child = Command::new(helper)
        // The helper needs nothing from the environment, and a log filter
        // or a credential has no business reaching a parser of strangers'
        // files.
        .env_clear()
        .stdin(theirs)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Unusable)?;
    let Some(stdout) = child.stdout.take() else {
        kill(&mut child);
        return Ok(failed(Via::Crashed));
    };

    // The request is written and the reply read on threads of their own,
    // so neither a helper that never reads nor one that never writes can
    // hold this one past the deadline. Neither is joined: each ends when
    // the helper's end of its channel closes, which a kill guarantees.
    let request = (
        Arc::clone(bytes),
        mime_type.to_owned(),
        name.map(str::to_owned),
        limits.clone(),
    );
    let writing = std::thread::Builder::new()
        .name("postio-extract-request".to_owned())
        .spawn(move || {
            let (bytes, mime_type, name, limits) = request;
            let mut ours = ours;
            let _ = wire::write_request(&mut ours, &bytes, &mime_type, name.as_deref(), &limits);
            // Dropping the socket is the helper's end of input.
        });
    if writing.is_err() {
        kill(&mut child);
        return Ok(failed(Via::Crashed));
    }
    let cap = reply_cap(limits);
    let (sender, receiver) = mpsc::channel();
    let reading = std::thread::Builder::new()
        .name("postio-extract-reply".to_owned())
        .spawn(move || {
            let mut reply = Vec::new();
            let read = stdout.take(cap + 1).read_to_end(&mut reply);
            let _ = sender.send(read.map(|_| reply));
        });
    if reading.is_err() {
        kill(&mut child);
        return Ok(failed(Via::Crashed));
    }

    let left = deadline.saturating_duration_since(Instant::now());
    let reply = match receiver.recv_timeout(left) {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            kill(&mut child);
            return Ok(failed(Via::Crashed));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            kill(&mut child);
            return Ok(Extraction {
                extracted: Extracted {
                    units: Vec::new(),
                    outcome: Outcome::Truncated(Limit::Time),
                },
                via: Via::Killed,
            });
        }
    };
    if reply.len() as u64 > cap {
        kill(&mut child);
        return Ok(failed(Via::Garbled));
    }

    // Its output is closed; it should be exiting. A helper that closes its
    // output and goes on running is still held to the deadline.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => {
                kill(&mut child);
                return Ok(Extraction {
                    extracted: Extracted {
                        units: Vec::new(),
                        outcome: Outcome::Truncated(Limit::Time),
                    },
                    via: Via::Killed,
                });
            }
            Err(_) => {
                kill(&mut child);
                return Ok(failed(Via::Crashed));
            }
        }
    };
    if !status.success() {
        return Ok(failed(Via::Crashed));
    }
    match wire::read_reply(&reply) {
        Ok(extracted) => Ok(Extraction {
            extracted,
            via: Via::Helper,
        }),
        Err(wire::Error::Extractor(_)) => Err(Unusable),
        Err(_) => Ok(failed(Via::Garbled)),
    }
}

fn failed(via: Via) -> Extraction {
    Extraction {
        extracted: Extracted {
            units: Vec::new(),
            outcome: Outcome::Failed,
        },
        via,
    }
}

/// Kill and reap. A helper that already ended is not an error.
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The request's channel: this process's end, written with no `SIGPIPE`,
/// and the helper's end as its stdin.
#[cfg(unix)]
fn request_channel() -> io::Result<(RequestWriter, Stdio)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    #[cfg(target_vendor = "apple")]
    rustix::net::sockopt::set_socket_nosigpipe(&ours, true)?;
    Ok((
        RequestWriter(ours),
        Stdio::from(std::os::fd::OwnedFd::from(theirs)),
    ))
}

#[cfg(unix)]
struct RequestWriter(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Write for RequestWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(target_vendor = "apple")]
        {
            // `SO_NOSIGPIPE` is set on the socket itself.
            self.0.write(buf)
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            rustix::net::send(&self.0, buf, rustix::net::SendFlags::NOSIGNAL).map_err(Into::into)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Elsewhere there is no helper yet: every extraction finds it unusable,
/// so a PDF is refused and the rest is read in this process.
#[cfg(not(unix))]
fn request_channel() -> io::Result<(RequestWriter, Stdio)> {
    Err(io::Error::other("the extraction helper needs a unix host"))
}

#[cfg(not(unix))]
struct RequestWriter;

#[cfg(not(unix))]
impl Write for RequestWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
