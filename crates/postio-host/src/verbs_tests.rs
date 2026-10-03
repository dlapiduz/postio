//! Postio Focus's verbs, sent through the host the way a frontend sends them
//! (specs/007-postio-focus). Asserted on what a frontend sees: what its reads
//! answer, the events it hears, and what its undo takes back.

use chrono::{DateTime, Local, TimeZone as _, Utc};
use postio_client::protocol::ClientKind;
use postio_core::{Command, Event, MessageTarget};
use postio_model::MessageId;
use postio_storage::repository::MessageRepository;

use crate::tests::World;

/// Saturday 26 September 2026 at 16:09 on this machine's clock: US5's fixed
/// clock, the one screen 11 draws its times at.
fn saturday_at_nine_past_four() -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 9, 26, 16, 9, 0)
        .single()
        .expect("a time the clocks do not skip")
}

/// When `message` comes back from its snooze, as the store holds it.
fn snoozed_until(world: &World, message: MessageId) -> Option<DateTime<Utc>> {
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        MessageRepository::new(&reader)
            .get(message)
            .await
            .expect("a read")
            .expect("the message is here")
            .snoozed_until
    })
}

/// A snooze of the world's message until `until`, as the picker sends it.
fn snooze(until: DateTime<Utc>, message: MessageId) -> Command {
    Command::Snooze {
        target: MessageTarget::Messages(vec![message]),
        until: Some(until),
    }
}

// ── Snooze until a chosen time (T092) ───────────────────────────────────────

#[test]
fn a_snooze_comes_back_at_the_preset_or_the_typed_time_it_was_given() {
    // US5 scenarios 1 and 2, at a fixed clock. Each of the snooze picker's
    // four presets, and "tue 9am" as `parse_when` reads it, is exactly when
    // the mail comes back: the picker's time reaches the store.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let now = saturday_at_nine_past_four();
    let local = |day: u32, hour: u32| {
        Local
            .with_ymd_and_hms(2026, 9, day, hour, 0, 0)
            .single()
            .expect("a time")
    };
    let presets = postio_ui::schedule::snooze_presets(now);
    assert_eq!(
        presets.map(|(_, at)| at),
        [
            local(26, 18),
            local(27, 8),
            local(28, 8),
            Local
                .with_ymd_and_hms(2026, 10, 3, 8, 0, 0)
                .single()
                .expect("a time"),
        ],
        "screen 11's four times"
    );
    let typed = postio_search::date::parse_when("tue 9am", now).expect("a time it reads");
    assert_eq!(
        typed,
        local(29, 9),
        "the coming Tuesday at 09:00, local time"
    );

    let chosen = presets.iter().map(|(_, at)| *at).chain([typed]);
    for until in chosen {
        let until = until.with_timezone(&Utc);
        world.send(&client, snooze(until, world.message()));
        world.hear(&events, |event| {
            matches!(event, Event::ActionCompleted { description, .. } if description.starts_with("Snoozed"))
        });
        assert_eq!(
            snoozed_until(&world, world.message()),
            Some(until),
            "snoozed until the time chosen, not a default"
        );
    }
}

#[test]
fn a_snooze_hides_the_mail_and_undoing_its_unsnooze_puts_back_the_chosen_time() {
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let message = world.message();
    let until = (Utc::now() + chrono::TimeDelta::days(2)).with_nanosecond_zeroed();

    world.send(&client, snooze(until, message));
    world.hear(&events, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
    assert_eq!(world.inbox_rows(&client), 0, "it left the inbox");

    world.send(
        &client,
        Command::Unsnooze {
            target: MessageTarget::Messages(vec![message]),
        },
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { description, .. } if description.starts_with("Unsnoozed"))
    });
    assert_eq!(snoozed_until(&world, message), None);

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(
        snoozed_until(&world, message),
        Some(until),
        "undoing the unsnooze snoozes it to the time it had, not a default"
    );
}

/// A time as the store keeps it: to the millisecond.
trait Millis {
    fn with_nanosecond_zeroed(self) -> Self;
}

impl Millis for DateTime<Utc> {
    fn with_nanosecond_zeroed(self) -> Self {
        DateTime::from_timestamp_millis(self.timestamp_millis()).expect("a time")
    }
}

// ── Remind if no reply (T094) ───────────────────────────────────────────────

/// A message filed in the world's inbox, from `from`, threaded as sync
/// threads it: `rfc` its `Message-ID`, `parent` the one it answers.
fn letter(
    world: &World,
    from: &str,
    subject: &str,
    rfc: &str,
    parent: Option<&str>,
    at: DateTime<Utc>,
) -> (MessageId, postio_model::ThreadId) {
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let mut message = postio_model::Message::new(world.account, world.inbox(), at);
        message.subject = Some(subject.to_owned());
        message.from = vec![postio_model::EmailAddress::new(None::<&str>, from)];
        message.to = vec![postio_model::EmailAddress::new(
            Some("Test User"),
            "test@example.com",
        )];
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(rfc));
        if let Some(parent) = parent {
            message.in_reply_to = Some(postio_model::RfcMessageId::new(parent));
            message.references = vec![postio_model::RfcMessageId::new(parent)];
        }
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        let thread =
            postio_storage::repository::ThreadingRepository::new(&connection, world.account)
                .thread(&message)
                .await
                .expect("threaded")
                .thread_id;
        (id, thread)
    })
}

/// Wait until Focus's catch-up has sorted everything up to `message`: its
/// mark, which it moves once it has, is there.
fn sorted_through(world: &World, message: MessageId) {
    crate::tests::eventually(world, || {
        world.rt.block_on(async {
            let reader = world.database().read().await.expect("a reader");
            postio_storage::repository::SettingsRepository::new(&reader)
                .get(crate::focus::FILED_THROUGH)
                .await
                .expect("a read")
                .filter(|mark| *mark == message.get().to_string())
                .map(|_| ())
        })
    });
}

/// The reminder standing on `thread`, as the store holds it.
fn reminder_on(
    world: &World,
    thread: postio_model::ThreadId,
) -> Option<postio_storage::repository::Reminder> {
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        postio_storage::repository::ReminderRepository::new(&reader)
            .standing(thread)
            .await
            .expect("a read")
    })
}

/// When a reminder was cancelled, and when it fired.
type Answered = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

/// Every reminder the store holds on `thread`, standing or not: when each
/// was cancelled and fired.
fn reminders_on(world: &World, thread: postio_model::ThreadId) -> Vec<Answered> {
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        postio_storage::sql::all(
            &reader,
            "SELECT cancelled_at, fired_at FROM reminders WHERE thread_id = ?1 ORDER BY id",
            [thread.get()],
            |row| {
                use postio_storage::sql::RowExt as _;
                let time = |index: usize| -> postio_storage::Result<Option<DateTime<Utc>>> {
                    Ok(row
                        .col::<Option<i64>>(index)?
                        .map(postio_storage::repository::from_millis))
                };
                Ok((time(0)?, time(1)?))
            },
        )
        .await
        .expect("a read")
    })
}

/// US5's reminder, set from the picker at the fixed clock: "In 2 working
/// days", Tuesday 29 September at 09:00.
fn tuesday_at_nine() -> DateTime<Utc> {
    postio_ui::schedule::remind_presets(saturday_at_nine_past_four())[1]
        .1
        .with_timezone(&Utc)
}

/// A reminder on `message`'s conversation, due at `at`, as the picker sends it.
fn remind(message: MessageId, at: Option<DateTime<Utc>>) -> Command {
    Command::RemindIfNoReply {
        target: MessageTarget::Messages(vec![message]),
        at,
    }
}

