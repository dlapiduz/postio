//! The binary itself, over a pipe: what Claude Code and Claude Desktop
//! launch. A demo store stands in for the installation's, so nothing reads
//! the keyring.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn the_binary_speaks_mcp_on_stdin_and_stdout() {
    let directory = tempfile::tempdir().expect("a directory");
    let config = directory.path().join("config.toml");
    std::fs::write(&config, "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"*\"]\n")
        .expect("the config");
    let audit = directory.path().join("access.log");

    let mut child = Command::new(env!("CARGO_BIN_EXE_postio-mcp"))
        .args(["--demo", "small", "--audit"])
        .arg(&audit)
        .env("POSTIO_CONFIG", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("postio-mcp starts");
    let mut input = child.stdin.take().expect("stdin");
    let mut output = BufReader::new(child.stdout.take().expect("stdout"));
    let mut ask = move |message: Value| -> Option<Value> {
        writeln!(input, "{message}").expect("written");
        input.flush().expect("flushed");
        message.get("id")?;
        let mut line = String::new();
        output.read_line(&mut line).expect("an answer");
        Some(serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line}")))
    };

    let initialized = ask(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "suite", "version": "0"}},
    }))
    .expect("answered");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "postio", "{initialized}");
    assert_eq!(
        ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
        None
    );
    let tools = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).expect("answered");
    assert_eq!(tools["result"]["tools"].as_array().map(Vec::len), Some(6), "{tools}");
    let accounts = ask(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "list_accounts", "arguments": {}},
    }))
    .expect("answered");
    assert!(accounts.to_string().contains("test@example.com"), "{accounts}");

    drop(ask);
    let status = child.wait().expect("it exits when stdin closes");
    assert!(status.success(), "{status}");
    let log = std::fs::read_to_string(&audit).expect("the access log");
    assert!(log.contains("\"tool\":\"list_accounts\""), "{log}");
}
