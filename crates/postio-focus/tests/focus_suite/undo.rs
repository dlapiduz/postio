//! US1 scenario 3's second half: the toast says what happened, and one
//! Ctrl+Z returns all three -- including after the toast has gone, because
//! the toast going is not the undo window closing (FR-041).

use crate::support::{self, Fixture};

pub fn one_ctrl_z_returns_all_three_after_the_toast_has_gone() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j", "x", "j", "x", "j", "x", "j", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Fourth", "Fifth"]).await,
            "the three never left: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || {
                window.toast_showing().as_deref() == Some("Archived 3 messages")
            })
            .await,
            "the toast says what happened: {:?}",
            window.toast_showing()
        );

        // The toast goes -- it would on its own after eight seconds -- and
        // the undo does not go with it.
        window.dismiss_toast();
        crate::settle();
        assert_eq!(window.toast_showing(), None);

        support::press(&window, "z", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 5).await,
            "one Ctrl+Z returns all three: {:?}",
            support::subjects(&window)
        );
        assert_eq!(
            support::subjects(&window),
            ["First", "Second", "Third", "Fourth", "Fifth"],
            "each back where it was"
        );
    });
}