/// Fire the reminders due as of `now`, as Focus's due timer does each tick.
fn fire_at(world: &World, now: DateTime<Utc>) -> usize {
    world
        .rt
        .block_on(crate::focus::fire_reminders(world.database(), now))
        .expect("the timer's pass")
}

#[test]
fn a_reminder_is_set_on_the_conversation_and_undo_takes_it_back() {
    // FR-045: a reminder is undoable when set, and when cleared.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let set_on = saturday_at_nine_past_four().with_timezone(&Utc);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        set_on,
    );
    assert_eq!(
        tuesday_at_nine(),
        Local
            .with_ymd_and_hms(2026, 9, 29, 9, 0, 0)
            .single()
            .expect("a time")
            .with_timezone(&Utc),
        "the picker's In 2 working days, on a Saturday"
    );

    world.send(&client, remind(budget, Some(tuesday_at_nine())));
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Reminder set on 1 conversation".to_owned(),
            undoable: true,
        }
    );
    let reminder = reminder_on(&world, thread).expect("a reminder on the conversation");
    assert_eq!(reminder.due_at, tuesday_at_nine());
    assert_eq!(reminder.anchor, budget);
    assert_eq!(reminder.fired_at, None);

    world.send(&client, remind(budget, None));
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { description, .. } if description == "Reminder cleared on 1 conversation")
    });
    assert_eq!(reminder_on(&world, thread), None, "cleared");

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(
        reminder_on(&world, thread).map(|reminder| reminder.due_at),
        Some(tuesday_at_nine()),
        "undoing the clear puts the reminder back at its time"
    );

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(
        reminder_on(&world, thread),
        None,
        "and undoing the setting takes it away"
    );
}

#[test]
fn a_reminder_nobody_answered_fires_when_its_time_passes() {
    // US5 scenario 4, at the fixed clock: no reply by Tuesday 09:00, and the
    // conversation surfaces then -- not before.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let set_on = saturday_at_nine_past_four().with_timezone(&Utc);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        set_on,
    );
    world.send(&client, remind(budget, Some(tuesday_at_nine())));
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });

    let monday = tuesday_at_nine() - chrono::TimeDelta::hours(20);
    assert_eq!(fire_at(&world, monday), 0, "not before it is due");
    assert_eq!(
        reminder_on(&world, thread).and_then(|reminder| reminder.fired_at),
        None
    );

    assert_eq!(fire_at(&world, tuesday_at_nine()), 1);
    let fired = reminder_on(&world, thread).expect("it still stands");
    assert_eq!(fired.fired_at, Some(tuesday_at_nine()));
    assert!(fired.is_surfaced());
    assert_eq!(fire_at(&world, tuesday_at_nine()), 0, "and fires once");
}

#[test]
fn a_reminder_that_came_due_while_focus_was_closed_fires_when_it_opens() {
    // US5 scenario 4's second half and FR-045: Focus was not running at the
    // due time, and the reminder surfaces when Focus next opens. The world
    // has no account syncing and nothing reaches a network: it is offline.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(3),
    );
    world.send(
        &client,
        remind(budget, Some(Utc::now() - chrono::TimeDelta::hours(1))),
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    assert_eq!(
        reminder_on(&world, thread).and_then(|reminder| reminder.fired_at),
        None,
        "nothing fires it while Focus mode is off"
    );

    world.host().enable_focus(crate::FocusSetup::default());

    let fired = crate::tests::eventually(&world, || {
        reminder_on(&world, thread).filter(|reminder| reminder.fired_at.is_some())
    });
    assert!(fired.is_surfaced());
}

/// A reminder due a day from now: ahead of Focus's own timer for the
/// whole test, so only the filing pass can have answered it.
fn tomorrow() -> DateTime<Utc> {
    (Utc::now() + chrono::TimeDelta::days(1)).with_nanosecond_zeroed()
}

#[test]
fn a_reply_from_somebody_else_cancels_the_reminder() {
    // US5 scenario 3: a reminder, and another participant's reply before
    // its time. Focus files the reply -- here as the mail another app took
    // in while Focus was closed, sorted when it opens -- and the reply
    // cancels the reminder, so nothing returns when its time passes.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(2),
    );
    let due = tomorrow();
    world.send(&client, remind(budget, Some(due)));
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    crate::tests::focus_ran_before(&world);

    let (_, replied_in) = letter(
        &world,
        "tove@example.org",
        "Re: Atlas Q3 budget",
        "<atlas-reply@example.org>",
        Some("<atlas@example.org>"),
        Utc::now() - chrono::TimeDelta::hours(1),
    );
    assert_eq!(replied_in, thread, "the reply joined the conversation");

    world.host().enable_focus(crate::FocusSetup::default());

    let (_, fired) = crate::tests::eventually(&world, || {
        reminders_on(&world, thread)
            .into_iter()
            .find(|(cancelled, _)| cancelled.is_some())
    });
    assert_eq!(fired, None, "cancelled before it fired");
    assert_eq!(reminder_on(&world, thread), None, "it no longer stands");
    assert_eq!(fire_at(&world, due), 0, "nothing returns");
}

#[test]
fn the_person_s_own_message_does_not_cancel_their_reminder() {
    // FR-044: a reply from anyone *but the user* cancels it. Their own
    // follow-up, in the same conversation, leaves it waiting.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(2),
    );
    let due = tomorrow();
    world.send(&client, remind(budget, Some(due)));
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    crate::tests::focus_ran_before(&world);
    let (nudge, _) = letter(
        &world,
        "test@example.com",
        "Re: Atlas Q3 budget",
        "<nudge@example.com>",
        Some("<atlas@example.org>"),
        Utc::now() - chrono::TimeDelta::hours(1),
    );

    world.host().enable_focus(crate::FocusSetup::default());
    sorted_through(&world, nudge);

    assert!(
        reminder_on(&world, thread).is_some(),
        "still waiting on somebody else"
    );
    assert_eq!(fire_at(&world, due), 1, "and it fires when its time comes");
}

#[test]
fn the_person_s_own_reply_settles_a_surfaced_reminder() {
    // T095: the conversation came back marked "No reply since"; the person
    // then wrote in it themselves -- their reply is filed in Sent, which
    // the filing pass does not read -- and the surfaced row stops
    // standing, as a reply from anyone else's does.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(3),
    );
    let due = Utc::now() - chrono::TimeDelta::hours(2);
    world.send(&client, remind(budget, Some(due)));
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    assert_eq!(
        fire_at(&world, due + chrono::TimeDelta::minutes(1)),
        1,
        "it fired"
    );
    // The world's account as one that sends: its identity and its Sent.
    sending(&world);
    let sent = crate::tests::folder(&world, postio_model::MailboxRole::Sent);
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let mut message = postio_model::Message::new(
            world.account,
            sent,
            Utc::now() - chrono::TimeDelta::minutes(30),
        );
        message.subject = Some("Re: Atlas Q3 budget".to_owned());
        message.from = vec![postio_model::EmailAddress::new(
            Some("Test User"),
            "test@example.com",
        )];
        message.rfc_message_id = Some(postio_model::RfcMessageId::new("<nudge@example.com>"));
        message.in_reply_to = Some(postio_model::RfcMessageId::new("<atlas@example.org>"));
        message.references = vec![postio_model::RfcMessageId::new("<atlas@example.org>")];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("the reply");
        let threaded =
            postio_storage::repository::ThreadingRepository::new(&connection, world.account)
                .thread(&message)
                .await
                .expect("threaded");
        assert_eq!(
            threaded.thread_id, thread,
            "the reply joined the conversation"
        );
    });

    let settled = world
        .rt
        .block_on(crate::focus::settle_replied(world.database(), Utc::now()))
        .expect("the timer's pass");
    assert_eq!(settled, 1, "one surfaced reminder settled");
    assert_eq!(reminder_on(&world, thread), None, "it no longer stands");
}

