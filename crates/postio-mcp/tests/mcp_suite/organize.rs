//! Archive, move and label: the writes the maintainer added to ADR 0010 on
//! 2026-10-14. Each goes through the command path the apps use, and each
//! is off until `actions` in `[mcp]` turns it on.

use postio_model::Operation;
use serde_json::json;

use crate::harness::{EVERY_FOLDER, INBOX_ONLY, ORGANIZE, World};

#[test]
fn archiving_moves_the_message_into_the_archive_and_queues_the_move() {
    let mut world = World::new(ORGANIZE);
    let result = world.call(
        "archive_messages",
        json!({"messages": [world.budget.get()]}),
    );
    assert!(!result.is_error(), "{}", result.0);
    assert_eq!(world.mailbox_of(world.budget), world.archive);
    assert!(
        world
            .queued()
            .iter()
            .any(|operation| matches!(operation, Operation::Move { .. })),
        "the server learns of it the way it learns of a keystroke: {:?}",
        world.queued()
    );
    // Where each came from, so the agent can put it back.
    assert_eq!(
        result.mail()["messages"][0],
        json!({"message": world.budget.get(), "from": world.inbox.get()})
    );
}

#[test]
fn a_move_puts_the_message_in_the_folder_named() {
    let mut world = World::new(ORGANIZE);
    let result = world.call(
        "move_messages",
        json!({"messages": [world.budget.get(), world.html.get()], "mailbox": world.projects.get()}),
    );
    assert!(!result.is_error(), "{}", result.0);
    assert_eq!(world.mailbox_of(world.budget), world.projects);
    assert_eq!(world.mailbox_of(world.html), world.projects);
}

#[test]
fn a_label_is_added_and_taken_off() {
    let mut world = World::new(ORGANIZE);
    let added = world.call(
        "add_label",
        json!({"messages": [world.budget.get()], "label": "Follow up"}),
    );
    assert!(!added.is_error(), "{}", added.0);
    assert_eq!(world.labels_of(world.budget), vec!["Follow up".to_owned()]);

    let removed = world.call(
        "remove_label",
        json!({"messages": [world.budget.get()], "label": "follow up"}),
    );
    assert!(!removed.is_error(), "{}", removed.0);
    assert!(world.labels_of(world.budget).is_empty());
}

#[test]
fn taking_off_a_label_that_does_not_exist_is_refused() {
    let mut world = World::new(ORGANIZE);
    let result = world.call(
        "remove_label",
        json!({"messages": [world.budget.get()], "label": "Never made"}),
    );
    assert!(result.is_error(), "{}", result.0);
}

#[test]
fn without_actions_reading_is_all_an_agent_may_do() {
    let mut world = World::new(EVERY_FOLDER);
    let calls = [
        (
            "archive_messages",
            json!({"messages": [world.budget.get()]}),
        ),
        (
            "move_messages",
            json!({"messages": [world.budget.get()], "mailbox": world.projects.get()}),
        ),
        (
            "add_label",
            json!({"messages": [world.budget.get()], "label": "X"}),
        ),
        (
            "remove_label",
            json!({"messages": [world.budget.get()], "label": "X"}),
        ),
    ];
    for (tool, arguments) in calls {
        let result = world.call(tool, arguments);
        assert!(result.is_error(), "{tool}: {}", result.0);
        assert!(
            result.notice().contains("actions"),
            "{tool}: {}",
            result.notice()
        );
    }
    assert_eq!(world.mailbox_of(world.budget), world.inbox);
    assert!(world.labels_of(world.budget).is_empty());
    assert!(world.queued().is_empty(), "{:?}", world.queued());
}

#[test]
fn each_action_is_allowed_on_its_own() {
    let mut world = World::new(
        "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"label\"]\n",
    );
    let archived = world.call(
        "archive_messages",
        json!({"messages": [world.budget.get()]}),
    );
    assert!(archived.is_error(), "{}", archived.0);
    let labelled = world.call(
        "add_label",
        json!({"messages": [world.budget.get()], "label": "Later"}),
    );
    assert!(!labelled.is_error(), "{}", labelled.0);
    assert_eq!(world.mailbox_of(world.budget), world.inbox);
}

