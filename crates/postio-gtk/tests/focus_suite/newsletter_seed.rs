//! The long-newsletter seed refiles one demo conversation as a newsletter:
//! its row must describe the newsletter, not the message it was made from.

use gtk::prelude::*;

use crate::support;

pub fn the_newsletters_row_says_nothing_of_the_message_it_was_made_from() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (database, _) = postio_gtk::demo::seeded(postio_gtk::demo::Seed::LongNewsletter).await;
        let blobs = tempfile::tempdir().expect("a blob directory");
        let store = postio_storage::BlobStore::open(
            blobs.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let host = postio_host::Host::start(database, store, |wiring| wiring).expect("a host");
        let window = postio_gtk::window::FocusWindow::new(None);
        window.set_default_size(1000, 1400);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        support::keep(blobs);
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || {
                pane.rows_on_screen()
                    .iter()
                    .any(|row| row.spoken().contains("Release notes"))
            })
            .await,
            "the newsletter's row never reached the screen: {:?}",
            window.rows_on_screen()
        );
        let said = window
            .rows_on_screen()
            .into_iter()
            .find(|row| row.contains("Release notes"))
            .expect("the row");
        for harbor in ["Uploaded v3", "leave comments", "To-do", "attachment"] {
            assert!(
                !said.contains(harbor),
                "the newsletter's row carries the Harbor draft's {harbor:?}: {said}"
            );
        }
    });
}
