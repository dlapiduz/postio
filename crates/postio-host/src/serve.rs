//! Serving frontends over the daemon's socket.
//!
//! One listener per user, in `$XDG_RUNTIME_DIR/postio`: the directory is
//! `0700`, the socket `0600`, and a peer whose uid is not this process's is
//! turned away before it can say anything (`contracts/protocol.md`). A lock
//! held for the daemon's lifetime makes a second daemon exit cleanly rather
//! than fight the first for the store; Turso's own file lock is the backstop.
//!
//! The listener exists before the store is open. Until it is, every hello
//! is answered [`Refusal::Starting`], which a frontend waits through: the
//! keyring alone has taken 28 seconds on a real install, and a frontend that
//! gave up after two would never start on that machine.

use std::fs::File;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use postio_client::protocol::{
    BuildId, Frame, PROTOCOL, Refusal, decode, encode, read_frame, write_frame,
};
use postio_client::socket::Endpoint;

use crate::{Host, Inner};

/// Why the daemon cannot listen.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    /// Another daemon holds the lock: it is the store's owner, not this one.
    #[error("another Postio background service is already running")]
    AlreadyRunning,
    /// The directory, lock or socket could not be made.
    #[error("Postio's background service could not listen at {path}: {error}")]
    Io {
        /// What could not be made.
        path: String,
        /// Why.
        error: std::io::Error,
    },
}

/// A bound, locked listener.
pub struct Listener {
    listener: std::os::unix::net::UnixListener,
    /// This user; every peer must be it.
    uid: u32,
    // Held, not read: the lock is released when the daemon exits.
    _lock: File,
}

fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> BindError + '_ {
    move |error| BindError::Io {
        path: path.display().to_string(),
        error,
    }
}

/// Take the lock and bind the socket.
pub fn bind(endpoint: &Endpoint) -> Result<Listener, BindError> {
    let dir = endpoint.dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io(dir))?;
    // A directory that already existed keeps whatever mode it had; this one
    // must be this user's alone either way.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io(dir))?;
    let uid = std::fs::metadata(dir).map_err(io(dir))?.uid();

    let lock_path = endpoint.lock();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(io(&lock_path))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(BindError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(error)) => return Err(io(&lock_path)(error)),
    }

    // Holding the lock makes any socket already there a dead daemon's.
    let socket = endpoint.socket();
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(&socket)(error)),
    }
    let listener = std::os::unix::net::UnixListener::bind(&socket).map_err(io(&socket))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .map_err(io(&socket))?;
    Ok(Listener {
        listener,
        uid,
        _lock: lock,
    })
}

/// Read one frame, blocking, for the moments before there is a runtime.
fn read_frame_blocking(stream: &mut std::os::unix::net::UnixStream) -> Option<Frame> {
    use std::io::Read;
    let mut bytes = vec![0u8; 4];
    stream.read_exact(&mut bytes).ok()?;
    let length = u32::from_be_bytes(bytes[..4].try_into().ok()?);
    if length > postio_client::protocol::MAX_FRAME {
        return None;
    }
    bytes.resize(4 + length as usize, 0);
    stream.read_exact(&mut bytes[4..]).ok()?;
    decode(&bytes).ok().flatten().map(|(frame, _)| frame)
}