#[test]
fn a_message_in_a_folder_nobody_shared_is_not_there_to_move() {
    let mut world = World::new(
        "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"INBOX\", \"Projects\"]\n\
         actions = [\"move\", \"label\"]\n",
    );
    let hidden = world.call(
        "move_messages",
        json!({"messages": [world.archived.get()], "mailbox": world.projects.get()}),
    );
    let missing = world.call(
        "move_messages",
        json!({"messages": [987_654], "mailbox": world.projects.get()}),
    );
    assert!(hidden.is_error() && missing.is_error());
    assert_eq!(hidden.notice(), missing.notice());
    assert_eq!(world.mailbox_of(world.archived), world.archive);

    let labelled = world.call(
        "add_label",
        json!({"messages": [world.elsewhere.get()], "label": "X"}),
    );
    assert!(labelled.is_error(), "{}", labelled.0);
}

#[test]
fn a_folder_nobody_shared_is_not_there_to_move_into() {
    let mut world = World::new(
        "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"INBOX\"]\n\
         actions = [\"archive\", \"move\"]\n",
    );
    let moved = world.call(
        "move_messages",
        json!({"messages": [world.budget.get()], "mailbox": world.projects.get()}),
    );
    let nowhere = world.call(
        "move_messages",
        json!({"messages": [world.budget.get()], "mailbox": 987_654}),
    );
    assert!(moved.is_error() && nowhere.is_error());
    assert_eq!(moved.notice(), nowhere.notice());
    // The archive is a folder too: archiving into one nobody shared is
    // refused.
    let archived = world.call(
        "archive_messages",
        json!({"messages": [world.budget.get()]}),
    );
    assert!(archived.is_error(), "{}", archived.0);
    assert_eq!(world.mailbox_of(world.budget), world.inbox);
    assert!(world.queued().is_empty(), "{:?}", world.queued());
}

#[test]
fn moving_to_the_trash_or_junk_is_deleting_and_is_refused() {
    let mut world = World::new(ORGANIZE);
    for folder in [world.trash, world.junk] {
        let result = world.call(
            "move_messages",
            json!({"messages": [world.budget.get()], "mailbox": folder.get()}),
        );
        assert!(result.is_error(), "{}", result.0);
        assert!(result.notice().contains("delet"), "{}", result.notice());
    }
    assert_eq!(world.mailbox_of(world.budget), world.inbox);
    assert!(world.queued().is_empty(), "{:?}", world.queued());
}

#[test]
fn every_organizing_call_is_logged_by_id() {
    let mut world = World::new(ORGANIZE);
    world.call(
        "archive_messages",
        json!({"messages": [world.budget.get()]}),
    );
    world.call(
        "add_label",
        json!({"messages": [world.html.get()], "label": "Receipts to file"}),
    );
    world.call(
        "move_messages",
        json!({"messages": [world.html.get()], "mailbox": world.trash.get()}),
    );
    let lines = world.audit_lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0]["tool"], "archive_messages");
    assert_eq!(lines[0]["messages"], json!([world.budget.get()]));
    assert_eq!(
        lines[0]["mailboxes"],
        json!([world.inbox.get(), world.archive.get()])
    );
    assert_eq!(lines[0]["outcome"], "answered");
    assert_eq!(lines[1]["tool"], "add_label");
    assert!(
        lines[1]["labels"]
            .as_array()
            .is_some_and(|labels| labels.len() == 1),
        "{}",
        lines[1]
    );
    assert_eq!(lines[2]["outcome"], "refused");
    let text = std::fs::read_to_string(&world.audit).expect("the log");
    assert!(!text.contains("Receipts to file"), "{text}");
}

#[test]
fn an_inbox_only_share_cannot_organize_without_actions_either() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "archive_messages",
        json!({"messages": [world.budget.get()]}),
    );
    assert!(result.is_error(), "{}", result.0);
}
