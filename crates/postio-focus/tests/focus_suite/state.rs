//! Knowing what state Focus is in (US6, T052): one banner under the header
//! strip, and the sync label saying the same in a word or two
//! (contracts/focus-surface.md, "States"). Each state is driven through the
//! host's event sink, as the engine would say it.

use gtk::prelude::*;
use postio_core::{ConnectionState, Event, FailureReason};

use crate::support::{self, Fixture};

pub fn each_sync_state_shows_its_banner_and_label() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_focus::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        let account = fixture.account.id;
        let tell = |event| assert!(sink.emit(event), "the hub took the event");

        // First sync: progress, no button, and the label counts.
        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account,
            done: 12_408,
            total: 18_204,
        });
        assert!(
            crate::settle_until(async || window.banner_showing().is_some()).await,
            "no banner for a first sync"
        );
        let (title, button, progress) = window.banner_showing().expect("a banner");
        assert!(
            title.starts_with("First sync. 12,408 of 18,204 messages"),
            "{title}"
        );
        assert_eq!(button, None);
        assert!(progress.is_some_and(|p| (p - 12_408.0 / 18_204.0).abs() < 1e-6));
        assert_eq!(window.sync_said(), "Syncing 12,408 of 18,204");

        // Offline wins over the first sync.
        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Offline,
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("You're offline")))
            .await,
            "no offline banner: {:?}",
            window.banner_showing()
        );
        let (_, button, progress) = window.banner_showing().expect("a banner");
        assert_eq!(button.as_deref(), Some("Retry now"));
        assert_eq!(progress, None, "no progress bar while offline");
        assert_eq!(window.sync_said(), "Offline");

        // A refused password wins over everything.
        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Failing {
                reason: FailureReason::Auth,
            },
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("Can't sign in")))
            .await,
            "no sign-in banner: {:?}",
            window.banner_showing()
        );
        let (title, button, _) = window.banner_showing().expect("a banner");
        assert!(
            title.contains("imap.example.com") && title.contains(&fixture.account.address.address),
            "the banner names the server and the address: {title}"
        );
        assert_eq!(button.as_deref(), Some("Update password\u{2026}"));
        assert_eq!(window.sync_said(), "Sync failed");

        // Back online, a pass finished: no banner, and when it synced.
        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account,
            done: 40,
            total: 40,
        });
        assert!(
            crate::settle_until(async || window.banner_showing().is_none()).await,
            "the banner stayed: {:?}",
            window.banner_showing()
        );
        assert!(
            window.sync_said().starts_with("Synced "),
            "the label says when it synced: {}",
            window.sync_said()
        );
    });
}
