use serde_json::{Value, json};

use crate::harness::{COMMON, EVERY_FOLDER, INBOX_ONLY, ORGANIZE, World};

const LABEL_INBOX: &str =
    "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"INBOX\"]\nactions = [\"label\"]\n";

fn more_in_the_inbox(world: &World, count: usize) {
    for nth in 0..count {
        world.message(world.ada, world.inbox, |message| {
            message.subject = Some(format!("Notice {nth} {COMMON}"));
            message.from = vec![postio_model::EmailAddress::new(
                None::<String>,
                format!("sender{nth}@example.net"),
            )];
        });
    }
}

#[test]
fn search_mail_counts_only_shared_folders_and_pages_to_the_end() {
    let mut world = World::new(INBOX_ONLY);
    more_in_the_inbox(&world, 3);
    let mut seen = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut arguments =
            json!({"query": COMMON, "account": world.ada.get(), "order": "newest", "limit": 2});
        if !cursor.is_null() {
            arguments["cursor"] = cursor.clone();
        }
        let result = world.call("search_mail", arguments);
        assert!(!result.is_error(), "{}", result.0);
        assert_eq!(
            result.mail()["total"],
            json!(5),
            "the two seeded inbox messages and three more; the archive is not shared, so not counted"
        );
        for row in result.mail()["messages"].as_array().expect("rows") {
            seen.push(row["message"].as_i64().expect("an id"));
        }
        cursor = result.mail()["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5, "every page, each message once");
    assert!(!seen.contains(&world.archived.get()));
}

#[test]
fn a_cursor_needs_its_account() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call("search_mail", json!({"query": COMMON, "cursor": "1:2"}));
    assert!(result.is_error());
    assert!(result.notice().contains("account"), "{}", result.notice());
}

#[test]
fn mail_stats_counts_by_sender_in_the_fenced_block_only() {
    let mut world = World::new(EVERY_FOLDER);
    more_in_the_inbox(&world, 2);
    let result = world.call(
        "mail_stats",
        json!({"query": COMMON, "account": world.ada.get(), "group_by": "sender"}),
    );
    assert!(!result.is_error(), "{}", result.0);
    let groups = result.mail()["groups"].as_array().expect("groups").clone();
    let keys: Vec<&str> = groups
        .iter()
        .map(|g| g["key"].as_str().expect("a key"))
        .collect();
    assert!(
        keys.contains(&"sender0@example.net") && keys.contains(&"sender1@example.net"),
        "{keys:?}"
    );
    assert!(
        !result.notice().contains("@"),
        "addresses come from mail, so they stay in the fenced block: {}",
        result.notice()
    );
}

fn label_by_query(world: &mut World, extra: Value) -> crate::harness::ToolResult {
    let mut arguments = json!({"account": world.ada.get(), "query": COMMON, "label": "Sweep"});
    for (key, value) in extra.as_object().expect("an object") {
        arguments[key] = value.clone();
    }
    world.call("add_label", arguments)
}

#[test]
fn a_query_labels_every_shared_match_after_a_dry_run_and_a_rerun_finds_none() {
    let mut world = World::new(ORGANIZE);
    let preview = label_by_query(&mut world, json!({"dry_run": true}));
    assert!(!preview.is_error(), "{}", preview.0);
    assert_eq!(
        preview.mail()["would_change"],
        json!(3),
        "budget, archived and html; not the other account"
    );
    assert!(
        world.labels_of(world.budget).is_empty(),
        "a dry run changes nothing"
    );

    let done = label_by_query(&mut world, json!({"expect": 3}));
    assert!(!done.is_error(), "{}", done.0);
    for message in [world.budget, world.archived, world.html] {
        assert_eq!(world.labels_of(message), vec!["Sweep".to_owned()]);
    }
    assert!(world.labels_of(world.elsewhere).is_empty());
    assert_eq!(done.mail()["changed"], json!(3));
    assert_eq!(done.mail()["remaining"], json!(0));

    let again = label_by_query(&mut world, json!({"dry_run": true}));
    assert_eq!(
        again.mail()["would_change"],
        json!(0),
        "what is done is not done twice"
    );
}

#[test]
fn an_expect_that_no_longer_holds_refuses_and_changes_nothing() {
    let mut world = World::new(ORGANIZE);
    let result = label_by_query(&mut world, json!({"expect": 2}));
    assert!(result.is_error());
    let notice = result.notice();
    assert!(
        notice.contains('3') && notice.contains('2'),
        "both numbers, so the agent can look again: {notice}"
    );
    assert!(world.labels_of(world.budget).is_empty());
}

#[test]
fn a_query_without_a_dry_run_or_an_expect_is_refused() {
    let mut world = World::new(ORGANIZE);
    let result = label_by_query(&mut world, json!({}));
    assert!(result.is_error());
    assert!(result.notice().contains("expect"), "{}", result.notice());
    assert!(world.labels_of(world.budget).is_empty());
}

#[test]
fn a_query_reaches_only_shared_folders() {
    let mut world = World::new(LABEL_INBOX);
    let preview = label_by_query(&mut world, json!({"dry_run": true}));
    assert_eq!(
        preview.mail()["would_change"],
        json!(2),
        "the archive is not shared: {}",
        preview.0
    );
}

#[test]
fn a_query_move_leaves_out_what_is_already_in_the_folder() {
    let mut world = World::new(ORGANIZE);
    let preview = world.call(
        "move_messages",
        json!({"account": world.ada.get(), "query": COMMON, "mailbox": world.archive.get(), "dry_run": true}),
    );
    assert_eq!(preview.mail()["would_change"], json!(2), "{}", preview.0);
    let done = world.call(
        "move_messages",
        json!({"account": world.ada.get(), "query": COMMON, "mailbox": world.archive.get(), "expect": 2}),
    );
    assert!(!done.is_error(), "{}", done.0);
    assert_eq!(world.mailbox_of(world.budget), world.archive);
    assert_eq!(world.mailbox_of(world.html), world.archive);
}
