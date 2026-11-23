//! Attachment text, read in a process the indexer can kill. Attachments are hostile bytes and
//! `pdf-extract` can overflow its stack or loop, so each goes to `postio-extract-helper`, one
//! process per attachment.
//! A helper past `Limits::max_time` plus `HELPER_GRACE` is killed. With no helper (or another
//! version) a PDF is skipped and OOXML and plain text are read in process.
//! The request goes over a socket pair, not a pipe: a write to a dead pipe raises `SIGPIPE`, which
//! would end the Mac app (Swift's `main` does not ignore it).

use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use postio_extract::{Extracted, Limit, Limits, Outcome, Skip, wire};

pub const HELPER_ENV: &str = "POSTIO_EXTRACT_HELPER";

pub const HELPER_NAME: &str = "postio-extract-helper";

/// How long past `Limits::max_time` a helper may take before it is killed: process start, reading
/// the request and writing the reply on a busy machine.
pub const HELPER_GRACE: Duration = Duration::from_secs(1);

/// How the indexer reads attachments: which helper, under what limits. One indexer keeps one, so it
/// warns once about a missing helper.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Helper,
    Killed,
    Crashed,
    Garbled,
    InProcess,
    Refused,
}

impl Via {
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

    pub fn helper_unavailable(self) -> bool {
        matches!(self, Self::InProcess | Self::Refused)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub extracted: Extracted,
    pub via: Via,
}

impl Extractor {
    /// The helper this application ships: `$POSTIO_EXTRACT_HELPER`, else beside the running
    /// executable, else the `target/<profile>` a test binary was built under. When none exists the
    /// first candidate is kept.
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
            if let Some(profile) = test_profile_dir(&directory) {
                candidates.push(profile.join(&name));
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

    pub fn with_helper(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Limits::default())
    }

    pub fn new(path: impl Into<PathBuf>, limits: Limits) -> Self {
        Self {
            inner: Arc::new(Inner {
                helper: path.into(),
                limits,
                warned: AtomicBool::new(false),
            }),
        }
    }

    pub fn helper(&self) -> &Path {
        &self.inner.helper
    }

    /// Whether there is a file where the helper is looked for, which lets the indexer retry skipped
    /// PDFs; a file from another build passes and is refused at extraction.
    pub(crate) fn helper_present(&self) -> bool {
        self.inner.helper.is_file()
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    pub(crate) fn first_warning(&self) -> bool {
        !self.inner.warned.swap(true, Ordering::Relaxed)
    }

    /// Read one attachment. Blocking: waits up to `max_time` plus `HELPER_GRACE`, so call it on the
    /// blocking pool.
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

struct Unusable;

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
        // No log filter or credential should reach a parser of strangers' files.
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

    // Request and reply run on their own threads so a helper that never reads or writes cannot hold
    // this past the deadline; a kill ends both.
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

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

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

/// The `target/<profile>` directory a test binary in `directory` was built under: `<profile>/deps/`
/// until cargo 1.99, `<profile>/build/<package>/<hash>/out/` from 1.100. Other directories are not
/// a build, so an installed executable never offers a helper from somewhere nobody put one.
fn test_profile_dir(directory: &Path) -> Option<PathBuf> {
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|last| last == name);
    if named(directory, "deps") {
        return directory.parent().map(Path::to_path_buf);
    }
    let build = directory.parent()?.parent()?.parent()?;
    (named(directory, "out") && named(build, "build"))
        .then(|| build.parent().map(Path::to_path_buf))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_binary_under_deps_finds_its_profile() {
        assert_eq!(
            test_profile_dir(Path::new("/w/target/debug/deps")),
            Some(PathBuf::from("/w/target/debug"))
        );
    }

    #[test]
    fn a_test_binary_in_cargo_1_100s_build_directory_finds_its_profile() {
        assert_eq!(
            test_profile_dir(Path::new(
                "/w/target/debug/build/postio-session/3cccfa3113b09911/out"
            )),
            Some(PathBuf::from("/w/target/debug"))
        );
    }

    #[test]
    fn an_installed_executable_is_not_taken_for_a_build() {
        for installed in ["/app/bin", "/home/ada/build/bin", "/usr/bin", "/"] {
            assert_eq!(test_profile_dir(Path::new(installed)), None, "{installed}");
        }
    }
}
