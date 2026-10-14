//! What the read tools return: rows, and bodies as sanitised text.

use serde_json::json;

use crate::harness::{EVERY_FOLDER, INBOX_ONLY, World};

#[test]
fn a_search_finds_mail_in_the_query_language() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call("search_mail", json!({"query": "atlas"}));
    assert!(!result.is_error(), "{}", result.0);
    let rows = result.mail()["messages"].as_array().expect("rows").clone();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["message"], json!(world.budget.get()));
    assert_eq!(row["subject"], "Atlas budget");
    assert_eq!(row["from"][0]["address"], "grace@example.org");
    assert_eq!(row["mailbox"], "INBOX");
    assert_eq!(row["account"], json!(world.ada.get()));
    // The server's own block says how many, and nothing the mail said.
    assert!(result.notice().contains('1'), "{}", result.notice());
    assert!(!result.notice().contains("Atlas"), "{}", result.notice());
}

#[test]
fn a_search_returns_no_more_than_the_limit() {
    let mut world = World::new(EVERY_FOLDER);
    let result = world.call("search_mail", json!({"query": crate::harness::COMMON, "limit": 1}));
    assert_eq!(result.mail()["messages"].as_array().expect("rows").len(), 1);
}

#[test]
fn a_message_reads_as_its_text() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call("read_message", json!({"message": world.budget.get()}));
    assert!(!result.is_error(), "{}", result.0);
    let mail = result.mail();
    assert_eq!(mail["subject"], "Atlas budget");
    assert_eq!(mail["body_status"], "ready");
    assert!(
        mail["body"]
            .as_str()
            .is_some_and(|body| body.contains("The atlas budget is due Friday.")),
        "{mail}"
    );
}

#[test]
fn html_reads_as_text_with_no_script_markup_or_remote_reference() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call("read_message", json!({"message": world.html.get()}));
    assert!(!result.is_error(), "{}", result.0);
    let body = result.mail()["body"].as_str().expect("a body").to_owned();
    assert!(body.contains("Hello reader"), "{body}");
    assert!(body.contains("Read on."), "{body}");
    for never in ["<script", "steal", "cookie", "tracker.example.net", "<p", "<b>", "onclick"] {
        assert!(!result.all_text().contains(never), "`{never}` came back: {body}");
    }
}

#[test]
fn a_thread_reads_as_its_shared_messages_oldest_first() {
    let mut world = World::new(INBOX_ONLY);
    let read = world.call("read_message", json!({"message": world.budget.get()}));
    let Some(thread) = read.mail()["thread"].as_i64() else {
        panic!("the message is in a thread: {}", read.0);
    };
    let result = world.call("read_thread", json!({"thread": thread}));
    assert!(!result.is_error(), "{}", result.0);
    let messages = result.mail()["messages"].as_array().expect("messages").clone();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["message"], json!(world.budget.get()));
    assert!(messages[0]["body"].as_str().is_some_and(|body| body.contains("atlas budget")));
}

#[test]
fn a_thread_in_a_folder_nobody_shared_is_refused() {
    let mut world = World::new(EVERY_FOLDER);
    let thread = world.call("read_message", json!({"message": world.archived.get()})).mail()
        ["thread"]
        .as_i64()
        .expect("a thread");
    world.share(crate::harness::INBOX_ONLY);
    let result = world.call("read_thread", json!({"thread": thread}));
    assert!(result.is_error(), "{}", result.0);
    assert!(!result.all_text().contains("Quarterly"));
}

#[test]
fn reading_marks_nothing_read() {
    let mut world = World::new(INBOX_ONLY);
    world.call("read_message", json!({"message": world.budget.get()}));
    assert!(world.queued().is_empty(), "{:?}", world.queued());
}
