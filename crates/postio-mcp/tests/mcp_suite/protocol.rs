//! The JSON-RPC and MCP framing every client relies on.

use serde_json::json;

use crate::harness::{INBOX_ONLY, World};

#[test]
fn initialize_names_the_server_and_offers_tools() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "a-suite", "version": "0"},
        }),
    );
    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18", "{answer}");
    assert_eq!(result["serverInfo"]["name"], "postio", "{answer}");
    assert!(result["capabilities"]["tools"].is_object(), "{answer}");
    assert!(
        result["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("untrusted")),
        "{answer}"
    );
}

#[test]
fn a_version_it_does_not_speak_is_answered_with_one_it_does() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request(
        "initialize",
        json!({"protocolVersion": "1999-01-01", "capabilities": {}, "clientInfo": {"name": "x", "version": "0"}}),
    );
    let offered = answer["result"]["protocolVersion"]
        .as_str()
        .expect("a version");
    assert_ne!(offered, "1999-01-01");
    assert!(offered.starts_with("20"), "{answer}");
}

#[test]
fn a_notification_is_not_answered() {
    let mut world = World::new(INBOX_ONLY);
    let line = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(world.server.handle(&line.to_string()), None);
}

#[test]
fn ping_is_answered_with_nothing_in_particular() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request("ping", json!({}));
    assert_eq!(answer["result"], json!({}), "{answer}");
}

#[test]
fn an_unknown_method_is_a_method_not_found_error() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request("resources/list", json!({}));
    assert_eq!(answer["error"]["code"], -32601, "{answer}");
}

#[test]
fn a_line_that_is_not_json_is_a_parse_error() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.server.handle("{not json").expect("answered");
    let answer: serde_json::Value = serde_json::from_str(&answer).expect("JSON");
    assert_eq!(answer["error"]["code"], -32700, "{answer}");
    assert_eq!(answer["id"], serde_json::Value::Null, "{answer}");
}

#[test]
fn an_unknown_tool_is_an_invalid_params_error() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request("tools/call", json!({"name": "summon", "arguments": {}}));
    assert_eq!(answer["error"]["code"], -32602, "{answer}");
}

#[test]
fn a_tool_called_with_the_wrong_arguments_says_so() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call("read_message", json!({"message": "seven"}));
    assert!(result.is_error(), "{}", result.0);
}
