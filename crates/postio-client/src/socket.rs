//! Reaching the store's owner over its socket.
//!
//! `$XDG_RUNTIME_DIR/postio/daemon.sock`, private to this user, never TCP
//! (`contracts/protocol.md`). The connection's I/O runs on a small runtime of
//! its own, on a thread of its own, so whatever holds the [`Client`] -- GTK's
//! main loop, the terminal's tokio -- only ever awaits channels.
//!
//! When nothing answers, [`connect_or_start`] starts `postio-daemon` and
//! waits for it. A frontend never opens the store itself: if the daemon
//! cannot be reached, the frontend says so in a sentence and stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use postio_core::EventEnvelope;
use tokio::net::UnixStream;
use tokio::sync::oneshot;

use crate::Client;
use crate::api::{Call, Disconnected, Transport};
use crate::protocol::{
    BuildId, ClientId, ClientKind, Frame, PROTOCOL, Refusal, Req, Resp, read_frame, write_frame,
};

/// Where the daemon listens and locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    dir: PathBuf,
}

impl Endpoint {
    /// The endpoint under `dir` -- a test's temporary directory, or the real
    /// `$XDG_RUNTIME_DIR/postio`.
    pub fn at(dir: impl Into<PathBuf>) -> Endpoint {
        Endpoint { dir: dir.into() }
    }

    /// This user's endpoint, from `$XDG_RUNTIME_DIR`.
    pub fn from_env() -> Result<Endpoint, ConnectError> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(|dir| Endpoint::at(PathBuf::from(dir).join("postio")))
            .ok_or(ConnectError::NoRuntimeDir)
    }

    /// The directory, private to this user.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The socket.
    pub fn socket(&self) -> PathBuf {
        self.dir.join("daemon.sock")
    }

    /// The lock the daemon holds for its lifetime.
    pub fn lock(&self) -> PathBuf {
        self.dir.join("daemon.lock")
    }
}

/// Why there is no connection. Every one is a sentence a frontend can show.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// There is no per-user runtime directory to find the daemon in.
    #[error("Postio needs $XDG_RUNTIME_DIR to find its background service, and it is not set.")]
    NoRuntimeDir,
    /// Nothing is listening.
    #[error("Postio's background service is not running.")]
    NotRunning,
    /// The daemon is a different build.
    #[error(
        "Postio's background service is version {host}, and this is version {client}. \
         Quit every Postio window and start again."
    )]
    VersionMismatch {
        /// The daemon's build.
        host: BuildId,
        /// This build.
        client: BuildId,
    },
    /// The daemon could not be started.
    #[error("Postio's background service did not start: {0}")]
    Start(String),
    /// The connection failed partway.
    #[error("Postio lost its connection to its background service: {0}")]
    Broken(String),
}

/// Connect to a daemon that is already running.
pub fn connect(endpoint: &Endpoint, kind: ClientKind) -> Result<Client, ConnectError> {
    let stream = match std::os::unix::net::UnixStream::connect(endpoint.socket()) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(ConnectError::NotRunning);
        }
        Err(error) => return Err(ConnectError::Broken(error.to_string())),
    };
    stream
        .set_nonblocking(true)
        .map_err(|error| ConnectError::Broken(error.to_string()))?;

    let (outgoing, to_send) = async_channel::unbounded::<Frame>();
    let (arrived, events) = async_channel::unbounded::<EventEnvelope>();
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Resp>>>> = Arc::default();
    let (handshake, shaken) = std::sync::mpsc::channel::<Result<ClientId, ConnectError>>();

    let answers = Arc::clone(&pending);
    std::thread::Builder::new()
        .name("postio-client".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = handshake.send(Err(ConnectError::Broken(error.to_string())));
                    return;
                }
            };
            runtime.block_on(run(stream, kind, handshake, to_send, arrived, answers));
        })
        .map_err(|error| ConnectError::Broken(error.to_string()))?;

    let client = shaken
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| ConnectError::Broken("the handshake did not finish".to_owned()))??;
    Ok(Client::new(Arc::new(Socket {
        outgoing,
        pending,
        next: AtomicU64::new(1),
        events,
        _client: client,
    })))
}

/// The connection's whole life, on its own thread: the handshake, then
/// requests out and answers and events in, until either side goes away.
async fn run(
    stream: std::os::unix::net::UnixStream,
    kind: ClientKind,
    handshake: std::sync::mpsc::Sender<Result<ClientId, ConnectError>>,
    to_send: async_channel::Receiver<Frame>,
    arrived: async_channel::Sender<EventEnvelope>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Resp>>>>,
) {
    let broken = |error: &dyn std::fmt::Display| Err(ConnectError::Broken(error.to_string()));
    let mut stream = match UnixStream::from_std(stream) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = handshake.send(broken(&error));
            return;
        }
    };
    let hello = Frame::Hello {
        build: BuildId::current(),
        kind,
        protocol: PROTOCOL,
    };
    if let Err(error) = write_frame(&mut stream, &hello).await {
        let _ = handshake.send(broken(&error));
        return;
    }
    let client = match read_frame(&mut stream).await {
        Ok(Some(Frame::Welcome { client, .. })) => client,
        Ok(Some(Frame::Refused(Refusal::VersionMismatch { host, client }))) => {
            let _ = handshake.send(Err(ConnectError::VersionMismatch { host, client }));
            return;
        }
        // Still opening the store: as good as not running yet, which is what
        // `connect_or_start` waits through.
        Ok(Some(Frame::Refused(Refusal::Starting))) => {
            let _ = handshake.send(Err(ConnectError::NotRunning));
            return;
        }
        Ok(other) => {
            let _ = handshake.send(broken(&format!("an unexpected first answer: {other:?}")));
            return;
        }
        Err(error) => {
            let _ = handshake.send(broken(&error));
            return;
        }
    };
    let _ = handshake.send(Ok(client));

    let (mut reader, mut writer) = stream.into_split();
    let writing = tokio::spawn(async move {
        while let Ok(frame) = to_send.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                return;
            }
        }
    });
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(Frame::Response { id, body })) => {
                let waiting = pending.lock().expect("never poisoned").remove(&id);
                if let Some(waiting) = waiting {
                    let _ = waiting.send(body);
                }
            }
            Ok(Some(Frame::Event(envelope))) => {
                if arrived.send(envelope).await.is_err() {
                    break;
                }
            }
            Ok(Some(other)) => {
                tracing::warn!(frame = ?std::mem::discriminant(&other), "an unexpected frame");
            }
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "the connection to the daemon broke: {error}");
                break;
            }
        }
    }
    // Everything still waiting learns the host is gone: dropping the senders
    // resolves each `call` to `Disconnected`.
    pending.lock().expect("never poisoned").clear();
    writing.abort();
}

