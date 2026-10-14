//! JSON-RPC 2.0 over stdio, as MCP frames it: one message per line.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::audit::Audit;
use crate::backend::Backend;
use crate::scope::Scope;
use crate::tools::{self, Tool};

/// The protocol versions this server speaks, newest first. A client asking
/// for one of them gets it; any other is offered the newest.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC's own error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The server: a backend to lease the store from, the `config.toml` whose
/// `[mcp]` says what is shared, and the access log.
pub struct Server<B: Backend> {
    backend: B,
    config: Option<PathBuf>,
    audit: Audit,
}

impl<B: Backend> Server<B> {
    /// A server over `backend`, sharing what `config` says.
    pub fn new(backend: B, config: Option<PathBuf>, audit: Audit) -> Self {
        Server {
            backend,
            config,
            audit,
        }
    }

    /// Answer one line; `None` for a notification, which has no answer.
    pub fn handle(&mut self, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(_) => return Some(error(Value::Null, PARSE_ERROR, "Not JSON.").to_string()),
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let (Some(method), true) = (method, message.get("jsonrpc") == Some(&json!("2.0"))) else {
            // A response to something this server never asks, or not
            // JSON-RPC at all.
            return id
                .map(|id| error(id, INVALID_REQUEST, "Not a JSON-RPC 2.0 request.").to_string());
        };
        // A notification -- `notifications/initialized`, a cancellation --
        // asks for nothing back, and none needs doing.
        let id = id?;
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let answer = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tools::definitions()),
            "tools/call" => self.call(&params),
            _ => Err((METHOD_NOT_FOUND, "Postio does not answer that method.")),
        };
        Some(
            match answer {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err((code, message)) => error(id, code, message),
            }
            .to_string(),
        )
    }

    /// `tools/call`: the tool, over what `[mcp]` shares as of now.
    fn call(&mut self, params: &Value) -> Result<Value, (i64, &'static str)> {
        let tool = params
            .get("name")
            .and_then(Value::as_str)
            .and_then(Tool::named)
            .ok_or((INVALID_PARAMS, "Postio has no tool by that name."))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments @ Value::Object(_)) => arguments.clone(),
            Some(_) => return Err((INVALID_PARAMS, "A tool's arguments are an object.")),
        };
        // Read again for every call: a person who takes an account out of
        // `[mcp]` has taken it out from the next call on (ADR 0010 Q6).
        let scope = Scope::load(self.config.as_deref());
        let answer = tools::call(tool, &arguments, &mut self.backend, &scope);
        self.audit
            .record(tool.name(), &answer.access, answer.outcome);
        Ok(answer.result())
    }
}

/// `initialize`'s answer: the version, what this server offers, and the
/// instructions an agent reads before it calls anything.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .into_iter()
        .find(|version| Some(*version) == asked)
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "postio", "title": "Postio", "version": env!("CARGO_PKG_VERSION")},
        "instructions": tools::INSTRUCTIONS,
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Answer every line of `input` on `output` until `input` ends.
pub fn serve<B: Backend>(
    input: impl BufRead,
    mut output: impl Write,
    server: &mut Server<B>,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = server.handle(&line) {
            writeln!(output, "{answer}")?;
            output.flush()?;
        }
    }
    Ok(())
}
