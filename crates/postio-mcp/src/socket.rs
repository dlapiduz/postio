//! The agent socket: a running Postio app answers an agent's tool calls
//! (ADR 0041, amended 2026-10-14).
//!
//! One app holds the store at a time. Before this, an agent's call made
//! while the desktop app was open could only be told so. Now the app that
//! holds the store listens on a Unix domain socket beside it, and
//! `postio-mcp` hands each `tools/call` line to that app instead of opening
//! the store itself. The app answers with the same [`Server`] `postio-mcp`
//! runs, over its own host, so the person's `[mcp]` is read where the store
//! is and the access log is written there. The agent's client stands
//! beside the app's window ([`Connector::connect_agent`]): the window
//! repaints, hears the undo offer, and `u` takes the change back.
//!
//! What crosses is MCP's JSON-RPC, one line each way, which is already a
//! wire format. The client protocol between a frontend and the host stays
//! in-process, as ADR 0041 decided.
//!
//! # Who can connect
//!
//! Nothing on the network: a Unix domain socket has no port. It lives in a
//! folder only this user can open (`0700`), and the socket file itself is
//! `0600`. Both ends check the other's user id from the kernel (peer
//! credentials) and refuse anyone else. `postio-mcp` also refuses a socket,
//! or a folder around it, that another user owns or that others can reach,
//! rather than hand its calls to whatever is listening there.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use postio_host::Connector;

use crate::audit::Audit;
use crate::backend::Held;
use crate::server::Server;

/// How long `postio-mcp` waits for the app to answer one call.
const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// The longest line either end reads: far more than any call or answer.
const LINE_LIMIT: u64 = 64 * 1024 * 1024;

/// Where the app holding the store at `store` listens: `agents/mcp.sock`
/// beside it, so a second profile (`POSTIO_STORE`) has a socket of its own.
pub fn path_for(store: &Path) -> PathBuf {
    store
        .parent()
        .unwrap_or(Path::new("."))
        .join("agents")
        .join("mcp.sock")
}

/// Where this installation's app listens.
pub fn installed_path() -> PathBuf {
    path_for(&postio_session::paths::store_path())
}

/// This process's effective user id.
fn me() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Whether a file owned by `owner` with `mode` is private to `user`: theirs,
/// and nobody else may read, write or enter it.
pub fn private_to(user: u32, owner: u32, mode: u32) -> bool {
    owner == user && mode & 0o077 == 0
}

fn refused(what: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, what)
}

/// The kernel's word on who is at the other end of `stream`.
fn peer_of(stream: &UnixStream) -> std::io::Result<u32> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;
    let _entered = runtime.enter();
    // A clone shares the original's file description, and with it the
    // non-blocking flag tokio needs: set it back before handing back.
    let clone = stream.try_clone()?;
    clone.set_nonblocking(true)?;
    let peer = tokio::net::UnixStream::from_std(clone)?.peer_cred();
    stream.set_nonblocking(false)?;
    Ok(peer?.uid())
}

/// The folder at `directory`, made private to this user, or refused.
fn private_folder(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let meta = std::fs::symlink_metadata(directory)?;
    if !meta.is_dir() || meta.uid() != me() {
        return Err(refused(format!(
            "{} is not a folder this user owns",
            directory.display()
        )));
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
}

/// A socket that nothing answers on any more: left behind by an app that
/// stopped without removing it.
fn is_stale(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => false,
        Err(error) => matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        ),
    }
}

/// The app's end: answers agents on the socket until dropped, which stops
/// it and removes the socket.
pub struct Listener {
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    accepting: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("path", &self.path)
            .finish()
    }
}

impl Listener {
    /// Where it listens.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = UnixStream::connect(&self.path);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answer agents at `path` with `connector`'s host, sharing what `config`
/// says and logging to `audit`.
///
/// A socket left at `path` by an app that crashed is replaced; one that
/// something still answers on is not, and this fails, since one app holds
/// the store and it is already serving.
pub fn listen(
    path: PathBuf,
    connector: Connector,
    config: Option<PathBuf>,
    audit: Audit,
) -> std::io::Result<Listener> {
    let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    private_folder(&directory)?;
    if std::fs::symlink_metadata(&path).is_ok() {
        if !is_stale(&path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another Postio is already answering agents",
            ));
        }
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let stopping = Arc::new(AtomicBool::new(false));
    let accepting = {
        let stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("postio-agent-socket".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    match peer_of(&stream) {
                        Ok(uid) if uid == me() => {}
                        Ok(_) => {
                            tracing::warn!("refused an agent connection from another user");
                            continue;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "an agent connection said nothing of who it is: {error}");
                            continue;
                        }
                    }
                    let connector = connector.clone();
                    let config = config.clone();
                    let audit = audit.clone();
                    let _ = std::thread::Builder::new()
                        .name("postio-agent".to_owned())
                        .spawn(move || answer(stream, connector, config, audit));
                }
            })?
    };
    tracing::info!("answering agents on the agent socket");
    Ok(Listener {
        path,
        stopping,
        accepting: Some(accepting),
    })
}

