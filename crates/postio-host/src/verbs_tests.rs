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
