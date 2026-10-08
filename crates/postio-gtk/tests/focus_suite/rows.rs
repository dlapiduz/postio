//! The one-line row (T043; contracts/focus-surface.md, "Rows"): what it
//! draws, read back from what its last snapshot laid out -- what a person
//! sees, not what the row was handed.

use gtk::prelude::*;
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

/// The inbox opens at its top, the first day's heading on screen above the
/// first row, not scrolled one heading's height past it: GTK brings a
/// section's first row into view without its header, and a list that opens
/// with its heading hidden says nothing about what day it is (FR-010).
pub fn the_inbox_opens_with_its_first_heading_on_screen() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        // More than a screen, over several days, every third row marked:
        // the shape of a real inbox. The rows arrive as one-line
        // placeholders and grow when their page lands, and GTK lays the
        // list out again then -- the second place it hid the heading.
        let fixture = Fixture::empty().await;
        for step in 0..120_i64 {
            let (message, _) = fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    &format!("Message {step}"),
                    "A line of preview.",
                    1 + step * 60 * 3,
                )
                .await;
            if step % 3 == 0 {
                fixture.ask(message, "Can you approve it?").await;
            }
        }
        // A window the size a person's is, not the list's natural height.
        let window = postio_gtk::window::FocusWindow::new(None);
        window.set_default_size(1000, 640);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        let pane = window.pane().expect("the inbox");
        // Long enough for anything that follows the landing to have moved it.
        crate::settle();
        assert_eq!(
            pane.widget().vadjustment().value(),
            0.0,
            "the list opened scrolled past its first heading"
        );
        let heading = with_class(&window, "focus-day-heading")
            .into_iter()
            .next()
            .expect("a day heading");
        let top = heading
            .compute_point(pane.widget(), &gtk::graphene::Point::new(0.0, 0.0))
            .expect("the heading is inside the list");
        assert!(
            heading.is_mapped() && top.y() >= 0.0,
            "the first heading is not on screen: y = {}",
            top.y()
        );
    });
}

/// Every colour node in `node`'s tree that covers the whole `width` by
/// `height`: the ground a selected row is painted on.
fn grounds(node: &gtk::gsk::RenderNode, width: f32, height: f32, found: &mut u32) {
    use gtk::gsk::{ClipNode, ColorNode, ContainerNode, TransformNode};
    if let Some(color) = node.downcast_ref::<ColorNode>() {
        let at = color.bounds();
        if at.x() <= 0.5
            && at.y() <= 0.5
            && at.width() >= width - 0.5
            && at.height() >= height - 0.5
        {
            *found += 1;
        }
    } else if let Some(container) = node.downcast_ref::<ContainerNode>() {
        for index in 0..container.n_children() {
            grounds(&container.child(index), width, height, found);
        }
    } else if let Some(transform) = node.downcast_ref::<TransformNode>() {
        grounds(&transform.child(), width, height, found);
    } else if let Some(clip) = node.downcast_ref::<ClipNode>() {
        grounds(&clip.child(), width, height, found);
    }
}

/// How many full-row grounds `row` paints.
fn ground_of(row: &postio_gtk::list::RowWidget) -> u32 {
    let paintable = gtk::WidgetPaintable::new(Some(row));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(row.width()), f64::from(row.height()));
    let mut found = 0;
    if let Some(node) = snapshot.to_node() {
        grounds(&node, row.width() as f32, row.height() as f32, &mut found);
    }
    found
}

/// A row the selection reaches looks selected whatever it stands for: the
/// digest row `J` extends onto was counted in the bar ("2 selected") and
/// drawn exactly like an unselected row with the cursor on it, so a person
/// could not tell which two rows an Archive would hit.
pub fn a_selected_digest_row_is_drawn_selected() {
    use postio_gtk::list::{RowObject, RowWidget};
    use postio_ui::focus_list::{Digest, FocusRow};
    use postio_widgets::list_model::ModelRow;

    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let digest = FocusRow::Digest(Digest {
            delivery: postio_model::DeliveryId::new(1),
            rule: "Newsletters".to_owned(),
            cadence: None,
            count: 6,
            senders: Vec::new(),
            summary_line: None,
            at: chrono::Utc::now(),
        });
        let picked = postio_ui::selection::SelectionState::new();
        let row = RowWidget::default();
        row.set_selection(picked.clone());
        row.bind(&RowObject::with_contents(digest.clone()));
        let window = gtk::Window::new();
        window.set_default_size(900, 60);
        window.set_child(Some(&row));
        window.present();
        assert!(
            crate::settle_until(async || row.is_mapped() && row.width() > 0).await,
            "the row never reached the screen"
        );
        assert_eq!(ground_of(&row), 0, "an unselected digest row has no ground");

        picked.toggle(digest.id());
        // The window redraws its rows when the selection moves.
        row.queue_draw();
        assert!(
            crate::settle_until(async || row.drawn().picked).await,
            "the row never drew itself selected"
        );
        assert_eq!(
            ground_of(&row),
            1,
            "a selected digest row is painted on the selection's ground, as a conversation's is"
        );
    });
}