/// [`listen`] at this installation's path, logging to its access log.
pub fn listen_installed(
    connector: Connector,
    config: Option<PathBuf>,
) -> std::io::Result<Listener> {
    listen(installed_path(), connector, config, Audit::installed())
}

/// One agent connection: each line a call, each answered with a line.
fn answer(stream: UnixStream, connector: Connector, config: Option<PathBuf>, audit: Audit) {
    let Ok(writing) = stream.try_clone() else {
        return;
    };
    let mut writing = std::io::BufWriter::new(writing);
    let mut server = Server::new(Held::new(connector), config, audit);
    let mut reading = BufReader::new(std::io::Read::take(stream, LINE_LIMIT));
    let mut line = String::new();
    while matches!(reading.read_line(&mut line), Ok(read) if read > 0) {
        if let Some(reply) = server.handle(line.trim_end())
            && (writeln!(writing, "{reply}").is_err() || writing.flush().is_err())
        {
            return;
        }
        line.clear();
    }
}

/// What asking the app came to.
#[derive(Debug)]
pub enum Asked {
    /// No app is listening: answer the call here.
    NoApp,
    /// The app answered with this line.
    Answered(String),
    /// Something is listening that this will not trust, or the app stopped
    /// answering: the sentence why.
    Refused(String),
}

/// Hand `line` to the app listening at `path`, if one is.
pub fn ask(path: &Path, line: &str) -> Asked {
    let stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        // No socket, or one left by an app that stopped: nobody is there.
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Asked::NoApp;
        }
        Err(error) => return Asked::Refused(format!("the agent socket did not connect: {error}")),
    };
    // Nothing is said to it until it is known to be this user's own.
    let directory = path.parent().unwrap_or(Path::new("."));
    let folder_private = std::fs::symlink_metadata(directory)
        .is_ok_and(|folder| folder.is_dir() && private_to(me(), folder.uid(), folder.mode()));
    let socket_private = std::fs::symlink_metadata(path)
        .is_ok_and(|socket| private_to(me(), socket.uid(), socket.mode()));
    if !folder_private || !socket_private {
        return Asked::Refused(
            "the agent socket, or the folder it is in, belongs to another user or is open \
             to others, so Postio did not use it"
                .to_owned(),
        );
    }
    match peer_of(&stream) {
        Ok(uid) if uid == me() => {}
        _ => {
            return Asked::Refused(
                "the program on the agent socket is not running as this user".to_owned(),
            );
        }
    }
    let exchanged = (|| {
        stream.set_read_timeout(Some(ANSWER_WITHIN))?;
        let mut writing = stream.try_clone()?;
        writeln!(writing, "{line}")?;
        writing.flush()?;
        let mut reply = String::new();
        BufReader::new(std::io::Read::take(stream, LINE_LIMIT)).read_line(&mut reply)?;
        Ok::<_, std::io::Error>(reply)
    })();
    match exchanged {
        Ok(reply) if !reply.trim().is_empty() => Asked::Answered(reply.trim_end().to_owned()),
        Ok(_) => {
            Asked::Refused("the Postio app closed the agent socket without answering".to_owned())
        }
        Err(error) => Asked::Refused(format!("the Postio app did not answer: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_file_this_user_owns_and_nobody_else_can_reach_is_private() {
        assert!(private_to(501, 501, 0o600));
        assert!(private_to(501, 501, 0o700));
        assert!(!private_to(501, 502, 0o600), "another user's");
        assert!(!private_to(501, 501, 0o660), "the group can reach it");
        assert!(!private_to(501, 501, 0o606), "anyone can reach it");
    }

    #[test]
    fn each_store_has_its_own_socket_beside_it() {
        assert_eq!(
            path_for(Path::new("/home/ada/.local/share/postio/postio.db")),
            PathBuf::from("/home/ada/.local/share/postio/agents/mcp.sock")
        );
    }
}
