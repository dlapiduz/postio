//! An agent's calls are answered by the Postio app that has the store open
//! (ADR 0041, amended 2026-10-14).
//!
//! The app is this test binary run again with [`APP`] set: a host over a
//! demo store, a window connected to it, and the agent socket. The parent
//! is `postio-mcp`: a [`Server`] whose own backend refuses to open
//! anything, so every answer it gets came from the app.

use std::cell::Cell;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

use postio_client::protocol::ClientKind;
use postio_core::Event;
use postio_mcp::socket::{self, Asked};
use postio_mcp::{Audit, Backend, Lease, Server};
use serde_json::{Value, json};

/// Set in the child that plays the app: the socket to listen on.
const APP: &str = "POSTIO_MCP_TEST_APP_SOCKET";
/// The child's `config.toml` and access log.
const APP_CONFIG: &str = "POSTIO_MCP_TEST_APP_CONFIG";
const APP_AUDIT: &str = "POSTIO_MCP_TEST_APP_AUDIT";

const NAME: &str = "an_open_app_answers_the_agent_and_its_window_sees_the_change";

/// What the app shares: every folder, and archiving.
const SHARED: &str =
    "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"archive\"]\n";

/// `postio-mcp`'s own way to the store, which must never be taken while an
/// app answers.
struct Never(Rc<Cell<u32>>);

impl Backend for Never {
    fn lease(&mut self) -> Result<Lease, String> {
        self.0.set(self.0.get() + 1);
        Err("opened here, not by the app".to_owned())
    }
}

/// A host over the demo store, as an app has one.
fn demo_host() -> (postio_host::Host, tempfile::TempDir) {
    let (database, _) = postio_core::blocking::now(postio_demo::seeded(postio_demo::Seed::Small));
    let blobs_dir = tempfile::tempdir().expect("a directory");
    let blobs = postio_storage::BlobStore::open(
        blobs_dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("blobs");
    let host = postio_host::Host::start(database, blobs, |wiring| {
        wiring.with_secrets(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))
    })
    .expect("a host");
    (host, blobs_dir)
}

fn the_app(socket: &Path) {
    let (host, _blobs) = demo_host();
    // The window the person is looking at.
    let window = host.connect(ClientKind::Ffi);
    let heard = window.events();
    let listener = socket::listen(
        socket.to_path_buf(),
        host.connector(),
        std::env::var_os(APP_CONFIG).map(PathBuf::from),
        Audit::to_file(PathBuf::from(
            std::env::var_os(APP_AUDIT).expect("an audit path"),
        )),
    )
    .expect("the app listens");
    std::thread::spawn(move || {
        while let Ok(envelope) = heard.recv_blocking() {
            if let Event::ActionCompleted { description, .. } = envelope.event {
                println!("window: {description}");
                std::io::stdout().flush().expect("flushed");
            }
        }
    });
    println!("listening");
    std::io::stdout().flush().expect("flushed");
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
    drop(listener);
    drop(window);
    host.stop();
}

fn call(server: &mut Server<Never>, tool: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    });
    let answer = server.handle(&line.to_string()).expect("answered");
    let answer: Value = serde_json::from_str(&answer).expect("JSON");
    assert_eq!(answer["id"], 7, "{answer}");
    answer["result"].clone()
}

fn mail(result: &Value) -> Value {
    let text = result["content"][1]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}"));
    serde_json::from_str::<Value>(text).expect("JSON")["untrusted_mail_content"].clone()
}

#[test]
fn an_open_app_answers_the_agent_and_its_window_sees_the_change() {
    if let Some(socket) = std::env::var_os(APP) {
        the_app(Path::new(&socket));
        return;
    }

    let directory = tempfile::tempdir().expect("a directory");
    let socket = directory.path().join("agents").join("mcp.sock");
    let config = directory.path().join("config.toml");
    std::fs::write(&config, SHARED).expect("the config");
    let audit = directory.path().join("access.log");

    let mut app = Command::new(std::env::current_exe().expect("this test"))
        .args([NAME, "--exact", "--nocapture"])
        .env(APP, &socket)
        .env(APP_CONFIG, &config)
        .env(APP_AUDIT, &audit)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the app starts");
    let mut said = BufReader::new(app.stdout.take().expect("stdout")).lines();
    assert!(
        said.any(|line| line.is_ok_and(|line| line == "listening")),
        "the app never listened"
    );

    // postio-mcp shares nothing of its own: whatever it hears, the app's
    // `[mcp]` decided.
    let leases = Rc::new(Cell::new(0));
    let mine = directory.path().join("mine.toml");
    std::fs::write(&mine, "").expect("an empty config");
    let mut server = Server::new(Never(Rc::clone(&leases)), Some(mine), Audit::none())
        .answered_by_app(socket.clone());

    let found = call(
        &mut server,
        "search_mail",
        json!({"query": "design review"}),
    );
    assert_eq!(found["isError"], false, "{found}");
    let rows = mail(&found)["messages"].as_array().expect("rows").clone();
    let inbox_row = rows
        .iter()
        .find(|row| row["mailbox"] == "INBOX")
        .unwrap_or_else(|| panic!("a hit in the inbox: {rows:?}"));
    let message = inbox_row["message"].as_i64().expect("an id");

    let read = call(&mut server, "read_message", json!({"message": message}));
    assert_eq!(read["isError"], false, "{read}");

    let archived = call(
        &mut server,
        "archive_messages",
        json!({"messages": [message]}),
    );
    assert_eq!(archived["isError"], false, "{archived}");
    // The window the person is looking at was told, as for its own key.
    assert!(
        said.any(|line| line.is_ok_and(|line| line.starts_with("window: Archived 1 message"))),
        "the app's window never heard of the archive"
    );
    let reread = call(&mut server, "read_message", json!({"message": message}));
    assert_eq!(mail(&reread)["mailbox"], "Archive", "{reread}");
    assert_eq!(leases.get(), 0, "postio-mcp never opened the store itself");

    let log = std::fs::read_to_string(&audit).expect("the app's access log");
    assert!(log.contains("\"tool\":\"archive_messages\""), "{log}");

    // The app quits: its socket goes with it, and the next call is
    // postio-mcp's own to answer.
    drop(app.stdin.take());
    assert!(app.wait().expect("the app ends").success());
    assert!(!socket.exists(), "the socket outlived the app");
    let after = call(&mut server, "list_accounts", json!({}));
    assert_eq!(after["isError"], false, "{after}");
    let again = call(
        &mut server,
        "search_mail",
        json!({"query": "design review"}),
    );
    assert_eq!(again["isError"], true, "{again}");
    assert_eq!(
        leases.get(),
        0,
        "with nothing shared here, nothing is opened here either"
    );
}

