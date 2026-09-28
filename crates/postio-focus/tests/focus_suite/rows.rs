//! The one-line row (T043; contracts/focus-surface.md, "Rows"): what it
//! draws, read back from what its last snapshot laid out -- what a person
//! sees, not what the row was handed.

use postio_ui::label_colour::{ACCENT_BAND, Rgb, hue_distance};

use crate::support::{self, Fixture, with_class};

/// US1 scenario 2: the row shows the subject and the first line exactly as
/// they arrived -- nothing rewritten, summarised or scored (FR-011).
pub fn a_row_shows_the_subject_and_first_line_exactly_as_they_arrived() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "RE: Q3 numbers!!",
                "Hi all \u{2014} the numbers are in the sheet.",
                5,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let row = window.pane().expect("the inbox").rows_on_screen()[0].clone();
        assert!(
            crate::settle_until(async || !row.drawn().texts.is_empty()).await,
            "the row was never drawn"
        );
        let drawn = row.drawn();
        for said in [
            "Ada Moreno",
            "RE: Q3 numbers!!",
            "Hi all \u{2014} the numbers are in the sheet.",
        ] {
            assert!(
                drawn.texts.iter().any(|text| text == said),
                "the row draws {said:?} verbatim: it drew {:?}",
                drawn.texts
            );
        }
        assert!(drawn.bold, "an unread conversation is drawn bold");
    });
}

/// US1 scenario 8: three labels draw two pills, and neither is in the
/// accent's hue (FR-012, FR-091).
pub fn a_third_label_draws_no_third_pill_and_none_is_the_accent() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor API draft v3",
                "Uploaded v3 with the pagination changes.",
                5,
            )
            .await;
        fixture.label(message, &["Harbor", "Atlas", "Home"]).await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || !pane.rows_on_screen()[0].drawn().pills.is_empty()).await,
            "the row never drew its pills"
        );
        let drawn = pane.rows_on_screen()[0].drawn();
        let names: Vec<&str> = drawn.pills.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["Harbor", "Atlas"],
            "two pills, the first two labels; the open message shows them all"
        );
        let accent = adw::StyleManager::default().accent_color_rgba();
        let accent = Rgb::new(
            (accent.red() * 255.0) as u8,
            (accent.green() * 255.0) as u8,
            (accent.blue() * 255.0) as u8,
        );
        for (name, colour) in &drawn.pills {
            let colour = Rgb::new(
                (colour.red() * 255.0) as u8,
                (colour.green() * 255.0) as u8,
                (colour.blue() * 255.0) as u8,
            );
            assert!(
                hue_distance(colour.hue(), accent.hue()) >= ACCENT_BAND,
                "{name}'s pill is {colour:?}, within the accent's hue band ({accent:?})"
            );
        }
    });
}

/// Rows sit under their day's heading: "Today · Saturday 26 September", or
/// the day it was (FR-010).
pub fn rows_sit_under_their_day_s_heading() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let day = (support::now() - chrono::Duration::minutes(5))
            .with_timezone(&chrono::Local)
            .date_naive();
        let expected = postio_ui::focus_row::day_heading(day, chrono::Local::now().date_naive());
        assert!(
            crate::settle_until(async || {
                with_class(&window, "focus-day-heading")
                    .iter()
                    .any(|heading| support::texts(heading).contains(&expected))
            })
            .await,
            "no heading reads {expected:?}: {:?}",
            with_class(&window, "focus-day-heading")
                .iter()
                .map(support::texts)
                .collect::<Vec<_>>()
        );
    });
}