impl Listener {
    /// Answer every hello with [`Refusal::Starting`] until `ready` is set:
    /// the frontends that arrive while the store is opening wait rather than
    /// start a second daemon.
    pub fn answer_starting_until(&self, ready: &AtomicBool) {
        use std::io::Write;
        if self.listener.set_nonblocking(true).is_err() {
            return;
        }
        while !ready.load(Ordering::Relaxed) {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    if let Some(Frame::Hello { .. }) = read_frame_blocking(&mut stream) {
                        let _ = stream.write_all(&encode(&Frame::Refused(Refusal::Starting)));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    tracing::warn!(%error, "could not accept a frontend while starting: {error}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

impl Host {
    /// Serve frontends on `listener` until none has been connected for
    /// `idle`, or until the process is sent SIGTERM, then return. Blocks; call it outside any async context, and
    /// drop the host after it returns.
    pub fn serve(&self, listener: Listener, idle: Duration) {
        let inner = Arc::clone(&self.inner);
        self.inner.runtime().block_on(serve(inner, listener, idle));
    }
}

async fn serve(inner: Arc<Inner>, listener: Listener, idle: Duration) {
    let Listener {
        listener,
        uid,
        _lock,
    } = listener;
    let listener = match listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::UnixListener::from_std(listener))
    {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%error, "could not listen for frontends: {error}");
            return;
        }
    };
    let mut connected = inner.connected.subscribe();
    // SIGTERM stops the daemon the way the grace period does: engines first,
    // by whoever called `serve`, once it returns.
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    loop {
        let nobody = *connected.borrow_and_update() == 0;
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(connection(Arc::clone(&inner), stream, uid));
                }
                Err(error) => tracing::warn!(%error, "could not accept a frontend: {error}"),
            },
            _ = connected.changed() => {}
            () = tokio::time::sleep(idle), if nobody => {
                tracing::info!("no frontend for the grace period; stopping");
                return;
            }
            Some(()) = async {
                match terminate.as_mut() {
                    Some(signal) => signal.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                tracing::info!("asked to stop");
                return;
            }
        }
    }
}

