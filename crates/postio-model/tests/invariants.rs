//! Acceptance: documented invariants for ids and flags are enforced in code.

use chrono::{TimeZone, Utc};
use postio_model::*;

// ---------------------------------------------------------------- id invariants

#[test]
fn unassigned_is_the_only_unassigned_value() {
    assert_eq!(MessageId::UNASSIGNED.get(), 0);
    assert!(!MessageId::UNASSIGNED.is_assigned());
    assert!(MessageId::new(1).is_assigned());
    assert!(!MessageId::new(0).is_assigned());
}

#[test]
fn a_new_entity_starts_unassigned() {
    let message = Message::new(
        AccountId::new(1),
        MailboxId::new(2),
        Utc.timestamp_opt(0, 0).unwrap(),
    );
    assert!(!message.id.is_assigned());
    assert!(!message.is_persisted());
}

#[test]
fn ids_of_different_entities_are_distinct_types() {
    // Compile-time property: this is a type-level assertion, not a runtime one.
    fn takes_message_id(_: MessageId) {}
    takes_message_id(MessageId::new(1));
    assert_eq!(AccountId::from(3i64).get(), 3);
    assert_eq!(i64::from(AccountId::new(3)), 3);
    assert_eq!(AccountId::new(3).to_string(), "3");
}

#[test]
fn rfc_message_id_is_normalized_but_keeps_angle_brackets() {
    let id = RfcMessageId::new("  <A@Example.com>  ");
    assert_eq!(id.as_str(), "<A@Example.com>");
    // Comparison for threading is case-insensitive on the domain-ish whole value.
    assert_eq!(id, RfcMessageId::new("<a@example.COM>"));
    assert_eq!(RfcMessageId::new("a@b").as_str(), "<a@b>");
}

// -------------------------------------------------------------- flag invariants

#[test]
fn flags_have_exactly_one_canonical_representation() {
    assert_eq!(Flag::parse("\\Seen"), Flag::Seen);
    assert_eq!(Flag::parse("\\SEEN"), Flag::Seen);
    assert_eq!(Flag::parse("\\Deleted"), Flag::Deleted);
    assert_eq!(Flag::parse("$Forwarded"), Flag::Forwarded);
    assert_eq!(Flag::parse("$forwarded"), Flag::Forwarded);
    assert_eq!(Flag::parse("$Junk"), Flag::Junk);
    assert_eq!(Flag::parse("$NotJunk"), Flag::NotJunk);
    assert_eq!(Flag::parse("Work"), Flag::Keyword("Work".into()));

    assert_eq!(Flag::Seen.as_str(), "\\Seen");
    assert_eq!(Flag::Forwarded.as_str(), "$Forwarded");
    assert_eq!(Flag::Keyword("Work".into()).as_str(), "Work");
}

#[test]
fn flag_set_is_a_set_and_order_independent() {
    let a = FlagSet::from_iter([Flag::Seen, Flag::Flagged, Flag::Seen]);
    let b = FlagSet::from_iter([Flag::Flagged, Flag::Seen]);
    assert_eq!(a, b);
    assert_eq!(a.len(), 2);
}

#[test]
fn flag_set_accessors_match_the_flags_present() {
    let mut flags = FlagSet::default();
    assert!(!flags.is_seen());
    assert!(flags.is_unread());

    assert!(flags.insert(Flag::Seen));
    assert!(!flags.insert(Flag::Seen));
    assert!(flags.is_seen());
    assert!(!flags.is_unread());

    flags.insert(Flag::Flagged);
    flags.insert(Flag::Answered);
    flags.insert(Flag::Draft);
    flags.insert(Flag::Deleted);
    assert!(flags.is_flagged());
    assert!(flags.is_answered());
    assert!(flags.is_draft());
    assert!(flags.is_deleted());
    assert!(flags.contains(&Flag::Seen));

    assert!(flags.remove(&Flag::Seen));
    assert!(!flags.remove(&Flag::Seen));
    assert!(flags.is_unread());
}