// ── Answering an invitation (T112) ──────────────────────────────────────────

/// The world's account as one that sends: an identity at its address, the
/// attendee its invitations name, and a Sent folder to file the reply in.
fn sending(world: &World) {
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .get(world.account)
            .await
            .expect("a read")
            .expect("the account");
        let mut identity = postio_model::Identity::new(account.id, account.address.clone());
        identity.display_name = "Test User".to_owned();
        identity.is_default = true;
        postio_storage::repository::IdentityRepository::new(&connection)
            .create(&mut identity)
            .await
            .expect("an identity");
        postio_storage::test_support::mailbox(&connection, &account, "Sent").await;
    });
}

/// An invitation from Ines to the world's account, an hour old, with its
/// marker as the body stage writes it: open, unanswered.
fn invited(world: &World) -> MessageId {
    let start = crate::tests::ahead();
    let request = crate::tests::invitation_mail(
        world,
        chrono::TimeDelta::hours(1),
        &crate::tests::calendar(
            "REQUEST",
            "review@example.org",
            0,
            Utc::now(),
            &crate::tests::when(start, 45),
        ),
    );
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::repository::MarkerRepository::new(&connection)
            .insert(&postio_storage::repository::Marker {
                message: request,
                kind: postio_model::listing::MarkerKind::Invite,
                source: postio_storage::repository::MarkerSource::Calendar,
                span: None,
                excerpt: None,
                starts_at: Some(start),
                ends_at: Some(start + chrono::TimeDelta::minutes(45)),
                due_at: None,
                invite: Some(postio_storage::repository::InviteIdentity {
                    uid: "review@example.org".to_owned(),
                    sequence: 0,
                    stamp: None,
                }),
                invite_state: Some(postio_storage::repository::InviteState::Open),
                answer: None,
                dismissed_at: None,
            })
            .await
            .expect("its marker");
    });
    request
}

/// A mail server and an SMTP server that accepts everything, as the
/// drainer meets them.
struct Outbox {
    backend: postio_account::backend::MockBackend,
    smtp: postio_smtp::transport::ScriptedConnector,
    secrets: std::sync::Arc<postio_account::secret::MemorySecretStore>,
}

fn outbox(world: &World) -> Outbox {
    use postio_account::secret::{AccountKey, Password, SecretStore as _};
    let backend = postio_account::backend::MockBackend::builder()
        .mailbox(postio_account::backend::MockMailbox::new("Sent"))
        .build();
    let secrets = std::sync::Arc::new(postio_account::secret::MemorySecretStore::new());
    world.rt.block_on(async {
        use postio_account::backend::MailBackend as _;
        backend.connect().await.expect("connected");
        secrets
            .store(
                &AccountKey::new("test@example.com".to_owned()),
                &Password::new("password"),
            )
            .await
            .expect("the password");
    });
    // The test account submits on 587 with STARTTLS, as most do.
    let script = postio_smtp::transport::SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(
            "EHLO",
            "250-mail.example.com\r\n250-STARTTLS\r\n250 AUTH PLAIN",
        )
        .on("STARTTLS", "220 go ahead")
        .on("AUTH PLAIN", "235 authenticated")
        .on("MAIL FROM", "250 ok")
        .on("RCPT TO", "250 ok")
        .on("DATA", "354 go ahead")
        .on("QUIT", "221 bye");
    Outbox {
        backend,
        smtp: postio_smtp::transport::ScriptedConnector::new(script),
        secrets,
    }
}

impl Outbox {
    /// Drain the account's queue as of `now`, as the engine does.
    fn drain_at(&self, world: &World, now: DateTime<Utc>) -> postio_sync::DrainReport {
        let tokens = postio_account::auth::StoredPasswordSource::new(self.secrets.clone());
        world.rt.block_on(async {
            let connection = world.database().connect().await.expect("a connection");
            postio_sync::Drainer::new(&self.backend)
                .with_smtp(postio_sync::send::SmtpContext {
                    connector: &self.smtp,
                    tokens: &tokens,
                    blobs: &world.host().wiring().blobs,
                })
                .drain(&connection, world.account, now)
                .await
                .expect("a drain")
        })
    }

    /// How many messages reached the SMTP server.
    fn sent(&self) -> usize {
        self.smtp
            .log()
            .commands()
            .iter()
            .filter(|line| line.starts_with("MAIL FROM"))
            .count()
    }

    /// Everything written to the SMTP server.
    fn written(&self) -> String {
        String::from_utf8_lossy(&self.smtp.log().written).into_owned()
    }
}

/// The invitation's answer, as its marker holds it.
fn answer_on(world: &World, message: MessageId) -> Option<postio_model::listing::InviteAnswer> {
    crate::tests::marker_on(world, message).and_then(|marker| marker.answer)
}

/// Past the window an answer waits out in the outbox.
fn after_the_window() -> DateTime<Utc> {
    Utc::now() + postio_session::actions::RSVP_WINDOW + chrono::TimeDelta::seconds(1)
}

#[test]
fn an_accepted_invitation_waits_out_its_window_and_then_one_reply_leaves() {
    // US8 scenarios 2 and 3, and FR-102: `y` queues the answer through the
    // outbox, local-first. Nothing reaches the transport before the window
    // ends; after it, exactly one reply goes to the organiser.
    let world = World::new();
    sending(&world);
    let request = invited(&world);
    let (client, events) = world.frontend(ClientKind::Focus);
    let outbox = outbox(&world);

    world.send(
        &client,
        Command::AcceptInvite {
            message: Some(request),
        },
    );
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Accepted".to_owned(),
            undoable: true,
        }
    );
    assert_eq!(
        answer_on(&world, request),
        Some(postio_model::listing::InviteAnswer::Accepting),
        "accepting while the window is open"
    );

    outbox.drain_at(&world, Utc::now());
    assert_eq!(
        outbox.sent(),
        0,
        "nothing reached the transport inside the window"
    );

    let report = outbox.drain_at(&world, after_the_window());
    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(outbox.sent(), 1, "one reply left");
    let written = outbox.written();
    assert!(
        written.contains("RCPT TO:<ines@example.org>"),
        "to the organiser"
    );
    assert!(
        written.contains("Content-Type: text/calendar; method=\"REPLY\""),
        "with the calendar answer beside its words:\n{written}"
    );
    assert!(written.contains("PARTSTAT=ACCEPTED"), "saying yes");

    outbox.drain_at(&world, after_the_window());
    assert_eq!(outbox.sent(), 1, "and only one");
}

