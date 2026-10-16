//! Agent access in Settings (#1832): what the pane shows and what revoking
//! writes, asserted on the sentences a person reads.

use chrono::{TimeZone, Utc};
use postio_ffi::{
    Session, SessionOptions, settings_agent_sharing, settings_agent_words,
    settings_patch_revoke_agent, settings_patch_revoke_all_agents, settings_sections,
};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const SHARED: &str = "[ui]\ntheme = \"dark\"\n\n\
    [mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\"]\n\n\
    [mcp.accounts.\"grace@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"archive\"]\n";

#[test]
fn the_pane_is_in_the_nav_above_the_raw_file() {
    let sections = settings_sections();
    let keys: Vec<&str> = sections.iter().map(|s| s.key.as_str()).collect();
    let agents = keys.iter().position(|key| *key == "mcp").expect("listed");
    assert_eq!(keys.last(), Some(&""), "the raw file stays last");
    assert_eq!(agents, keys.len() - 2);
    assert_eq!(sections[agents].label, "Agents");
    assert_eq!(sections[agents].table.as_deref(), Some("[mcp]"));
}

#[test]
fn the_pane_lists_what_is_shared_and_nothing_when_nothing_is() {
    let rows = settings_agent_sharing(SHARED.to_owned()).expect("parses");
    let ada = rows
        .iter()
        .find(|r| r.address == "ada@example.com")
        .unwrap();
    assert_eq!(
        (ada.folders.as_str(), ada.abilities.as_str()),
        ("INBOX", "Read and draft only")
    );
    let grace = rows
        .iter()
        .find(|r| r.address == "grace@example.com")
        .unwrap();
    assert_eq!(
        (grace.folders.as_str(), grace.abilities.as_str()),
        ("All folders", "Also archive")
    );
    assert!(
        settings_agent_sharing(String::new())
            .expect("parses")
            .is_empty()
    );
    assert!(
        settings_agent_sharing("[mcp".to_owned()).is_none(),
        "a broken file disables the pane"
    );
}

#[test]
fn revoking_one_account_leaves_the_rest_and_the_file_alone() {
    let after =
        settings_patch_revoke_agent(SHARED.to_owned(), "ada@example.com".to_owned()).unwrap();
    let rows = settings_agent_sharing(after.clone()).unwrap();
    let addresses: Vec<&str> = rows.iter().map(|r| r.address.as_str()).collect();
    assert_eq!(addresses, ["grace@example.com"]);
    assert!(after.contains("theme = \"dark\""));
    let none = settings_patch_revoke_all_agents(after).unwrap();
    assert!(settings_agent_sharing(none.clone()).unwrap().is_empty());
    assert!(none.contains("theme = \"dark\"") && !none.contains("mcp"));
}

#[test]
fn the_pane_says_what_revoking_does_not_do() {
    let words = settings_agent_words();
    assert!(words.revoke_note.contains("next call"));
    assert!(words.nothing_shared.contains("No agent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_activity_resolves_ids_to_the_rows_they_name() {
    let database = test_support::memory().await;
    let (account, inbox, message) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Shop"), "shop@example.org")];
        message.subject = Some("Your receipt".to_owned());
        let id = repository.create(&mut message).await.expect("a message");
        (account, inbox, id)
    };
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    let (a, i, m) = (account.id.get(), inbox.get(), message.get());
    let log = format!(
        "{{\"at\":\"2026-10-16T11:00:00+00:00\",\"tool\":\"read_message\",\"accounts\":[{a}],\"mailboxes\":[{i}],\"messages\":[{m}],\"outcome\":\"answered\"}}\n\
         {{\"at\":\"2026-10-16T11:30:00+00:00\",\"tool\":\"archive_messages\",\"accounts\":[{a}],\"mailboxes\":[],\"messages\":[],\"outcome\":\"refused\"}}\n"
    );
    let now = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
    let activity = session.agent_activity_from(&log, now).await;

    assert_eq!(activity.summary, "2 calls \u{b7} last 30 min ago");
    let address = &account.address.address;
    assert_eq!(
        activity.calls[0].line,
        format!("Archived no messages \u{b7} {address} \u{b7} refused")
    );
    assert!(activity.calls[0].refused);
    assert_eq!(activity.calls[1].when, "1 h ago");
    assert!(
        activity.calls[1]
            .line
            .starts_with("Read 1 message \u{201c}Your receipt\u{201d} in "),
        "{}",
        activity.calls[1].line
    );
    assert!(!activity.calls[1].refused);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_no_log_has_no_activity() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    let activity = session.agent_activity().await;
    assert!(activity.calls.is_empty());
    assert_eq!(activity.summary, "No agent has used Postio yet.");
    session.shutdown();
}