#[test]
fn recent_is_transient_and_never_persisted() {
    assert!(Flag::Recent.is_transient());
    assert!(!Flag::Seen.is_transient());

    let flags = FlagSet::from_iter([Flag::Seen, Flag::Recent]);
    let persisted = flags.persistable();
    assert!(persisted.contains(&Flag::Seen));
    assert!(!persisted.contains(&Flag::Recent));
}

#[test]
fn keywords_are_case_insensitive_so_a_set_holds_one_of_them() {
    let flags = FlagSet::from_iter([Flag::parse("Work"), Flag::parse("work")]);
    assert_eq!(flags.len(), 1);
}

// ------------------------------------------------------------ mailbox roles

#[test]
fn special_use_attributes_map_to_roles() {
    assert_eq!(
        MailboxRole::from_special_use("\\Inbox"),
        Some(MailboxRole::Inbox)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Sent"),
        Some(MailboxRole::Sent)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Drafts"),
        Some(MailboxRole::Drafts)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Trash"),
        Some(MailboxRole::Trash)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Junk"),
        Some(MailboxRole::Junk)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Archive"),
        Some(MailboxRole::Archive)
    );
    assert_eq!(
        MailboxRole::from_special_use("\\Flagged"),
        Some(MailboxRole::Flagged)
    );
    assert_eq!(MailboxRole::from_special_use("\\All"), None);
    assert_eq!(MailboxRole::from_special_use("\\Noselect"), None);
}

#[test]
fn icloud_folder_names_are_recognised_by_name() {
    // iCloud does not advertise SPECIAL-USE for these; the names are non-standard.
    assert_eq!(
        MailboxRole::guess_from_name("Sent Messages"),
        MailboxRole::Sent
    );
    assert_eq!(
        MailboxRole::guess_from_name("Deleted Messages"),
        MailboxRole::Trash
    );
    assert_eq!(MailboxRole::guess_from_name("Junk"), MailboxRole::Junk);
    assert_eq!(
        MailboxRole::guess_from_name("Archive"),
        MailboxRole::Archive
    );
    assert_eq!(MailboxRole::guess_from_name("Drafts"), MailboxRole::Drafts);
    assert_eq!(MailboxRole::guess_from_name("INBOX"), MailboxRole::Inbox);
    assert_eq!(MailboxRole::guess_from_name("inbox"), MailboxRole::Inbox);
    assert_eq!(
        MailboxRole::guess_from_name("Projects/Postio"),
        MailboxRole::Regular
    );
    // Common alternatives from other providers.
    assert_eq!(
        MailboxRole::guess_from_name("Sent Items"),
        MailboxRole::Sent
    );
    assert_eq!(MailboxRole::guess_from_name("Bin"), MailboxRole::Trash);
    assert_eq!(MailboxRole::guess_from_name("Spam"), MailboxRole::Junk);
    assert_eq!(
        MailboxRole::guess_from_name("All Mail"),
        MailboxRole::Regular
    );
}

#[test]
fn resolving_a_role_prefers_the_server_attribute_over_the_name() {
    assert_eq!(
        MailboxRole::resolve(["\\Archive"], "Sent Messages"),
        MailboxRole::Archive
    );
    assert_eq!(
        MailboxRole::resolve(Vec::<String>::new(), "Sent Messages"),
        MailboxRole::Sent
    );
    assert_eq!(
        MailboxRole::resolve(["\\HasNoChildren"], "Deleted Messages"),
        MailboxRole::Trash
    );
}

#[test]
fn a_role_knows_whether_it_is_special() {
    assert!(MailboxRole::Inbox.is_special());
    assert!(!MailboxRole::Regular.is_special());
    assert_eq!(MailboxRole::default(), MailboxRole::Regular);
}

// ------------------------------------------------------------------- addresses