#[test]
fn an_answer_taken_back_inside_its_window_sends_nothing() {
    // US8 scenario 2: cancelled within the ten seconds, nothing is sent,
    // and the invitation is as it was.
    let world = World::new();
    sending(&world);
    let request = invited(&world);
    let (client, events) = world.frontend(ClientKind::Focus);
    let outbox = outbox(&world);

    world.send(
        &client,
        Command::DeclineInvite {
            message: Some(request),
        },
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { description, .. } if description == "Declined")
    });
    assert_eq!(
        answer_on(&world, request),
        Some(postio_model::listing::InviteAnswer::Declining)
    );

    world.send(&client, Command::Undo);
    let undone = world.hear(&events, |event| {
        matches!(
            event,
            Event::UndoPerformed { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        undone,
        Event::UndoPerformed {
            description: "Declined".to_owned()
        }
    );
    assert_eq!(
        answer_on(&world, request),
        None,
        "the invitation is as it was"
    );

    outbox.drain_at(&world, after_the_window());
    assert_eq!(outbox.sent(), 0, "nothing was sent");
}

#[test]
fn when_the_window_closes_the_answer_stands() {
    // Research R9: the due timer turns `accepting` into `accepted` once the
    // window has closed, and nothing earlier.
    let world = World::new();
    sending(&world);
    let request = invited(&world);
    let (client, events) = world.frontend(ClientKind::Focus);
    world.send(
        &client,
        Command::AcceptInvite {
            message: Some(request),
        },
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });

    let settle = |now: DateTime<Utc>| {
        world
            .rt
            .block_on(crate::focus::settle_answers(world.database(), now))
            .expect("the timer's pass")
    };
    assert!(settle(Utc::now()).is_empty(), "not while it is open");
    assert_eq!(
        answer_on(&world, request),
        Some(postio_model::listing::InviteAnswer::Accepting)
    );

    let settled = settle(after_the_window());
    assert_eq!(settled, vec![(world.account, request)]);
    assert_eq!(
        answer_on(&world, request),
        Some(postio_model::listing::InviteAnswer::Accepted)
    );
}

#[test]
fn an_invitation_answers_only_from_an_address_it_invited() {
    // "With no match, there is no answer": an invitation that names none of
    // the person's addresses among its attendees has nobody to answer as.
    let world = World::new();
    sending(&world);
    let request = invited(&world);
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .get(world.account)
            .await
            .expect("a read")
            .expect("the account");
        for identity in account.identities {
            postio_storage::repository::IdentityRepository::new(&connection)
                .delete(identity.id)
                .await
                .expect("gone");
        }
        let mut other = postio_model::Identity::new(
            account.id,
            postio_model::EmailAddress::new(None::<&str>, "desk@example.com"),
        );
        other.is_default = true;
        postio_storage::repository::IdentityRepository::new(&connection)
            .create(&mut other)
            .await
            .expect("an identity");
    });
    let (client, events) = world.frontend(ClientKind::Focus);

    world.send(
        &client,
        Command::AcceptInvite {
            message: Some(request),
        },
    );
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert!(
        matches!(
            &said,
            Event::CommandRejected { reason, .. }
                if reason.contains("does not name any address you send from")
        ),
        "refused, saying why: {said:?}"
    );
    assert_eq!(answer_on(&world, request), None);
}

// ── Dismissing a marker (T118) ──────────────────────────────────────────────

/// A `config.toml` in a directory of its own, saying `text`.
fn config_file(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, text).expect("the file");
    (directory, path)
}

/// `[focus]` as the file at `path` now says it.
fn focus_in(path: &std::path::Path) -> postio_config::FocusConfig {
    postio_config::Config::from_toml_str(&std::fs::read_to_string(path).expect("the file"))
        .expect("it reads")
        .focus
}

/// A dismissal, or its undoing, of `message`'s marker, as the frontend
/// sends it.
fn dismiss(message: MessageId, dismissed: bool) -> Command {
    Command::DismissMarker {
        target: MessageTarget::Messages(vec![message]),
        dismissed,
    }
}

#[test]
fn a_dismissed_marker_stays_gone_and_three_from_one_sender_stop_that_kind() {
    // US12 scenario 5 and FR-108. A dismissed marker never returns on its
    // message, however often the message is classified. Three questions
    // from one sender dismissed as wrong teach Focus that this sender's
    // questions are not questions to the person: `[focus.filter]
    // stop_markers` gains the correction, written as an editor saves it,
    // and the sender's next question is not marked. Undo takes both back.
    let world = World::new();
    let (_directory, path) = config_file("# Mine.\n[ui]\ndensity = \"compact\"\n");
    let questions: Vec<MessageId> = (0..3)
        .map(|_| {
            crate::tests::letter_from_tove(
                &world,
                crate::tests::saturday_noon(),
                &format!("Hi,\n\n{}\n\nThanks,\nTove", crate::tests::APPROVE),
                |_| {},
            )
        })
        .collect();
    let focus = world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config_path(path.clone()));
    crate::tests::eventually(&world, || focus.caught_up().then_some(()));
    for question in &questions {
        assert!(
            crate::tests::marker_on(&world, *question).is_some(),
            "the fixture's questions are marked"
        );
    }
    let (client, events) = world.frontend(ClientKind::Focus);

    for (n, question) in questions.iter().enumerate() {
        world.send(&client, dismiss(*question, true));
        let said = world.hear(&events, |event| {
            matches!(
                event,
                Event::ActionCompleted { .. } | Event::CommandRejected { .. }
            )
        });
        // Dismissals in one breath are one gesture, as archives are: one
        // toast that counts them, and one undo.
        assert_eq!(
            said,
            Event::ActionCompleted {
                description: format!(
                    "Dismissed {} marker{}",
                    n + 1,
                    if n == 0 { "" } else { "s" }
                ),
                undoable: true,
            }
        );
        let marker = crate::tests::marker_on(&world, *question).expect("the marker is kept");
        assert!(marker.dismissed_at.is_some(), "dismissed");
        assert_eq!(
            focus_in(&path).filter.stop_markers.is_empty(),
            n < 2,
            "the correction is written at the third dismissal, and not before"
        );
    }
    let written = std::fs::read_to_string(&path).expect("the file");
    assert!(
        written.starts_with("# Mine.\n[ui]\ndensity = \"compact\"\n"),
        "{written}"
    );
    assert_eq!(
        focus_in(&path).filter.stop_markers,
        vec![postio_config::StopMarker {
            sender: "tove@example.org".to_owned(),
            kind: "question".to_owned(),
        }]
    );

    // Classified again, the first question's marker does not come back.
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::sql::execute(
            &connection,
            "DELETE FROM focus_classified WHERE message_id = ?1",
            [questions[0].get()],
        )
        .await
        .expect("its record gone");
    });
    world.host().inner.hub.emit(Event::BodyLoaded {
        account: world.account,
        message: questions[0],
    });
    crate::tests::eventually(&world, || {
        crate::tests::body_classified_at(&world, questions[0])
    });
    assert!(
        crate::tests::marker_on(&world, questions[0])
            .is_some_and(|marker| marker.dismissed_at.is_some()),
        "still dismissed"
    );

    // The same sender's next question is not marked.
    let next = crate::tests::letter_from_tove(
        &world,
        Utc::now(),
        &format!("Hi,\n\n{}\n\nThanks,\nTove", crate::tests::APPROVE),
        |_| {},
    );
    world.host().inner.hub.emit(Event::BodyLoaded {
        account: world.account,
        message: next,
    });
    crate::tests::eventually(&world, || crate::tests::body_classified_at(&world, next));
    assert_eq!(
        crate::tests::marker_on(&world, next),
        None,
        "the sender's questions are not marked any more"
    );

    // Undo takes the dismissals back, and the correction they taught.
    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    for question in &questions {
        assert!(
            crate::tests::marker_on(&world, *question)
                .is_some_and(|marker| marker.dismissed_at.is_none()),
            "each marker is back"
        );
    }
    assert!(
        focus_in(&path).filter.stop_markers.is_empty(),
        "and the correction is gone"
    );
}

// ── Restoring from Filtered (T125) ──────────────────────────────────────────

/// File `message` as an arrival in the inbox, through the filing pass
/// Focus mode has installed: what the next sync would do with it.
fn file_now(world: &World, message: MessageId) {
    let pass = world
        .host()
        .wiring()
        .filing
        .get()
        .expect("Focus mode's filing pass");
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        let row = MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        let filed = [postio_sync::FiledMessage {
            message: &row,
            thread: row.thread_id,
            role: postio_model::MailboxRole::Inbox,
        }];
        let filed = &filed;
        let pass = &pass;
        postio_storage::transaction(&connection, |transaction| async move {
            pass.file(&transaction, filed).await
        })
        .await
        .expect("filed");
    });
}

