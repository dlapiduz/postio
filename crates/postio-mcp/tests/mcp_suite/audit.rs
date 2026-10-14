//! Every call is logged by id, never by content (ADR 0010 Q6).

use serde_json::json;

use crate::harness::{INBOX_ONLY, World};

#[test]
fn every_call_is_logged_with_what_it_returned_by_id() {
    let mut world = World::new(INBOX_ONLY);
    world.call("search_mail", json!({"query": "atlas"}));
    world.call("read_message", json!({"message": world.budget.get()}));
    world.call("read_message", json!({"message": world.archived.get()}));

    let lines = world.audit_lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0]["tool"], "search_mail");
    assert_eq!(lines[0]["messages"], json!([world.budget.get()]));
    assert_eq!(lines[0]["accounts"], json!([world.ada.get()]));
    assert_eq!(lines[0]["outcome"], "answered");
    assert_eq!(lines[1]["tool"], "read_message");
    assert_eq!(lines[1]["messages"], json!([world.budget.get()]));
    assert_eq!(lines[2]["outcome"], "refused");
    assert_eq!(lines[2]["messages"], json!([]));
    for line in &lines {
        assert!(
            line["at"].as_str().is_some_and(|at| at.contains('T')),
            "{line}"
        );
    }
}

#[test]
fn the_log_never_holds_what_the_mail_or_the_query_said() {
    let mut world = World::new(INBOX_ONLY);
    world.call("search_mail", json!({"query": "atlas"}));
    world.call("read_message", json!({"message": world.budget.get()}));
    world.call(
        "create_draft",
        json!({"to": ["grace@example.org"], "subject": "Lunch", "body": "Thursday?"}),
    );
    let text = std::fs::read_to_string(&world.audit).expect("the log");
    for content in [
        "atlas",
        "Atlas",
        "grace@example.org",
        "Lunch",
        "Thursday",
        "test@example.com",
    ] {
        assert!(!text.contains(content), "the log holds `{content}`: {text}");
    }
    let lines = world.audit_lines();
    assert_eq!(lines[2]["tool"], "create_draft");
    assert_eq!(
        lines[2]["drafts"].as_array().map(Vec::len),
        Some(1),
        "{text}"
    );
}