fn private_folder(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let folder = root.join("agents");
    std::fs::create_dir(&folder).expect("a folder");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).expect("private");
    folder
}

#[test]
fn a_socket_left_by_an_app_that_crashed_is_nobody_and_is_replaced() {
    let directory = tempfile::tempdir().expect("a directory");
    let socket = private_folder(directory.path()).join("mcp.sock");
    // Bound and abandoned, as a crash leaves it.
    drop(std::os::unix::net::UnixListener::bind(&socket).expect("bound"));
    assert!(socket.exists());
    assert!(matches!(socket::ask(&socket, "{}"), Asked::NoApp));

    let (host, _blobs) = demo_host();
    let listener = socket::listen(socket.clone(), host.connector(), None, Audit::none())
        .expect("the next app takes its place");
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    assert!(matches!(socket::ask(&socket, &line), Asked::Answered(_)));
    drop(listener);
    host.stop();
}

#[test]
fn a_second_app_does_not_take_a_socket_another_is_answering_on() {
    let directory = tempfile::tempdir().expect("a directory");
    let socket = directory.path().join("agents").join("mcp.sock");
    let (host, _blobs) = demo_host();
    let first = socket::listen(socket.clone(), host.connector(), None, Audit::none())
        .expect("the first listens");
    let second = socket::listen(socket.clone(), host.connector(), None, Audit::none());
    assert!(second.is_err(), "{second:?}");
    drop(first);
    host.stop();
}

#[test]
fn a_socket_others_can_reach_is_not_trusted() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("a directory");
    let folder = private_folder(directory.path());
    let socket = folder.join("mcp.sock");
    let _listening = std::os::unix::net::UnixListener::bind(&socket).expect("bound");

    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).expect("opened up");
    assert!(matches!(socket::ask(&socket, "{}"), Asked::Refused(_)));

    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).expect("private");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).expect("opened up");
    assert!(matches!(socket::ask(&socket, "{}"), Asked::Refused(_)));
}

#[test]
fn the_app_makes_its_socket_private() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let directory = tempfile::tempdir().expect("a directory");
    let folder = directory.path().join("agents");
    std::fs::create_dir(&folder).expect("a folder");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).expect("open");
    let socket = folder.join("mcp.sock");
    let (host, _blobs) = demo_host();
    let listener =
        socket::listen(socket.clone(), host.connector(), None, Audit::none()).expect("listens");
    assert_eq!(
        std::fs::metadata(&folder).expect("the folder").mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::symlink_metadata(&socket)
            .expect("the socket")
            .mode()
            & 0o777,
        0o600
    );
    drop(listener);
    assert!(!socket.exists());
    host.stop();
}

#[test]
fn with_no_app_there_is_no_socket_and_the_call_is_answered_here() {
    let directory = tempfile::tempdir().expect("a directory");
    let socket = directory.path().join("agents").join("mcp.sock");
    assert!(matches!(socket::ask(&socket, "{}"), Asked::NoApp));
    let leases = Rc::new(Cell::new(0));
    let config = directory.path().join("config.toml");
    std::fs::write(&config, SHARED).expect("the config");
    let mut server = Server::new(Never(Rc::clone(&leases)), Some(config), Audit::none())
        .answered_by_app(socket.clone());
    call(&mut server, "list_accounts", json!({}));
    assert_eq!(leases.get(), 1, "answered here");
    assert!(!socket.exists(), "postio-mcp listens on nothing");
    assert!(!socket.parent().expect("a folder").exists());
}