#[test]
fn a_restore_brings_the_message_back_pins_its_sender_and_one_undo_reverses_both() {
    // US9 scenario 4, FR-116: `R` returns a filtered message to the inbox
    // and never filters its sender again -- the correction written to
    // `[focus.filter] never` -- and one `Ctrl+Z` reverses all of it: the
    // message goes back to Filtered with its reason, and the sender is
    // filtered again.
    let world = World::new();
    let (_directory, path) = config_file("[ui]\ndensity = \"compact\"\n");
    crate::tests::focus_ran_before(&world);
    let notification =
        crate::tests::filed_elsewhere(&world, "notifications@forge.example", "Build passed");
    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config_path(path.clone()));
    let (role, reason) = crate::tests::eventually(&world, || {
        crate::tests::filed_where(&world, "Build passed")
            .filter(|(role, _)| *role == postio_model::MailboxRole::Archive)
    });
    assert_eq!(
        role,
        postio_model::MailboxRole::Archive,
        "Focus filtered it"
    );
    let reason = reason.expect("with its reason");
    let (client, events) = world.frontend(ClientKind::Focus);

    world.send(
        &client,
        Command::RestoreFiltered {
            target: MessageTarget::Messages(vec![notification]),
            restored: true,
        },
    );
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Restored 1 message".to_owned(),
            undoable: true,
        }
    );
    assert_eq!(
        crate::tests::filed_where(&world, "Build passed"),
        Some((postio_model::MailboxRole::Inbox, None)),
        "back in the inbox, and no longer filtered"
    );
    assert_eq!(
        focus_in(&path).filter.never,
        vec!["notifications@forge.example"],
        "its sender pinned"
    );
    let next = crate::tests::filed_elsewhere(&world, "notifications@forge.example", "Build failed");
    file_now(&world, next);
    assert_eq!(
        crate::tests::filed_where(&world, "Build failed"),
        Some((postio_model::MailboxRole::Inbox, None)),
        "and the sender is never filtered again"
    );

    world.send(&client, Command::Undo);
    let undone = world.hear(&events, |event| {
        matches!(
            event,
            Event::UndoPerformed { .. } | Event::CommandRejected { .. }
        )
    });
    assert!(matches!(undone, Event::UndoPerformed { .. }), "{undone:?}");
    assert_eq!(
        crate::tests::filed_where(&world, "Build passed"),
        Some((postio_model::MailboxRole::Archive, Some(reason))),
        "back in Filtered, with its reason"
    );
    assert!(focus_in(&path).filter.never.is_empty(), "and unpinned");
    let again = crate::tests::filed_elsewhere(&world, "notifications@forge.example", "Build fixed");
    file_now(&world, again);
    assert_eq!(
        crate::tests::filed_where(&world, "Build fixed").map(|(role, _)| role),
        Some(postio_model::MailboxRole::Archive),
        "the sender's mail is filtered again"
    );
}

// ── Sweeping the inbox (T128) ───────────────────────────────────────────────

#[test]
fn a_sweep_says_what_it_would_move_then_moves_exactly_that_as_one_undo() {
    // FR-118: filtering applies to mail filed after it is turned on, so the
    // mail already in the inbox when Focus first opens stays there. Moving
    // it is a deliberate command, which says first how much would move,
    // then moves exactly that, and is one undoable action.
    let world = World::new();
    crate::tests::filed_elsewhere(&world, "notifications@forge.example", "Build passed");
    crate::tests::filed_elsewhere(&world, "alerts@builds.example", "Deploy finished");
    let (letter, _) = letter(
        &world,
        "tove@example.org",
        "Lunch?",
        "<lunch@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::hours(1),
    );
    world.host().enable_focus(crate::FocusSetup::default());
    sorted_through(&world, letter);
    let inbox = postio_model::MailboxRole::Inbox;
    for subject in ["Build passed", "Deploy finished", "Lunch?"] {
        assert_eq!(
            crate::tests::filed_where(&world, subject),
            Some((inbox, None)),
            "{subject}: Focus's first open moved nothing already in the inbox"
        );
    }
    let (client, events) = world.frontend(ClientKind::Focus);

    let preview = world
        .rt
        .block_on(client.sweep_preview())
        .expect("a preview");
    assert_eq!(preview, 2, "the two automated messages would move");

    world.send(&client, Command::SweepInbox);
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Filtered 2 messages out of the inbox".to_owned(),
            undoable: true,
        }
    );
    let archive = postio_model::MailboxRole::Archive;
    let mut moved = 0;
    for subject in ["Build passed", "Deploy finished"] {
        let (role, reason) = crate::tests::filed_where(&world, subject).expect("still here");
        assert_eq!(role, archive, "{subject}");
        assert!(reason.is_some(), "{subject}: filed away with its reason");
        moved += 1;
    }
    assert_eq!(moved, preview, "the sweep moved what the preview counted");
    assert_eq!(
        crate::tests::filed_where(&world, "Lunch?"),
        Some((inbox, None)),
        "a letter stays"
    );

    world.send(&client, Command::Undo);
    let undone = world.hear(&events, |event| {
        matches!(
            event,
            Event::UndoPerformed { .. } | Event::CommandRejected { .. }
        )
    });
    assert!(matches!(undone, Event::UndoPerformed { .. }), "{undone:?}");
    for subject in ["Build passed", "Deploy finished"] {
        assert_eq!(
            crate::tests::filed_where(&world, subject),
            Some((inbox, None)),
            "{subject}: one undo put it back, unfiltered"
        );
    }
}

// ── Surfaced rows (T136) ────────────────────────────────────────────────────

/// A message from `from` in the world's inbox, received `ago`, held for the
/// "Newsletters" rule from then.
fn newsletter(world: &World, from: &str, subject: &str, ago: chrono::TimeDelta) -> MessageId {
    let (id, _) = letter(
        world,
        from,
        subject,
        &format!(
            "<{}@ledger.example>",
            subject.to_lowercase().replace(' ', "-")
        ),
        None,
        Utc::now() - ago,
    );
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::repository::DigestRepository::new(&connection)
            .hold(id, "Newsletters", Utc::now() - ago)
            .await
            .expect("held");
    });
    id
}

/// The surfaced rows, as a Focus client reads them.
fn surfaced(world: &World, client: &postio_client::Client) -> Vec<postio_model::listing::Surfaced> {
    world
        .rt
        .block_on(client.surfaced())
        .expect("the surfaced rows")
}

#[test]
fn a_delivered_digest_is_announced_and_listed_with_its_count_senders_and_place() {
    // FR-123, FR-124: when a digest comes due, one row surfaces among the
    // conversations, saying its cadence, its name, its count and its
    // senders; the frontend hears that the surfaced rows changed, and reads
    // them -- each with its time and where it sits in the inbox.
    let world = World::new();
    let config = crate::tests::digesting("cadence = \"daily\"\nat = \"00:00\"");
    newsletter(
        &world,
        "news@ledger.example",
        "The weekly numbers",
        chrono::TimeDelta::days(2),
    );
    newsletter(
        &world,
        "news@ledger.example",
        "The rate decision",
        chrono::TimeDelta::days(1),
    );
    let (client, events) = world.frontend(ClientKind::Focus);
    assert!(
        surfaced(&world, &client).is_empty(),
        "nothing has come due yet"
    );

    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config(config));
    world.hear(&events, |event| matches!(event, Event::SurfacedChanged));

    let rows = surfaced(&world, &client);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let postio_model::listing::Surfaced::Digest {
        rule,
        cadence,
        count,
        senders,
        at,
        position,
        ..
    } = &rows[0]
    else {
        panic!("a digest row: {rows:?}");
    };
    assert_eq!(rule, "Newsletters");
    assert_eq!(*cadence, Some(postio_model::listing::Cadence::Daily));
    assert_eq!(*count, 2);
    assert_eq!(
        senders
            .iter()
            .map(|sender| sender.address.as_str())
            .collect::<Vec<_>>(),
        ["news@ledger.example"]
    );
    let (_, due, _) = crate::tests::deliveries(&world).remove(0);
    assert_eq!(*at, due, "it sits at the time it came due");
    assert_eq!(
        *position, 1,
        "below the one conversation newer than it: the world's own message"
    );
}

