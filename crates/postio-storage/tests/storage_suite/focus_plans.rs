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