#[test]
fn addresses_display_as_rfc5322_and_compare_case_insensitively() {
    let a = EmailAddress::new(Some("Alice"), "Alice@Example.com");
    assert_eq!(a.to_string(), "Alice <Alice@Example.com>");
    assert_eq!(EmailAddress::new(None::<String>, "a@b").to_string(), "a@b");

    assert_eq!(a.normalized(), "alice@example.com");
    assert_eq!(a.local_part(), Some("Alice"));
    assert_eq!(a.domain(), Some("Example.com"));
    assert!(a.same_address(&EmailAddress::new(Some("Other"), "alice@EXAMPLE.com")));
    assert_eq!(a.display(), "Alice");
    assert_eq!(EmailAddress::new(None::<String>, "a@b").display(), "a@b");
}

// --------------------------------------------------------------------- headers

#[test]
fn headers_preserve_order_and_duplicates_but_look_up_case_insensitively() {
    let headers = Headers::from_iter([
        ("Received", "from one"),
        ("Received", "from two"),
        ("X-Mailer", "postio"),
    ]);
    assert_eq!(headers.len(), 3);
    assert_eq!(headers.get("received"), Some("from one"));
    assert_eq!(headers.get("X-MAILER"), Some("postio"));
    assert_eq!(headers.get("Missing"), None);
    assert_eq!(headers.get_all("RECEIVED"), vec!["from one", "from two"]);
    assert_eq!(headers.iter().count(), 3);
}

// -------------------------------------------------------- message + threading

#[test]
fn message_exposes_what_jwz_threading_needs() {
    let mut message = Message::new(
        AccountId::new(1),
        MailboxId::new(2),
        Utc.timestamp_opt(10, 0).unwrap(),
    );
    message.rfc_message_id = Some(RfcMessageId::new("<c@x>"));
    message.in_reply_to = Some(RfcMessageId::new("<b@x>"));
    message.references = vec![RfcMessageId::new("<a@x>"), RfcMessageId::new("<b@x>")];

    // References, with In-Reply-To appended when it is not already the last link.
    let chain: Vec<_> = message.reference_chain().cloned().collect();
    assert_eq!(
        chain,
        vec![RfcMessageId::new("<a@x>"), RfcMessageId::new("<b@x>")]
    );

    message.in_reply_to = Some(RfcMessageId::new("<z@x>"));
    let chain: Vec<_> = message.reference_chain().cloned().collect();
    assert_eq!(chain.last(), Some(&RfcMessageId::new("<z@x>")));
    assert_eq!(chain.len(), 3);
}

#[test]
fn subject_normalization_strips_reply_and_forward_prefixes() {
    assert_eq!(normalize_subject("Re: Contract"), "contract");
    assert_eq!(normalize_subject("RE: Re: FWD: Contract"), "contract");
    assert_eq!(normalize_subject("Fwd:  Contract  "), "contract");
    assert_eq!(normalize_subject("[list] Re: Contract"), "[list] contract");
    assert_eq!(normalize_subject("Re[2]: Contract"), "contract");
    assert_eq!(normalize_subject(""), "");
}

#[test]
fn message_convenience_accessors() {
    let mut message = Message::new(
        AccountId::new(1),
        MailboxId::new(2),
        Utc.timestamp_opt(10, 0).unwrap(),
    );
    assert!(!message.has_attachments());
    assert_eq!(message.best_date(), Utc.timestamp_opt(10, 0).unwrap());

    message.date = Some(Utc.timestamp_opt(5, 0).unwrap());
    assert_eq!(message.best_date(), Utc.timestamp_opt(5, 0).unwrap());

    message.attachments.push(Attachment {
        id: AttachmentId::UNASSIGNED,
        message_id: MessageId::UNASSIGNED,
        filename: Some("a.pdf".into()),
        mime_type: "application/pdf".into(),
        size: 1,
        content_id: None,
        disposition: Disposition::Attachment,
        part_id: None,
        blob_id: None,
    });
    assert!(message.has_attachments());
    assert!(!message.attachments[0].is_downloaded());
    assert!(!message.attachments[0].is_inline());

    message.subject = Some("Re: Hello".into());
    assert_eq!(message.normalized_subject(), "hello");
}