#[test]
fn a_fired_reminder_is_announced_and_listed_with_its_conversation() {
    // A reminder nobody answered surfaces too: the frontend hears it, and
    // reads the conversation it is about and the day it was set.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, thread) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(3),
    );
    world.send(
        &client,
        remind(budget, Some(Utc::now() - chrono::TimeDelta::hours(1))),
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    let set_at = reminder_on(&world, thread).expect("set").set_at;

    world.host().enable_focus(crate::FocusSetup::default());
    world.hear(&events, |event| matches!(event, Event::SurfacedChanged));

    let rows = surfaced(&world, &client);
    let [
        postio_model::listing::Surfaced::Reminder {
            thread: surfaced_thread,
            since,
            representative,
            ..
        },
    ] = rows.as_slice()
    else {
        panic!("one reminder row: {rows:?}");
    };
    assert_eq!(*surfaced_thread, thread);
    assert_eq!(*since, set_at, "No reply since the day it was set");
    assert_eq!(
        representative.id, budget,
        "the conversation's latest message"
    );
}

#[test]
fn an_archived_reminder_row_leaves_and_undo_brings_it_back() {
    // T095: a surfaced reminder is a row of Focus's inbox while its
    // conversation is in an inbox. Archiving it takes the row away with
    // the conversation, and undoing the archive brings both back.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Focus);
    let (budget, _) = letter(
        &world,
        "tove@example.org",
        "Atlas Q3 budget",
        "<atlas@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(3),
    );
    world.send(
        &client,
        remind(budget, Some(Utc::now() - chrono::TimeDelta::hours(1))),
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    world.host().enable_focus(crate::FocusSetup::default());
    world.hear(&events, |event| matches!(event, Event::SurfacedChanged));
    assert_eq!(surfaced(&world, &client).len(), 1, "the reminder surfaced");

    world.send(
        &client,
        Command::default_for(postio_core::CommandId::Archive)
            .with_target(MessageTarget::Messages(vec![budget])),
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    assert!(
        surfaced(&world, &client).is_empty(),
        "archived, the conversation's reminder row went with it"
    );

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(
        surfaced(&world, &client).len(),
        1,
        "undone, the reminder row is back"
    );
}

// ── The digest verbs (T137, T138, T139) ─────────────────────────────────────

/// Every message a digest rule holds, and whether each is delivered yet.
fn holds(world: &World) -> Vec<(MessageId, String, bool)> {
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        postio_storage::sql::all(
            &reader,
            "SELECT message_id, rule, delivery_id IS NOT NULL FROM digest_holds ORDER BY message_id",
            (),
            |row| {
                use postio_storage::sql::RowExt as _;
                Ok((MessageId::new(row.col(0)?), row.col(1)?, row.col(2)?))
            },
        )
        .await
        .expect("a read")
    })
}

/// The one digest row surfaced, as a Focus client reads it.
fn the_digest(world: &World, client: &postio_client::Client) -> postio_model::DeliveryId {
    match surfaced(world, client).as_slice() {
        [postio_model::listing::Surfaced::Digest { delivery, .. }] => *delivery,
        other => panic!("one digest row: {other:?}"),
    }
}

/// Where the message is: its folder's role.
fn role_of(world: &World, message: MessageId) -> postio_model::MailboxRole {
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        let row = MessageRepository::new(&reader)
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        postio_storage::repository::MailboxRepository::new(&reader)
            .get(row.mailbox_id)
            .await
            .expect("a read")
            .expect("its folder")
            .role
    })
}

#[test]
fn archiving_a_digest_archives_every_message_in_it_as_one_undo() {
    // US10 scenario 4, FR-125: `⇧A` in the digest archives all its messages
    // as one action, its row leaves the inbox, and one `Ctrl+Z` puts both
    // back.
    let world = World::new();
    let first = newsletter(
        &world,
        "news@ledger.example",
        "The weekly numbers",
        chrono::TimeDelta::days(2),
    );
    let second = newsletter(
        &world,
        "news@ledger.example",
        "The rate decision",
        chrono::TimeDelta::days(1),
    );
    let (client, events) = world.frontend(ClientKind::Focus);
    world.host().enable_focus(
        crate::FocusSetup::default().with_config(crate::tests::digesting(
            "cadence = \"daily\"\nat = \"00:00\"",
        )),
    );
    world.hear(&events, |event| matches!(event, Event::SurfacedChanged));
    let delivery = the_digest(&world, &client);

    world.send(
        &client,
        Command::ArchiveDigest {
            delivery,
            archived: true,
        },
    );
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Archived 2 messages".to_owned(),
            undoable: true,
        }
    );
    let archive = postio_model::MailboxRole::Archive;
    assert_eq!(
        (role_of(&world, first), role_of(&world, second)),
        (archive, archive)
    );
    world.hear(&events, |event| matches!(event, Event::SurfacedChanged));
    assert!(
        surfaced(&world, &client).is_empty(),
        "its row left the inbox"
    );

    world.send(&client, Command::Undo);
    let undone = world.hear(&events, |event| {
        matches!(
            event,
            Event::UndoPerformed { .. } | Event::CommandRejected { .. }
        )
    });
    assert!(matches!(undone, Event::UndoPerformed { .. }), "{undone:?}");
    let inbox = postio_model::MailboxRole::Inbox;
    assert_eq!(
        (role_of(&world, first), role_of(&world, second)),
        (inbox, inbox)
    );
    assert_eq!(the_digest(&world, &client), delivery, "and its row is back");
}