/// One frontend's connection: the handshake, then its requests in and its
/// answers and events out, until it goes away.
async fn connection(inner: Arc<Inner>, stream: tokio::net::UnixStream, uid: u32) {
    match stream.peer_cred() {
        Ok(peer) if peer.uid() == uid => {}
        Ok(peer) => {
            tracing::warn!(peer = peer.uid(), "refused a connection from another user");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "could not tell who connected: {error}");
            return;
        }
    }
    let (mut reader, mut writer) = stream.into_split();
    let kind = match read_frame(&mut reader).await {
        Ok(Some(Frame::Hello {
            build,
            kind,
            protocol,
        })) => {
            if build != BuildId::current() || protocol != PROTOCOL {
                let refusal = Frame::Refused(Refusal::VersionMismatch {
                    host: BuildId::current(),
                    client: build,
                });
                let _ = write_frame(&mut writer, &refusal).await;
                return;
            }
            kind
        }
        _ => return,
    };
    let (client, events, notices) = inner.join(kind);
    let welcome = Frame::Welcome {
        client,
        host_build: BuildId::current(),
    };
    if write_frame(&mut writer, &welcome).await.is_err() {
        inner.leave(client);
        return;
    }

    // One writer, so answers and events never interleave inside a frame.
    let (out, to_write) = async_channel::unbounded::<Frame>();
    let writing = tokio::spawn(async move {
        while let Ok(frame) = to_write.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                return;
            }
        }
    });
    let telling = tokio::spawn({
        let out = out.clone();
        async move {
            while let Ok(envelope) = events.recv().await {
                if out.send(Frame::Event(envelope)).await.is_err() {
                    return;
                }
            }
        }
    });
    let noticing = tokio::spawn({
        let out = out.clone();
        async move {
            while let Ok(notification) = notices.recv().await {
                if out.send(Frame::Notify(notification)).await.is_err() {
                    return;
                }
            }
        }
    });
    while let Ok(Some(frame)) = read_frame(&mut reader).await {
        let Frame::Request { id, body } = frame else {
            continue;
        };
        match inner.answer_in_order(client, body) {
            crate::InOrder::Answered(answered) => {
                let _ = out.send(Frame::Response { id, body: answered }).await;
            }
            crate::InOrder::Pending(landing) => {
                let out = out.clone();
                tokio::spawn(async move {
                    let answered = landing.await;
                    let _ = out.send(Frame::Response { id, body: answered }).await;
                });
            }
            crate::InOrder::Later(request) => {
                let inner = Arc::clone(&inner);
                let out = out.clone();
                tokio::spawn(async move {
                    let answered = inner.answer(client, request).await;
                    let _ = out.send(Frame::Response { id, body: answered }).await;
                });
            }
        }
    }
    inner.leave(client);
    telling.abort();
    noticing.abort();
    drop(out);
    let _ = writing.await;
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use postio_client::protocol::ClientKind;
    use postio_client::socket::{ConnectError, connect};
    use postio_core::{Command, Event, MessageTarget};

    use super::*;
    use crate::tests::World;

    #[test]
    fn a_second_daemon_finds_the_first_and_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let _first = bind(&endpoint).expect("the first binds");
        assert!(matches!(bind(&endpoint), Err(BindError::AlreadyRunning)));
    }

    #[test]
    fn the_socket_and_its_directory_are_this_users_alone() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let _listener = bind(&endpoint).expect("binds");
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(endpoint.dir()), 0o700);
        assert_eq!(mode(&endpoint.socket()), 0o600);
    }

    #[test]
    fn while_the_store_opens_a_frontend_is_told_to_wait() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let listener = bind(&endpoint).expect("binds");
        let ready = Arc::new(AtomicBool::new(false));
        let answering = {
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || {
                listener.answer_starting_until(&ready);
                listener
            })
        };
        // Starting, not "not running": a frontend waits through this rather
        // than starting a second daemon or giving up.
        let error = connect(&endpoint, ClientKind::Test).unwrap_err();
        assert!(matches!(error, ConnectError::Starting), "{error}");
        ready.store(true, Ordering::Relaxed);
        answering.join().unwrap();
    }

    #[test]
    fn a_frontend_reads_and_acts_over_the_socket_and_hears_back() {
        let world = World::new();
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let listener = bind(&endpoint).expect("binds");
        let host = world.host();
        let serving = std::thread::scope(|scope| {
            let serving = scope.spawn(|| host.serve(listener, Duration::from_millis(300)));

            let client = connect(&endpoint, ClientKind::Tui)
                .expect("connects")
                .with_state(world.looking_at_the_message());
            let events = client.events();
            assert_eq!(world.inbox_rows(&client), 1);
            world
                .rt
                .block_on(client.send(Command::Archive {
                    target: MessageTarget::Selection,
                }))
                .expect("sent");
            world.hear(&events, |event| {
                matches!(event, Event::MessagesRemoved { .. })
            });
            assert_eq!(world.inbox_rows(&client), 0);

            // The last frontend leaves; the daemon stops after its grace.
            drop(client);
            let left = Instant::now();
            serving.join().unwrap();
            left.elapsed()
        });
        assert!(
            serving >= Duration::from_millis(250),
            "it waited out the grace period, not less: {serving:?}"
        );
    }

    #[test]
    fn a_notification_crosses_the_socket_to_the_elected_frontend_alone() {
        let world = World::new();
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let listener = bind(&endpoint).expect("binds");
        let host = world.host();
        std::thread::scope(|scope| {
            scope.spawn(|| host.serve(listener, Duration::from_millis(300)));
            let terminal = connect(&endpoint, ClientKind::Tui).expect("connects");
            let desktop = connect(&endpoint, ClientKind::Gtk).expect("connects");
            crate::notify::tests::arrive(&world);
            let told = |client: &postio_client::Client| {
                let notices = client.notifications();
                world.rt.block_on(async {
                    tokio::time::timeout(Duration::from_millis(500), notices.recv())
                        .await
                        .ok()
                        .and_then(Result::ok)
                })
            };
            let notification = told(&desktop).expect("the desktop app is told");
            assert_eq!(notification.mailbox, world.inbox());
            assert_eq!(told(&terminal), None);
            drop((terminal, desktop));
        });
    }

    #[test]
    fn a_frontend_of_another_build_is_refused() {
        let world = World::new();
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path().join("postio"));
        let listener = bind(&endpoint).expect("binds");
        let host = world.host();
        std::thread::scope(|scope| {
            scope.spawn(|| host.serve(listener, Duration::from_millis(200)));
            let mut stream =
                std::os::unix::net::UnixStream::connect(endpoint.socket()).expect("connects");
            use std::io::{Read, Write};
            stream
                .write_all(&encode(&Frame::Hello {
                    build: BuildId("0.0.0+elsewhere".into()),
                    kind: ClientKind::Test,
                    protocol: PROTOCOL,
                }))
                .unwrap();
            let mut answer = Vec::new();
            stream.read_to_end(&mut answer).unwrap();
            let (frame, _) = decode(&answer).unwrap().expect("a whole answer");
            assert!(
                matches!(frame, Frame::Refused(Refusal::VersionMismatch { .. })),
                "{frame:?}"
            );
        });
    }
}
