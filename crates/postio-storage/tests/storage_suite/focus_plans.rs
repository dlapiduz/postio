//! What the reads behind Focus's verbs and its due timer cost (spec 007):
//! reminders, answers waiting out their windows, dismissals and restores.
//!
//! The due timer asks what has come due every five seconds for as long as
//! Focus runs, the filing pass asks about the conversations a reply joined,
//! and a verb asks about one sender's mail. None of them may walk a table:
//! each is asked of the planner here.

use postio_storage::repository::ReminderRepository;
use postio_storage::test_support;

/// The steps of `sql`'s plan that walk a table.
async fn scans(sql: &str) -> Vec<String> {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let plan = test_support::plan(&connection, sql).await;
    assert!(!plan.is_empty(), "{sql}: no plan");
    plan.lines()
        .filter(|step| step.trim_start().starts_with("SCAN"))
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn no_read_of_the_reminders_walks_a_table() {
    for sql in [
        ReminderRepository::explain_due().to_owned(),
        ReminderRepository::explain_surfaced().to_owned(),
        ReminderRepository::explain_standing_on(3),
        ReminderRepository::explain_writers_since().to_owned(),
    ] {
        let walked = scans(&sql).await;
        assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
    }
}

#[tokio::test]
async fn the_due_timer_finds_answers_whose_window_closed_without_a_walk() {
    // Research R9: every tick asks which answers' windows have closed. The
    // markers are every invitation, question and to-do Focus has seen, so
    // the question has to be a seek, not a walk of them.
    let sql = postio_storage::repository::MarkerRepository::explain_answers_due();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn counting_a_sender_s_dismissals_walks_no_table() {
    // FR-108: every dismissal asks how many of that kind the person has
    // dismissed in that sender's mail. It is driven from the sender's
    // address, never a walk of the markers or the mail.
    let sql = postio_storage::repository::MarkerRepository::explain_dismissed_from();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn asking_whether_a_restore_still_stands_behind_a_pin_walks_no_table() {
    // FR-116: taking a restore back unpins its sender only when no other
    // restore of theirs stands. Asked from the sender's address, never a
    // walk of the decisions or the mail.
    let sql = postio_storage::repository::FilterDecisionRepository::explain_restored_from();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn a_sweep_walks_the_inbox_a_window_at_a_time_by_its_index() {
    // FR-118: a sweep reads the inbox in windows, each a seek on the
    // list's own index -- never the whole inbox at once, and never a walk
    // of the mail to find where the next window starts.
    let sql = postio_storage::repository::MessageRepository::explain_focus_inbox_window();
    let walked = scans(&sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn the_surfaced_rows_are_read_without_a_walk() {
    // Spec 007, contracts/engine.md: the digest rows, their senders, a
    // reminder's conversation and each row's place are each a seek.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let threads = postio_storage::repository::ThreadRepository::new(&connection);
    for sql in [
        postio_storage::repository::DigestRepository::explain_open_deliveries().to_owned(),
        postio_storage::repository::DigestRepository::explain_senders_of(2),
        postio_storage::repository::ThreadRepository::explain_latest_member(),
        threads.explain_focus_position(2),
    ] {
        let walked = scans(&sql).await;
        assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
    }
}

#[tokio::test]
async fn a_surfaced_row_s_position_is_one_statement() {
    // contracts/engine.md: "a surfaced row's position is 1 statement",
    // however many conversations are newer than it.
    use postio_storage::test_support::counting;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let now = chrono::Utc::now();
    for minutes in 0..5 {
        let mut message = postio_model::Message::new(
            account.id,
            inbox,
            now - chrono::TimeDelta::minutes(minutes),
        );
        postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
    }
    let threads = postio_storage::repository::ThreadRepository::new(&connection);

    counting::reset();
    let position = threads
        .focus_position(
            &[(account.id, inbox)],
            now - chrono::TimeDelta::seconds(150),
        )
        .await
        .expect("a count");
    let cost = counting::here();

    assert_eq!(
        position, 3,
        "the three newer than two and a half minutes ago"
    );
    assert_eq!(cost.statements, 1, "{cost:?}");
}

#[tokio::test]
async fn a_digest_s_verbs_find_its_mail_without_a_walk() {
    // Spec 007 FR-124, FR-125: archiving a delivery reads what it holds,
    // and stopping a sender finds and releases that sender's held mail --
    // a seek on the holds' own indexes, however much the rule has held.
    use postio_storage::repository::DigestRepository;
    for sql in [
        DigestRepository::explain_delivery_messages(),
        DigestRepository::explain_held_from(),
        DigestRepository::explain_release_sender(),
    ] {
        let walked = scans(sql).await;
        assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
    }
}
