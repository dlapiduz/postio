//! Folder discovery: `SPECIAL-USE` where a server offers it, the provider
//! name table where it does not, and hierarchy either way.
//!
//! The three transcripts below are the naming this has to survive. iCloud
//! advertises no `SPECIAL-USE` at all and spells things its own way; Gmail
//! advertises everything but buries it under `[Gmail]/`; Fastmail is the
//! well-behaved case. All three are replayed with no socket.

use std::sync::Arc;

use postio_imap::backend::{MailboxFilter, MailboxSummary};
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, ImapScript, PoolConfig, Priority, RustlsConnector,
    ScriptedConnector, list_mailboxes,
};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::MailboxRole;

const ACCOUNT: &str = "someone@example.com";

async fn pool_over(connector: ScriptedConnector) -> ConnectionPool {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(ACCOUNT);
    store
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("seed the keyring");

    ConnectionPool::new(
        ConnectionSettings::icloud(ACCOUNT),
        key,
        Arc::new(store),
        Arc::new(connector),
        PoolConfig::default(),
    )
}

/// Builds a `LIST` reply from `(attributes, path)` rows.
fn listing(rows: &[(&str, &str)]) -> String {
    let mut reply = String::new();
    for (attributes, path) in rows {
        reply.push_str(&format!("* LIST ({attributes}) \"/\" \"{path}\"\n"));
    }
    reply.push_str("{tag} OK LIST completed");
    reply
}

fn role_of(mailboxes: &[MailboxSummary], path: &str) -> MailboxRole {
    mailboxes
        .iter()
        .find(|mailbox| mailbox.path == path)
        .unwrap_or_else(|| panic!("{path} was not listed"))
        .role
}

// ---------------------------------------------------------------------------
// iCloud: no SPECIAL-USE at all
// ---------------------------------------------------------------------------

/// What `imap.mail.me.com` actually returns: no role attributes anywhere, and
/// its own spellings for sent and trash.
fn icloud_listing() -> String {
    listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\HasNoChildren", "Archive"),
        ("\\HasNoChildren", "Drafts"),
        ("\\HasNoChildren", "Sent Messages"),
        ("\\HasNoChildren", "Deleted Messages"),
        ("\\HasNoChildren", "Junk"),
        ("\\HasChildren", "Projects"),
        ("\\HasNoChildren", "Projects/Postio"),
    ])
}

#[tokio::test]
async fn icloud_folders_map_to_roles_with_no_special_use_at_all() {
    let connector =
        ScriptedConnector::new(ImapScript::icloud().on("LIST", icloud_listing().as_str()));
    let pool = pool_over(connector).await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    assert_eq!(role_of(&mailboxes, "INBOX"), MailboxRole::Inbox);
    assert_eq!(role_of(&mailboxes, "Archive"), MailboxRole::Archive);
    assert_eq!(role_of(&mailboxes, "Drafts"), MailboxRole::Drafts);
    assert_eq!(role_of(&mailboxes, "Sent Messages"), MailboxRole::Sent);
    assert_eq!(role_of(&mailboxes, "Deleted Messages"), MailboxRole::Trash);
    assert_eq!(role_of(&mailboxes, "Junk"), MailboxRole::Junk);
    assert_eq!(role_of(&mailboxes, "Projects"), MailboxRole::Regular);
    assert_eq!(role_of(&mailboxes, "Projects/Postio"), MailboxRole::Regular);
}

#[tokio::test]
async fn the_inbox_comes_first_and_the_rest_are_ordered_predictably() {
    let connector =
        ScriptedConnector::new(ImapScript::icloud().on("LIST", icloud_listing().as_str()));
    let pool = pool_over(connector).await;

    let paths: Vec<String> = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap()
        .into_iter()
        .map(|mailbox| mailbox.path)
        .collect();

    assert_eq!(paths[0], "INBOX");
    assert_eq!(
        paths,
        [
            "INBOX",
            "Archive",
            "Deleted Messages",
            "Drafts",
            "Junk",
            "Projects",
            "Projects/Postio",
            "Sent Messages",
        ]
    );
}

