//! Off by default, opt-in per account and per folder, revoked on the next
//! call (ADR 0010 Q6).

use serde_json::{Value, json};

use crate::harness::{ADA, COMMON, EVERY_FOLDER, INBOX_ONLY, OTHER, World};

/// The message ids a search for `query` returned.
fn found(world: &mut World, query: &str) -> Vec<i64> {
    let result = world.call("search_mail", json!({"query": query, "limit": 50}));
    assert!(!result.is_error(), "{}", result.0);
    result.mail()["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|row| row["message"].as_i64().expect("an id"))
        .collect()
}

fn addresses(world: &mut World) -> Vec<String> {
    let result = world.call("list_accounts", json!({}));
    assert!(!result.is_error(), "{}", result.0);
    result.mail()["accounts"]
        .as_array()
        .map(|accounts| {
            accounts
                .iter()
                .map(|account| account["address"].as_str().expect("an address").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn with_nothing_shared_there_are_no_accounts_and_the_store_is_never_opened() {
    let mut world = World::new("");
    assert_eq!(addresses(&mut world), Vec::<String>::new());
    let searched = world.call("search_mail", json!({"query": COMMON}));
    assert!(searched.is_error(), "{}", searched.0);
    let read = world.call("read_message", json!({"message": world.budget.get()}));
    assert!(read.is_error(), "{}", read.0);
    assert_eq!(world.leases.get(), 0, "nothing shared, nothing to open");
}

#[test]
fn an_account_nobody_shared_is_invisible_to_every_tool() {
    let mut world = World::new(INBOX_ONLY);
    assert_eq!(addresses(&mut world), vec![ADA.to_owned()]);
    assert!(!found(&mut world, COMMON).contains(&world.elsewhere.get()));

    let listed = world.call("list_mailboxes", json!({"account": world.other.get()}));
    assert!(listed.is_error());
    let read = world.call("read_message", json!({"message": world.elsewhere.get()}));
    let missing = world.call("read_message", json!({"message": 987_654}));
    assert!(read.is_error() && missing.is_error());
    // Refused in the same words as a message that does not exist: the
    // answer does not say there is something there.
    assert_eq!(read.notice(), missing.notice());
    assert!(!read.all_text().contains(OTHER));
    assert!(!read.all_text().contains("Private matter"));
}

#[test]
fn a_folder_nobody_shared_is_invisible() {
    let mut world = World::new(INBOX_ONLY);
    let listed = world.call("list_mailboxes", json!({"account": world.ada.get()}));
    let paths: Vec<Value> = listed.mail()["mailboxes"]
        .as_array()
        .expect("mailboxes")
        .iter()
        .map(|mailbox| mailbox["path"].clone())
        .collect();
    assert_eq!(paths, vec![json!("INBOX")]);

    let hits = found(&mut world, COMMON);
    assert!(hits.contains(&world.budget.get()), "{hits:?}");
    assert!(!hits.contains(&world.archived.get()), "{hits:?}");

    let read = world.call("read_message", json!({"message": world.archived.get()}));
    assert!(read.is_error());
    assert!(!read.all_text().contains("Quarterly"));
}

#[test]
fn sharing_every_folder_shares_the_archive_too() {
    let mut world = World::new(EVERY_FOLDER);
    let hits = found(&mut world, COMMON);
    assert!(hits.contains(&world.archived.get()), "{hits:?}");
    assert!(!hits.contains(&world.elsewhere.get()), "{hits:?}");
}

#[test]
fn taking_an_account_out_of_the_file_takes_effect_on_the_next_call() {
    let mut world = World::new(INBOX_ONLY);
    let read = world.call("read_message", json!({"message": world.budget.get()}));
    assert!(!read.is_error(), "{}", read.0);

    world.share("");
    let read = world.call("read_message", json!({"message": world.budget.get()}));
    assert!(read.is_error());
    assert_eq!(addresses(&mut world), Vec::<String>::new());
}

#[test]
fn a_file_that_does_not_parse_shares_nothing() {
    let mut world = World::new(INBOX_ONLY);
    world.share("[mcp.accounts.\"test@example.com\"\nmailboxes = [\"*\"]\n");
    assert_eq!(addresses(&mut world), Vec::<String>::new());
}
