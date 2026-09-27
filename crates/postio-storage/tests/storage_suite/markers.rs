//! Focus's markers, as the store keeps them (spec 007, data-model.md
//! "`markers`"): at most one per message, written once, replaced by an
//! invitation update, and a dismissal that stands.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::listing::{InviteAnswer, MarkerKind};
use postio_model::{Message, MessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    InviteIdentity, InviteState, Marker, MarkerRepository, MarkerSource, MessageRepository,
};
use postio_storage::test_support;

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

/// `count` messages in one inbox, oldest first.
async fn messages(connection: &Connection, count: i64) -> Vec<MessageId> {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let mut ids = Vec::new();
    for hour in 0..count {
        let mut message = Message::new(account.id, inbox, at(hour));
        ids.push(
            MessageRepository::new(connection)
                .create(&mut message)
                .await
                .expect("a message"),
        );
    }
    ids
}

async fn message(connection: &Connection) -> MessageId {
    messages(connection, 1).await[0]
}

/// An invitation to an event from 10:00 to 10:45 on the day, as its first
/// `SEQUENCE` would put it.
fn invitation(message: MessageId) -> Marker {
    Marker {
        message,
        kind: MarkerKind::Invite,
        source: MarkerSource::Calendar,
        span: None,
        excerpt: None,
        starts_at: Some(at(10)),
        ends_at: Some(at(10) + TimeDelta::minutes(45)),
        due_at: None,
        invite: Some(InviteIdentity {
            uid: "roadmap-review@calendar.example".to_owned(),
            sequence: 0,
            stamp: Some(at(1)),
        }),
        invite_state: Some(InviteState::Open),
        answer: None,
        dismissed_at: None,
    }
}

fn question(message: MessageId) -> Marker {
    Marker {
        message,
        kind: MarkerKind::Question,
        source: MarkerSource::Detector,
        span: Some((12, 48)),
        excerpt: Some("Can you send the Q3 numbers by Friday?".to_owned()),
        starts_at: None,
        ends_at: None,
        due_at: Some(at(96)),
        invite: None,
        invite_state: None,
        answer: None,
        dismissed_at: None,
    }
}

#[tokio::test]
async fn a_marker_is_written_once_per_message_and_reads_back_whole() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    let markers = MarkerRepository::new(&connection);

    assert_eq!(markers.get(message).await.expect("a read"), None);
    assert!(markers.insert(&question(message)).await.expect("written"));
    assert_eq!(
        markers.get(message).await.expect("a read"),
        Some(question(message)),
        "every field round-trips"
    );

    // A second classification of the same message writes nothing: the
    // marker it has stands, whatever the person has done with it.
    assert!(
        !markers
            .insert(&invitation(message))
            .await
            .expect("an insert that finds one"),
        "a message has one marker at most"
    );
    assert_eq!(
        markers.get(message).await.expect("a read"),
        Some(question(message))
    );
}

#[tokio::test]
async fn an_invitation_update_replaces_its_marker_and_a_dismissal_stands() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let [message, other] = messages(&connection, 2).await[..] else {
        unreachable!("two messages")
    };
    let markers = MarkerRepository::new(&connection);
    let mut accepted = invitation(message);
    accepted.answer = Some(InviteAnswer::Accepted);
    markers.insert(&accepted).await.expect("written");

    // SEQUENCE 1 moves the event an hour later: the marker takes the new
    // time, and the answer given to the old time no longer holds.
    let mut moved = invitation(message);
    moved.starts_at = Some(at(11));
    moved.ends_at = Some(at(11) + TimeDelta::minutes(45));
    moved.invite = Some(InviteIdentity {
        uid: "roadmap-review@calendar.example".to_owned(),
        sequence: 1,
        stamp: Some(at(2)),
    });
    assert!(markers.replace(&moved).await.expect("replaced"));
    assert_eq!(markers.get(message).await.expect("a read"), Some(moved));

    // A dismissed marker stays dismissed through a replacement: the
    // cancellation's new state is kept, and the marker never returns.
    markers
        .dismiss(message, Some(at(3)))
        .await
        .expect("dismissed");
    let mut cancelled = invitation(message);
    cancelled.invite_state = Some(InviteState::Cancelled);
    assert!(markers.replace(&cancelled).await.expect("replaced"));
    let read = markers
        .get(message)
        .await
        .expect("a read")
        .expect("a marker");
    assert_eq!(read.invite_state, Some(InviteState::Cancelled));
    assert_eq!(read.dismissed_at, Some(at(3)), "the dismissal stands");

    // With no marker to replace, a replacement writes nothing.
    assert!(
        !markers
            .replace(&invitation(other))
            .await
            .expect("no marker")
    );
    assert_eq!(markers.get(other).await.expect("a read"), None);
}

