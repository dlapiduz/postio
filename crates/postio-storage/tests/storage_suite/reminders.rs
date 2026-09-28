//! Focus's reminders, as the store keeps them (spec 007 US5, data-model.md
//! "`reminders`"): what each read of them costs.
//!
//! The due timer asks for what has come due every five seconds for as long
//! as Focus runs, the filing pass asks about the conversations a reply
//! joined, and the surfaced rows are read with Focus's inbox. None of them
//! may walk the reminders or the mail.

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