/// `[[focus.digests]]` "Newsletters" for two senders, weekly, on the day
/// three days from now, so nothing comes due in a test.
///
/// The day is computed, not written down: the tests hold mail from one and
/// two days ago, and a rule comes due at its first time after the oldest
/// hold. A fixed "sunday" came due on any Monday or Tuesday run, and the due
/// timer's first tick, which is immediate, then raced the test's own verbs.
fn two_senders() -> String {
    use chrono::Datelike as _;
    let day = (Utc::now() + chrono::TimeDelta::days(3))
        .weekday()
        .to_string()
        .to_lowercase();
    let day = match day.as_str() {
        "mon" => "monday",
        "tue" => "tuesday",
        "wed" => "wednesday",
        "thu" => "thursday",
        "fri" => "friday",
        "sat" => "saturday",
        _ => "sunday",
    };
    format!(
        "[[focus.digests]]
name = \"Newsletters\"
match = [\"from:news@ledger.example\", \"from:editor@ledger.example\"]
cadence = \"weekly\"
day = \"{day}\"
at = \"09:00\"
"
    )
}

#[test]
fn stopping_a_sender_releases_what_the_rule_held_for_them_and_undo_puts_it_back() {
    // US10 scenario 5, FR-125: `D` stops digesting the sender -- the rule
    // in config.toml loses them, what it held of theirs rejoins the inbox,
    // and their next message goes to the inbox -- and one undo takes it all
    // back.
    let world = World::new();
    let (_directory, path) = config_file(&two_senders());
    let news = newsletter(
        &world,
        "news@ledger.example",
        "The weekly numbers",
        chrono::TimeDelta::days(2),
    );
    let more = newsletter(
        &world,
        "news@ledger.example",
        "The rate decision",
        chrono::TimeDelta::days(1),
    );
    let editor = newsletter(
        &world,
        "editor@ledger.example",
        "A letter",
        chrono::TimeDelta::days(1),
    );
    world.host().enable_focus(
        crate::FocusSetup::default()
            .with_config(focus_in(&path))
            .with_config_path(path.clone()),
    );
    let (client, events) = world.frontend(ClientKind::Focus);

    world.send(
        &client,
        Command::StopDigestingSender {
            target: MessageTarget::Messages(vec![news]),
            stopped: true,
            kept: None,
        },
    );
    let said = world.hear(&events, |event| {
        matches!(
            event,
            Event::ActionCompleted { .. } | Event::CommandRejected { .. }
        )
    });
    assert_eq!(
        said,
        Event::ActionCompleted {
            description: "Stopped digesting 1 sender".to_owned(),
            undoable: true,
        }
    );
    assert_eq!(
        focus_in(&path).digests[0].queries,
        ["from:editor@ledger.example"],
        "the rule no longer holds the sender"
    );
    assert_eq!(
        holds(&world),
        vec![(editor, "Newsletters".to_owned(), false)],
        "what it held of theirs rejoined the inbox"
    );
    let next = newsletter_unheld(&world, "news@ledger.example", "The next issue");
    file_now(&world, next);
    assert!(
        holds(&world).iter().all(|(held, _, _)| *held != next),
        "their next message goes to the inbox"
    );

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(
        focus_in(&path).digests[0].queries,
        ["from:news@ledger.example", "from:editor@ledger.example"],
        "the rule is as it was"
    );
    let held: Vec<MessageId> = holds(&world).into_iter().map(|(held, _, _)| held).collect();
    for message in [news, more, editor] {
        assert!(held.contains(&message), "{message:?} is held again");
    }
}

#[test]
fn stopping_a_rule_s_only_sender_removes_the_rule_and_undo_puts_it_back_whole() {
    // A rule made by "Digest this sender" names one sender. Stopping them
    // leaves a rule that holds nothing, which is no rule: it goes, and undo
    // puts it back exactly, cadence and all.
    let world = World::new();
    let (_directory, path) =
        config_file(&two_senders().replace(", \"from:editor@ledger.example\"", ""));
    let news = newsletter(
        &world,
        "news@ledger.example",
        "The weekly numbers",
        chrono::TimeDelta::days(1),
    );
    world.host().enable_focus(
        crate::FocusSetup::default()
            .with_config(focus_in(&path))
            .with_config_path(path.clone()),
    );
    let (client, events) = world.frontend(ClientKind::Focus);
    let before = focus_in(&path).digests;

    world.send(
        &client,
        Command::StopDigestingSender {
            target: MessageTarget::Messages(vec![news]),
            stopped: true,
            kept: None,
        },
    );
    world.hear(&events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    assert!(focus_in(&path).digests.is_empty(), "the rule went");
    assert!(holds(&world).is_empty(), "and holds nothing");

    world.send(&client, Command::Undo);
    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(focus_in(&path).digests, before, "back exactly");
    assert_eq!(holds(&world).len(), 1, "holding its mail again");
}

/// A message from `from` in the world's inbox, received now, held by
/// nothing.
fn newsletter_unheld(world: &World, from: &str, subject: &str) -> MessageId {
    letter(
        world,
        from,
        subject,
        &format!(
            "<{}@ledger.example>",
            subject.to_lowercase().replace(' ', "-")
        ),
        None,
        Utc::now(),
    )
    .0
}

#[test]
fn a_rule_s_preview_counts_what_the_executor_finds_in_the_last_ninety_days() {
    // US10 scenario 1, FR-120, FR-127: the rule dialog's preview is the
    // rule's query run through the executor over the last 90 days -- its
    // count, and the newest four rows -- so it means what the same query
    // means in search.
    let world = World::new();
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
    });
    let mut recent = Vec::new();
    for days in 1..=5 {
        let (id, _) = letter(
            &world,
            "news@ledger.example",
            &format!("Issue {days}"),
            &format!("<issue-{days}@ledger.example>"),
            None,
            Utc::now() - chrono::TimeDelta::days(days),
        );
        recent.push(id);
    }
    letter(
        &world,
        "news@ledger.example",
        "An old issue",
        "<old-issue@ledger.example>",
        None,
        Utc::now() - chrono::TimeDelta::days(100),
    );
    letter(
        &world,
        "tove@example.org",
        "Lunch?",
        "<lunch@example.org>",
        None,
        Utc::now() - chrono::TimeDelta::days(1),
    );
    let (client, _) = world.frontend(ClientKind::Focus);
    let since = Utc::now() - chrono::TimeDelta::days(90);

    let preview = world
        .rt
        .block_on(client.digest_preview(vec!["from:news@ledger.example".to_owned()], since))
        .expect("a preview");

    assert_eq!(preview.count, 5, "the five in the last 90 days");
    assert_eq!(
        preview.first.iter().map(|row| row.id).collect::<Vec<_>>(),
        recent[..4],
        "the newest four"
    );
    let found = world
        .rt
        .block_on(client.search(postio_client::protocol::Search {
            account: postio_model::AccountScope::Unified,
            query: format!(
                "from:news@ledger.example after:{}",
                since.format("%Y-%m-%d")
            ),
            newest_first: true,
            scope: postio_search::facets::Scope::default(),
        }))
        .expect("a search")
        .expect("it ran");
    assert_eq!(
        u64::from(preview.count),
        found.hits,
        "the same count the executor gives the same query"
    );
}

#[test]
fn a_preview_counts_a_message_two_of_its_queries_match_once() {
    // T138: a rule of several queries -- "Digest these…" writes one per
    // sender, and a query rule may overlap them -- would have caught each
    // message once, however many of its queries match it.
    let world = World::new();
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
    });
    for days in 1..=3 {
        letter(
            &world,
            "news@ledger.example",
            &format!("Issue {days}"),
            &format!("<issue-{days}@ledger.example>"),
            None,
            Utc::now() - chrono::TimeDelta::days(days),
        );
    }
    let (client, _) = world.frontend(ClientKind::Focus);
    let since = Utc::now() - chrono::TimeDelta::days(90);
    let preview = world
        .rt
        .block_on(client.digest_preview(
            vec![
                "from:news@ledger.example".to_owned(),
                "subject:issue".to_owned(),
            ],
            since,
        ))
        .expect("a preview");
    assert_eq!(
        preview.count, 3,
        "three messages, each matched by both queries"
    );
    assert_eq!(preview.first.len(), 3, "each listed once");
}