/// Connect, starting `daemon` first if nothing answers, and waiting up to two
/// seconds for it.
pub fn connect_or_start(
    endpoint: &Endpoint,
    kind: ClientKind,
    daemon: &Path,
) -> Result<Client, ConnectError> {
    match connect(endpoint, kind) {
        Err(ConnectError::NotRunning) => {}
        other => return other,
    }
    std::fs::create_dir_all(endpoint.dir())
        .map_err(|error| ConnectError::Start(error.to_string()))?;
    let mut command = std::process::Command::new(daemon);
    command
        .arg("--runtime-dir")
        .arg(endpoint.dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Its own process group: a Ctrl+C in the terminal that started it is
    // for the frontend, not for the daemon every frontend shares.
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .map_err(|error| ConnectError::Start(format!("{}: {error}", daemon.display())))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pause = Duration::from_millis(5);
    loop {
        match connect(endpoint, kind) {
            Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(100));
            }
            Err(ConnectError::NotRunning) => {
                return Err(ConnectError::Start(
                    "it did not answer within two seconds".to_owned(),
                ));
            }
            other => return other,
        }
    }
}

/// The `postio-daemon` beside this executable, or the one on `PATH`.
pub fn daemon_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("postio-daemon")))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("postio-daemon"))
}

/// The socket transport: requests are matched to answers by id; events are
/// forwarded as they arrive.
struct Socket {
    outgoing: async_channel::Sender<Frame>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Resp>>>>,
    next: AtomicU64,
    events: async_channel::Receiver<EventEnvelope>,
    _client: ClientId,
}

impl Transport for Socket {
    fn call(&self, request: Req) -> Call<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        self.pending
            .lock()
            .expect("never poisoned")
            .insert(id, answer);
        let sent = self.outgoing.try_send(Frame::Request { id, body: request });
        Box::pin(async move {
            sent.map_err(|_| Disconnected)?;
            answered.await.map_err(|_| Disconnected)
        })
    }

    fn post(&self, request: Req) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let _ = self.outgoing.try_send(Frame::Request { id, body: request });
    }

    fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        self.events.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listener that answers every hello with `answer`.
    fn fake_daemon(dir: &Path, answer: Frame) -> std::thread::JoinHandle<()> {
        let socket = Endpoint::at(dir).socket();
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                listener.set_nonblocking(true).unwrap();
                let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                let (mut stream, _) = listener.accept().await.unwrap();
                let hello = read_frame(&mut stream).await.unwrap().unwrap();
                assert!(matches!(
                    hello,
                    Frame::Hello {
                        kind: ClientKind::Test,
                        ..
                    }
                ));
                write_frame(&mut stream, &answer).await.unwrap();
            });
        })
    }

    #[test]
    fn with_nothing_listening_the_daemon_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let error = connect(&Endpoint::at(dir.path()), ClientKind::Test).unwrap_err();
        assert!(matches!(error, ConnectError::NotRunning), "{error}");
    }

    #[test]
    fn a_daemon_of_another_build_is_refused_naming_both_versions() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = fake_daemon(
            dir.path(),
            Frame::Refused(Refusal::VersionMismatch {
                host: BuildId("9.9.9+other".into()),
                client: BuildId::current(),
            }),
        );
        let error = connect(&Endpoint::at(dir.path()), ClientKind::Test).unwrap_err();
        let sentence = error.to_string();
        assert!(sentence.contains("9.9.9+other"), "{sentence}");
        assert!(sentence.contains(&BuildId::current().0), "{sentence}");
        daemon.join().unwrap();
    }

    #[test]
    fn a_welcome_is_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = fake_daemon(
            dir.path(),
            Frame::Welcome {
                client: ClientId(1),
                host_build: BuildId::current(),
            },
        );
        connect(&Endpoint::at(dir.path()), ClientKind::Test).expect("connected");
        daemon.join().unwrap();
    }

    #[test]
    fn the_endpoint_lives_under_the_runtime_dir() {
        let endpoint = Endpoint::at("/run/user/1000/postio");
        assert_eq!(
            endpoint.socket(),
            Path::new("/run/user/1000/postio/daemon.sock")
        );
        assert_eq!(
            endpoint.lock(),
            Path::new("/run/user/1000/postio/daemon.lock")
        );
    }
}