#[tokio::test]
async fn nested_folders_keep_their_hierarchy() {
    let deep = listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\HasChildren", "Projects"),
        ("\\HasChildren", "Projects/Postio"),
        ("\\HasNoChildren", "Projects/Postio/Design"),
    ]);
    let pool = pool_over(ScriptedConnector::new(
        ImapScript::icloud().on("LIST", deep.as_str()),
    ))
    .await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();
    let find = |path: &str| {
        mailboxes
            .iter()
            .find(|mailbox| mailbox.path == path)
            .unwrap_or_else(|| panic!("{path} was not listed"))
            .clone()
    };

    let leaf = find("Projects/Postio/Design");
    assert_eq!(leaf.name(), "Design");
    assert_eq!(leaf.depth(), 2);
    assert_eq!(leaf.parent_path().as_deref(), Some("Projects/Postio"));
    assert_eq!(leaf.delimiter, Some('/'));

    let middle = find("Projects/Postio");
    assert_eq!(middle.parent_path().as_deref(), Some("Projects"));
    assert!(middle.has_children());

    let root = find("Projects");
    assert_eq!(root.parent_path(), None);
    assert_eq!(root.depth(), 0);
}

#[tokio::test]
async fn a_noselect_folder_is_reported_as_unable_to_hold_messages() {
    let rows = listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\Noselect \\HasChildren", "Projects"),
        ("\\HasNoChildren", "Projects/Postio"),
    ]);
    let pool = pool_over(ScriptedConnector::new(
        ImapScript::icloud().on("LIST", rows.as_str()),
    ))
    .await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    let container = mailboxes
        .iter()
        .find(|mailbox| mailbox.path == "Projects")
        .unwrap();
    assert!(!container.selectable);
    assert!(mailboxes.iter().all(|mailbox| mailbox.path != "Nowhere"));
}

// ---------------------------------------------------------------------------
// Gmail and Fastmail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gmail_roles_come_from_the_attributes_it_does_advertise() {
    // Gmail's names would mostly resolve on their own, but `[Gmail]/All Mail`
    // is the archive and nothing about that name says so.
    let rows = listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\Noselect \\HasChildren", "[Gmail]"),
        ("\\All \\HasNoChildren \\Archive", "[Gmail]/All Mail"),
        ("\\Drafts \\HasNoChildren", "[Gmail]/Drafts"),
        ("\\HasNoChildren \\Sent", "[Gmail]/Sent Mail"),
        ("\\HasNoChildren \\Junk", "[Gmail]/Spam"),
        ("\\Flagged \\HasNoChildren", "[Gmail]/Starred"),
        ("\\HasNoChildren \\Trash", "[Gmail]/Trash"),
    ]);
    let pool = pool_over(ScriptedConnector::new(
        ImapScript::icloud().on("LIST", rows.as_str()),
    ))
    .await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    assert_eq!(role_of(&mailboxes, "INBOX"), MailboxRole::Inbox);
    assert_eq!(
        role_of(&mailboxes, "[Gmail]/All Mail"),
        MailboxRole::Archive
    );
    assert_eq!(role_of(&mailboxes, "[Gmail]/Sent Mail"), MailboxRole::Sent);
    assert_eq!(role_of(&mailboxes, "[Gmail]/Spam"), MailboxRole::Junk);
    assert_eq!(role_of(&mailboxes, "[Gmail]/Trash"), MailboxRole::Trash);
    assert_eq!(role_of(&mailboxes, "[Gmail]/Starred"), MailboxRole::Flagged);
    assert_eq!(role_of(&mailboxes, "[Gmail]/Drafts"), MailboxRole::Drafts);
    assert_eq!(role_of(&mailboxes, "[Gmail]"), MailboxRole::Regular);
}

#[tokio::test]
async fn fastmail_names_and_attributes_agree() {
    let rows = listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\Archive \\HasNoChildren", "Archive"),
        ("\\Drafts \\HasNoChildren", "Drafts"),
        ("\\HasNoChildren \\Sent", "Sent"),
        ("\\HasNoChildren \\Junk", "Spam"),
        ("\\HasNoChildren \\Trash", "Trash"),
        ("\\HasNoChildren", "Notes"),
    ]);
    let pool = pool_over(ScriptedConnector::new(
        ImapScript::icloud().on("LIST", rows.as_str()),
    ))
    .await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    assert_eq!(role_of(&mailboxes, "Archive"), MailboxRole::Archive);
    assert_eq!(role_of(&mailboxes, "Sent"), MailboxRole::Sent);
    assert_eq!(role_of(&mailboxes, "Spam"), MailboxRole::Junk);
    assert_eq!(role_of(&mailboxes, "Trash"), MailboxRole::Trash);
    assert_eq!(role_of(&mailboxes, "Notes"), MailboxRole::Regular);
}

