use postio_storage::repository::ReminderRepository;
use postio_storage::test_support;

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
    let sql = postio_storage::repository::MarkerRepository::explain_answers_due();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn counting_a_sender_s_dismissals_walks_no_table() {
    let sql = postio_storage::repository::MarkerRepository::explain_dismissed_from();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn asking_whether_a_restore_still_stands_behind_a_pin_walks_no_table() {
    let sql = postio_storage::repository::FilterDecisionRepository::explain_restored_from();
    let walked = scans(sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn a_sweep_walks_the_inbox_a_window_at_a_time_by_its_index() {
    let sql = postio_storage::repository::MessageRepository::explain_focus_inbox_window();
    let walked = scans(&sql).await;
    assert!(walked.is_empty(), "{sql}\nwalks: {walked:?}");
}

#[tokio::test]
async fn the_surfaced_rows_are_read_without_a_walk() {
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