#[tokio::test]
async fn a_dismissal_is_taken_back_by_undo() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    let markers = MarkerRepository::new(&connection);
    assert!(
        !markers
            .dismiss(message, Some(at(3)))
            .await
            .expect("nothing to dismiss"),
        "a message with no marker has nothing to dismiss"
    );
    markers.insert(&question(message)).await.expect("written");

    assert!(
        markers
            .dismiss(message, Some(at(3)))
            .await
            .expect("dismissed")
    );
    assert_eq!(
        markers
            .get(message)
            .await
            .expect("a read")
            .and_then(|marker| marker.dismissed_at),
        Some(at(3))
    );
    assert!(markers.dismiss(message, None).await.expect("undone"));
    assert_eq!(
        markers.get(message).await.expect("a read"),
        Some(question(message)),
        "undo leaves the marker as it was before the dismissal"
    );
}

#[tokio::test]
async fn an_invitation_s_markers_are_found_by_its_uid() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let [first, second, third] = messages(&connection, 3).await[..] else {
        unreachable!("three messages")
    };
    let markers = MarkerRepository::new(&connection);
    markers.insert(&invitation(first)).await.expect("written");
    markers.insert(&invitation(second)).await.expect("written");
    markers.insert(&question(third)).await.expect("written");

    let mut found: Vec<MessageId> = markers
        .invitations("roadmap-review@calendar.example")
        .await
        .expect("a read")
        .into_iter()
        .map(|marker| marker.message)
        .collect();
    found.sort();
    assert_eq!(found, vec![first, second]);
    assert!(
        markers
            .invitations("another@calendar.example")
            .await
            .expect("a read")
            .is_empty()
    );
    let sql = "SELECT message_id FROM markers WHERE invite_uid = ?1";
    assert!(
        test_support::counting::scans(&connection, sql)
            .await
            .is_empty(),
        "finding an invitation's markers seeks:\n{}",
        test_support::plan(&connection, sql).await
    );
}

#[tokio::test]
async fn the_store_refuses_a_marker_it_has_no_word_for() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    // The words it has are stored: without this the refusals below would
    // pass against a store with no table at all.
    for (kind, source) in [
        ("invite", "calendar"),
        ("question", "detector"),
        ("todo", "model"),
        ("no_reply", "reminder"),
    ] {
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO markers (message_id, kind, source) VALUES (?1, ?2, ?3)",
            vec![
                turso::Value::Integer(message.get()),
                turso::Value::Text(kind.to_owned()),
                turso::Value::Text(source.to_owned()),
            ],
        )
        .await
        .unwrap_or_else(|error| panic!("{kind} from {source} was refused: {error}"));
        postio_storage::sql::execute(
            &connection,
            "DELETE FROM markers WHERE message_id = ?1",
            [message.get()],
        )
        .await
        .expect("cleared");
    }
    for (column, value) in [
        ("kind", "reminder"),
        ("source", "guess"),
        ("invite_state", "tentative"),
        ("answer", "maybe"),
    ] {
        let (kind, source) = match column {
            "kind" => (value, "detector"),
            "source" => ("question", value),
            _ => ("invite", "calendar"),
        };
        let extra = match column {
            "invite_state" | "answer" => format!(", {column}"),
            _ => String::new(),
        };
        let placeholder = if extra.is_empty() { "" } else { ", ?4" };
        let refused = postio_storage::sql::execute(
            &connection,
            &format!(
                "INSERT INTO markers (message_id, kind, source{extra})
                 VALUES (?1, ?2, ?3{placeholder})"
            ),
            if extra.is_empty() {
                vec![
                    turso::Value::Integer(message.get()),
                    turso::Value::Text(kind.to_owned()),
                    turso::Value::Text(source.to_owned()),
                ]
            } else {
                vec![
                    turso::Value::Integer(message.get()),
                    turso::Value::Text(kind.to_owned()),
                    turso::Value::Text(source.to_owned()),
                    turso::Value::Text(value.to_owned()),
                ]
            },
        )
        .await;
        assert!(refused.is_err(), "{column} = {value:?} was stored");
    }
}
