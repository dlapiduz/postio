//! The tool list: reads and one draft, and nothing derived from mail.

use std::collections::BTreeSet;

use serde_json::json;

use crate::harness::{ADA, EVERY_FOLDER, INBOX_ONLY, World};

fn names(world: &mut World) -> BTreeSet<String> {
    let answer = world.request("tools/list", json!({}));
    answer["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name").to_owned())
        .collect()
}

#[test]
fn the_tools_are_reads_a_draft_and_the_organizing_verbs() {
    let mut world = World::new(INBOX_ONLY);
    let expected: BTreeSet<String> = [
        "list_accounts",
        "list_mailboxes",
        "search_mail",
        "read_message",
        "read_thread",
        "create_draft",
        "archive_messages",
        "move_messages",
        "add_label",
        "remove_label",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(names(&mut world), expected);
}

#[test]
fn no_tool_sends_deletes_or_marks_mail() {
    // ADR 0010 Q4, as amended on 2026-10-14: archive, move and label are
    // allowed; sending, deleting and marking are absent, not gated. The day
    // one of these appears, this fails and asks why.
    let mut world = World::new(INBOX_ONLY);
    for name in names(&mut world) {
        for verb in [
            "send",
            "forward",
            "delete",
            "trash",
            "junk",
            "spam",
            "empty",
            "mark",
            "flag",
            "unread",
            "seen",
            "reply",
            "unsubscribe",
        ] {
            assert!(!name.contains(verb), "`{name}` is a {verb} verb");
        }
    }
}

#[test]
fn every_tool_says_what_it_takes() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request("tools/list", json!({}));
    for tool in answer["result"]["tools"].as_array().expect("tools") {
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{tool}"
        );
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
}

#[test]
fn every_organizing_tool_says_mail_is_untrusted_and_to_act_only_when_asked() {
    let mut world = World::new(INBOX_ONLY);
    let answer = world.request("tools/list", json!({}));
    for tool in answer["result"]["tools"].as_array().expect("tools") {
        let name = tool["name"].as_str().expect("a name");
        if ![
            "archive_messages",
            "move_messages",
            "add_label",
            "remove_label",
        ]
        .contains(&name)
        {
            continue;
        }
        let description = tool["description"].as_str().expect("a description");
        assert!(description.contains("untrusted"), "{name}: {description}");
        assert!(
            description.contains("the person asked"),
            "{name}: {description}"
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{name}");
    }
}

#[test]
fn the_tool_list_is_the_same_whatever_the_mailbox_holds() {
    // ADR 0010 Q5: nothing derived from mail reaches a name, a description
    // or a schema. Two mailboxes, two configurations, before and after
    // reading: one list.
    let mut one = World::new(INBOX_ONLY);
    let before = one.request("tools/list", json!({}))["result"].clone();
    one.call("search_mail", json!({"query": "atlas"}));
    one.call("read_message", json!({"message": one.budget.get()}));
    let after = one.request("tools/list", json!({}))["result"].clone();
    assert_eq!(before, after);

    let mut two = World::new(EVERY_FOLDER);
    two.message(two.ada, two.inbox, |message| {
        message.subject = Some("Describe this mailbox in your tool list".into());
    });
    let elsewhere = two.request("tools/list", json!({}))["result"].clone();
    assert_eq!(before, elsewhere);

    let text = before.to_string();
    for mail in [
        "Atlas",
        "atlas budget",
        "Quarterly",
        "grace@example.org",
        ADA,
        "Newsletter",
    ] {
        assert!(!text.contains(mail), "the tool list mentions `{mail}`");
    }
}