#[test]
fn body_state_knows_when_a_body_is_available() {
    assert!(!BodyState::NotFetched.has_body());
    assert!(!BodyState::HeadersOnly.has_body());
    assert!(BodyState::Partial.has_body());
    assert!(BodyState::Full.has_body());
    assert_eq!(BodyState::default(), BodyState::NotFetched);
}

#[test]
fn local_sync_state_reports_whether_the_message_is_clean() {
    let mut sync = LocalSyncState::default();
    assert!(sync.is_clean());
    sync.flags_dirty = true;
    assert!(!sync.is_clean());
}

#[test]
fn account_finds_its_default_identity() {
    let mut account = Account::new(
        "Personal",
        EmailAddress::new(None::<String>, "ada@example.com"),
    );
    assert!(account.default_identity().is_none());

    account.identities.push(Identity {
        id: IdentityId::new(1),
        account_id: AccountId::UNASSIGNED,
        display_name: "Work".into(),
        address: EmailAddress::new(None::<String>, "work@example.com"),
        reply_to: None,
        signature: None,
        is_default: false,
    });
    // With no identity marked default, the first one wins.
    assert_eq!(account.default_identity().unwrap().display_name, "Work");

    account.identities.push(Identity {
        id: IdentityId::new(2),
        account_id: AccountId::UNASSIGNED,
        display_name: "Home".into(),
        address: EmailAddress::new(None::<String>, "home@example.com"),
        reply_to: None,
        signature: None,
        is_default: true,
    });
    assert_eq!(account.default_identity().unwrap().display_name, "Home");
}

#[test]
fn thread_reports_read_state() {
    let mut thread = Thread::new(AccountId::new(1));
    assert!(thread.is_empty());
    thread.message_ids = vec![MessageId::new(1), MessageId::new(2)];
    thread.message_count = 2;
    thread.unread_count = 1;
    assert!(!thread.is_empty());
    assert!(thread.has_unread());
    thread.unread_count = 0;
    assert!(!thread.has_unread());
}

#[test]
fn draft_starts_unsent_and_empty() {
    let draft = Draft::new(AccountId::new(1));
    assert_eq!(draft.state, DraftState::Editing);
    assert_eq!(draft.kind, DraftKind::New);
    assert!(!draft.has_recipients());
    assert!(!draft.id.is_assigned());
}

#[test]
fn contact_tracks_how_often_an_address_was_seen() {
    let mut contact = Contact::new(EmailAddress::new(Some("Alice"), "alice@example.com"));
    assert_eq!(contact.times_seen, 0);
    contact.record_seen(Utc.timestamp_opt(10, 0).unwrap());
    contact.record_seen(Utc.timestamp_opt(20, 0).unwrap());
    assert_eq!(contact.times_seen, 2);
    assert_eq!(
        contact.last_seen_at,
        Some(Utc.timestamp_opt(20, 0).unwrap())
    );
    assert_eq!(contact.display_name(), "Alice");
}

#[test]
fn mailbox_new_derives_its_leaf_name_and_role_from_the_path() {
    let mailbox = Mailbox::new(AccountId::new(1), "Sent Messages", Some('/'));
    assert_eq!(mailbox.name, "Sent Messages");
    assert_eq!(mailbox.role, MailboxRole::Sent);
    assert!(!mailbox.id.is_assigned());

    let nested = Mailbox::new(AccountId::new(1), "Projects/Postio", Some('/'));
    assert_eq!(nested.name, "Postio");
    assert_eq!(nested.path, "Projects/Postio");
    assert_eq!(nested.role, MailboxRole::Regular);
}

#[test]
fn message_body_knows_when_it_is_empty() {
    let mut body = MessageBody::default();
    assert!(body.is_empty());
    body.html = Some("<p>hi</p>".into());
    assert!(!body.is_empty());
}
