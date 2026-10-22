//! The server holds the store only while a call is being answered (ADR
//! 0041: one Postio has the store at a time).
//!
//! An agent's MCP server runs for as long as its session does. If it kept
//! the store open for that long, the person could not open their own mail
//! client. So it opens the store for a call and closes it after: between
//! calls another Postio can have it, and a call made while one does is
//! told so, and the next call after it closes is answered.
//!
//! The other Postio is this test binary run again with [`HOLD`] set. Two
//! processes because the store's lock is a POSIX record lock, which a
//! process never conflicts with itself on.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_mcp::{Audit, Installed, Server};
use serde_json::{Value, json};

/// Set in the child that holds the store open: the store key, in hex.
const HOLD: &str = "POSTIO_MCP_TEST_HOLD_STORE_KEY";
/// The store the child holds.
const STORE: &str = "POSTIO_MCP_TEST_STORE";

const NAME: &str = "the_store_is_held_only_while_a_call_is_answered";

/// A keyring holding `hex` as the store key.
fn keyring(hex: &str) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(hex),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

fn list_accounts(server: &mut Server<Installed>) -> Value {
    let line = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "list_accounts", "arguments": {}},
    });
    let answer = server.handle(&line.to_string()).expect("answered");
    serde_json::from_str::<Value>(&answer).expect("JSON")["result"].clone()
}

/// The other Postio, holding the store, and its output, which has to stay
/// open while it runs.
type Holder = (
    std::process::Child,
    std::io::Lines<BufReader<std::process::ChildStdout>>,
);

fn hold(store: &Path, hex: &str) -> Holder {
    let mut holder = Command::new(std::env::current_exe().expect("this test"))
        .args([
            postio_test_support::this_test!(NAME).as_str(),
            "--exact",
            "--nocapture",
        ])
        .env(STORE, store)
        .env(HOLD, hex)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the child runs");
    let mut lines = BufReader::new(holder.stdout.take().expect("stdout")).lines();
    assert!(
        lines.any(|line| line.is_ok_and(|line| line == "held")),
        "the other Postio never opened the store: the server is still holding it"
    );
    (holder, lines)
}

#[test]
fn the_store_is_held_only_while_a_call_is_answered() {
    if let Ok(hex) = std::env::var(HOLD) {
        let store = std::env::var(STORE).expect("the store");
        let host = postio_host::Host::open_at(None, Path::new(&store), keyring(&hex), &|_| {})
            .expect("the child opens it");
        println!("held");
        std::io::stdout().flush().expect("flushed");
        // Held until the parent closes our stdin.
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        host.stop();
        return;
    }

    let directory = tempfile::tempdir().expect("a directory");
    let store = directory.path().join("postio.db");
    let config = directory.path().join("config.toml");
    std::fs::write(
        &config,
        "[mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"*\"]\n",
    )
    .expect("the config");
    let hex = postio_storage::key::StoreKey::generate()
        .to_hex()
        .as_str()
        .to_owned();
    let mut server = Server::new(
        Installed::at(Some(config.clone()), store.clone(), keyring(&hex)),
        Some(config),
        Audit::none(),
    );

    // A call opens the store, answers, and lets it go.
    let answered = list_accounts(&mut server);
    assert_eq!(answered["isError"], false, "{answered}");

    // So another Postio can open it now...
    let (mut holder, _output) = hold(&store, &hex);
    // ...and a call while it does is told why there is no answer.
    let refused = list_accounts(&mut server);
    assert_eq!(refused["isError"], true, "{refused}");
    let notice = refused["content"][0]["text"].as_str().expect("a notice");
    assert!(notice.contains("already open"), "{notice}");

    // Once it closes, the next call is answered, without a restart.
    drop(holder.stdin.take());
    assert!(holder.wait().expect("the child ends").success());
    let again = list_accounts(&mut server);
    assert_eq!(again["isError"], false, "{again}");
}