#[test]
fn a_rule_is_written_to_config_edited_in_its_place_and_removing_it_releases_its_mail() {
    // FR-120, FR-126: the dialog's Create writes the rule to config.toml,
    // an edit rewrites it where it stands, and removing it at `g d` takes
    // it out of the file and releases what it held into the inbox.
    let world = World::new();
    let (_directory, path) = config_file("# Mine.\n");
    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config_path(path.clone()));
    let (client, _) = world.frontend(ClientKind::Focus);
    let weekly = |name: &str| postio_client::protocol::DigestRuleDraft {
        name: name.to_owned(),
        queries: vec!["from:news@ledger.example".to_owned()],
        cadence: postio_model::listing::Cadence::Weekly,
        day: Some(postio_client::protocol::RuleDay::Weekday(
            chrono::Weekday::Sun,
        )),
        at: chrono::NaiveTime::from_hms_opt(9, 0, 0).expect("a time"),
    };

    world
        .rt
        .block_on(client.save_digest_rule(None, weekly("Ledger")))
        .expect("written");
    let written = focus_in(&path).digests;
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].name, "Ledger");
    assert_eq!(written[0].queries, ["from:news@ledger.example"]);
    assert_eq!(written[0].cadence, "weekly");
    assert_eq!(written[0].at, "09:00");
    assert!(
        std::fs::read_to_string(&path)
            .expect("the file")
            .starts_with("# Mine.\n"),
        "nothing else moved"
    );

    world
        .rt
        .block_on(client.save_digest_rule(Some("Ledger".to_owned()), weekly("The Ledger")))
        .expect("edited");
    let names: Vec<String> = focus_in(&path)
        .digests
        .into_iter()
        .map(|rule| rule.name)
        .collect();
    assert_eq!(names, ["The Ledger"], "edited, not added");

    let held = newsletter_unheld(&world, "news@ledger.example", "The weekly numbers");
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_storage::repository::DigestRepository::new(&connection)
            .hold(held, "The Ledger", Utc::now())
            .await
            .expect("held");
    });
    let released = world
        .rt
        .block_on(client.delete_digest_rule("The Ledger".to_owned()))
        .expect("removed");
    assert_eq!(released, 1);
    assert!(focus_in(&path).digests.is_empty(), "gone from the file");
    assert!(holds(&world).is_empty(), "and its mail rejoined the inbox");
}

#[test]
fn view_source_fetches_the_raw_message_when_asked_and_not_before() {
    // Spec 007 US2 scenario 5, T068: `v` shows the raw message's header
    // lines. The background lane stores no raw source (ADR 0017), so the
    // bytes come from the server on the key press -- and only then.
    use crate::tests::{eventually, row_titled, server_with_one_message, syncing_world};
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let (client, _events) = world.frontend(ClientKind::Focus);
    world.host().start_syncing();
    let message = eventually(&world, || row_titled(&world, &client, "Tide gate"));
    assert!(mock.body_fetches().is_empty(), "nothing fetched before `v`");

    let raw = world
        .rt
        .block_on(client.raw_source(message))
        .expect("the raw source");
    let text = String::from_utf8(raw).expect("the fixture is text");
    assert!(text.contains("Subject: Tide gate\r\n"), "{text}");
    assert!(text.contains("From: Ada <ada@example.com>\r\n"), "{text}");
    assert!(
        text.contains("Message-ID: <tide@example.com>\r\n"),
        "{text}"
    );
    assert_eq!(
        mock.body_fetches(),
        vec!["INBOX".to_owned()],
        "fetched once, on the request"
    );

    // Asked again, the source is on this machine and nothing is fetched.
    let again = world
        .rt
        .block_on(client.raw_source(message))
        .expect("the raw source again");
    assert_eq!(String::from_utf8(again).expect("text"), text);
    assert_eq!(mock.body_fetches().len(), 1, "read from the blob store");
}

#[test]
fn a_first_sync_that_lands_between_ticks_is_not_sorted_at_the_next_open() {
    // Spec 007 T164, FR-118: a first sync files its backlog, which is never
    // filtered. Focus keeps its mark on its tick, so a first sync that lands
    // after one tick and before the next, then a quit, left its rows past
    // the mark -- and the next open sorted them as if another app had
    // filed them. Stopping keeps the mark current.
    let mut world = World::new();
    world.host().enable_focus(crate::FocusSetup::default());
    sorted_through(&world, world.message());
    // The first tick, at once after the catch-up, has moved the mark.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // A new account's first sync, between two ticks: no filing pass sees it.
    let first_sync =
        crate::tests::filed_elsewhere(&world, "notifications@forge.example", "Build passed");
    world.relaunch();
    world.host().enable_focus(crate::FocusSetup::default());
    sorted_through(&world, first_sync);

    assert_eq!(
        crate::tests::filed_where(&world, "Build passed"),
        Some((postio_model::MailboxRole::Inbox, None)),
        "the first sync's row stays in the inbox"
    );
}

// ── List and query rules (spec 007 T155, US14) ──────────────────────────────

#[test]
fn a_query_rule_s_preview_lists_what_it_matches_and_the_rule_holds_the_same() {
    // US14 scenario 2, and the story's Independent Test: a query rule's
    // preview is the query through the executor over the recent window, in
    // the one language search speaks (ADR 0008), and what it lists is what
    // the rule then holds as mail is filed.
    let world = World::new();
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
    });
    let mut receipts = Vec::new();
    for days in 1..=3 {
        let (id, _) = letter(
            &world,
            "orders@shop.example",
            &format!("Your receipt for order {days}"),
            &format!("<receipt-{days}@shop.example>"),
            None,
            Utc::now() - chrono::TimeDelta::days(days),
        );
        receipts.push(id);
    }
    let (shipped, _) = letter(
        &world,
        "orders@shop.example",
        "Your order has shipped",
        "<shipped@shop.example>",
        None,
        Utc::now() - chrono::TimeDelta::days(1),
    );
    let (client, _) = world.frontend(ClientKind::Focus);
    let query = "subject:receipt from:orders@shop.example";

    let preview = world
        .rt
        .block_on(client.digest_preview(
            vec![query.to_owned()],
            Utc::now() - chrono::TimeDelta::days(90),
        ))
        .expect("a preview");

    assert_eq!(preview.count, 3);
    let mut listed: Vec<MessageId> = preview.first.iter().map(|row| row.id).collect();
    listed.sort();
    assert_eq!(listed, receipts);
    let rule = postio_search::matcher::Matcher::new(&postio_search::parse(
        query,
        chrono::Local::now().date_naive(),
    ))
    .expect("a rule can hold by it as mail is filed");
    world.rt.block_on(async {
        let reader = world.database().read().await.expect("a reader");
        let messages = MessageRepository::new(&reader);
        for id in receipts.iter().chain([&shipped]) {
            let row = messages.get(*id).await.expect("a read").expect("a row");
            assert_eq!(
                rule.matches(&row),
                receipts.contains(id),
                "the rule holds what the preview listed, and nothing else"
            );
        }
    });
}

#[test]
fn a_rule_the_filing_pass_could_never_answer_is_refused_with_a_sentence() {
    // FR-171: a rule holds mail as it is filed, before its body is here, so
    // it can ask only what is known then. A query beyond that would be
    // saved and hold nothing; it is refused, saying what a rule can use.
    let world = World::new();
    let (_directory, path) = config_file("");
    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config_path(path.clone()));
    let (client, _) = world.frontend(ClientKind::Focus);
    let rule = |query: &str| postio_client::protocol::DigestRuleDraft {
        name: "Invoices".to_owned(),
        queries: vec![query.to_owned()],
        cadence: postio_model::listing::Cadence::Daily,
        day: None,
        at: chrono::NaiveTime::from_hms_opt(9, 0, 0).expect("a time"),
    };

    for query in ["invoice", "from:orders@shop.example has:attachment"] {
        let refused = world
            .rt
            .block_on(client.save_digest_rule(None, rule(query)))
            .expect_err(query);
        assert!(refused.to_string().contains("list:"), "{refused}");
    }
    assert!(focus_in(&path).digests.is_empty(), "nothing written");

    world
        .rt
        .block_on(client.save_digest_rule(None, rule("list:billing.lists.example.org")))
        .expect("a list rule is written");
    assert_eq!(
        focus_in(&path).digests[0].queries,
        ["list:billing.lists.example.org"]
    );
}