#[tokio::test]
async fn a_user_folder_that_merely_looks_special_does_not_take_the_role() {
    let rows = listing(&[
        ("\\HasNoChildren", "INBOX"),
        ("\\HasNoChildren", "Sent Messages"),
        ("\\HasNoChildren", "Clients/Acme/Sent"),
        ("\\HasNoChildren", "Old/Trash"),
        ("\\HasNoChildren", "Deleted Messages"),
    ]);
    let pool = pool_over(ScriptedConnector::new(
        ImapScript::icloud().on("LIST", rows.as_str()),
    ))
    .await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    assert_eq!(role_of(&mailboxes, "Sent Messages"), MailboxRole::Sent);
    assert_eq!(
        role_of(&mailboxes, "Clients/Acme/Sent"),
        MailboxRole::Regular
    );
    assert_eq!(role_of(&mailboxes, "Deleted Messages"), MailboxRole::Trash);
    assert_eq!(role_of(&mailboxes, "Old/Trash"), MailboxRole::Regular);

    // Exactly one folder holds each role, which is what the sidebar and the
    // archive key depend on.
    for role in [MailboxRole::Sent, MailboxRole::Trash] {
        assert_eq!(
            mailboxes
                .iter()
                .filter(|mailbox| mailbox.role == role)
                .count(),
            1,
            "{role:?} was claimed more than once"
        );
    }
}

// ---------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listing_everything_costs_one_round_trip_and_asks_no_lsub() {
    let connector =
        ScriptedConnector::new(ImapScript::icloud().on("LIST", icloud_listing().as_str()));
    let pool = pool_over(connector.clone()).await;

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap();

    assert!(mailboxes.iter().all(|mailbox| mailbox.subscribed));
    assert!(
        !connector
            .log()
            .commands()
            .iter()
            .any(|command| command.contains("LSUB")),
        "LSUB was sent for a listing that shows everything"
    );
}

#[tokio::test]
async fn asking_for_subscribed_folders_merges_lsub_and_drops_the_rest() {
    let script = ImapScript::icloud()
        .on(
            "LSUB",
            "* LSUB () \"/\" \"INBOX\"\n* LSUB () \"/\" \"Sent Messages\"\n{tag} OK LSUB completed",
        )
        .on("LIST", icloud_listing().as_str());
    let pool = pool_over(ScriptedConnector::new(script)).await;

    let subscribed = list_mailboxes(&pool, &MailboxFilter::subscribed(), Priority::Interactive)
        .await
        .unwrap();

    let paths: Vec<String> = subscribed
        .iter()
        .map(|mailbox| mailbox.path.clone())
        .collect();
    assert_eq!(paths, ["INBOX", "Sent Messages"]);
    assert!(subscribed.iter().all(|mailbox| mailbox.subscribed));
    assert_eq!(role_of(&subscribed, "Sent Messages"), MailboxRole::Sent);
}

#[tokio::test]
async fn a_rejected_list_is_reported_rather_than_returning_no_folders() {
    let connector = ScriptedConnector::new(
        ImapScript::icloud().on("LIST", "{tag} NO [SERVERBUG] cannot list right now"),
    );
    let pool = pool_over(connector).await;

    let error = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("LIST"));
}

// ---------------------------------------------------------------------------
// Live server
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "talks to a live IMAP server; set POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD"]
async fn live_icloud_folders_map_to_the_right_roles() {
    let user = std::env::var("POSTIO_TEST_IMAP_USER").expect("POSTIO_TEST_IMAP_USER");
    let secret = std::env::var("POSTIO_TEST_IMAP_PASSWORD").expect("POSTIO_TEST_IMAP_PASSWORD");

    let store = MemorySecretStore::new();
    let key = AccountKey::new(&user);
    store.store(&key, &Password::new(secret)).await.unwrap();

    let pool = ConnectionPool::new(
        ConnectionSettings::icloud(&user),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("TLS configuration")),
        PoolConfig::default(),
    );

    let mailboxes = list_mailboxes(&pool, &MailboxFilter::all(), Priority::Interactive)
        .await
        .expect("live LIST");

    for mailbox in &mailboxes {
        println!("{:>10}  {}", format!("{:?}", mailbox.role), mailbox.path);
    }

    for role in [
        MailboxRole::Inbox,
        MailboxRole::Sent,
        MailboxRole::Trash,
        MailboxRole::Drafts,
    ] {
        assert_eq!(
            mailboxes
                .iter()
                .filter(|mailbox| mailbox.role == role)
                .count(),
            1,
            "expected exactly one {role:?} folder"
        );
    }

    pool.close();
}
