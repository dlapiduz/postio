use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_render::{FallbackReason, Outcome};
use postio_widgets::body_view::BodyView;
use postio_widgets::reader::{Reader, RemoteImageAllowList};

use crate::support::{content, until};

/// The deadline and the result poll are both main-loop timers; a main thread busy past the deadline
/// found both due, ran the deadline first and replaced a finished render with the fallback.
///
/// POSTIO-MEASUREMENT: it sleeps two seconds so the main loop is late, which no fake
/// clock can stand in for under GTK, so it runs on the nightly timer.
pub fn a_finished_render_is_shown_when_the_main_loop_was_late() {
    let deadline = Duration::from_millis(200);
    let view = BodyView::new(deadline);
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    assert!(until(|| view.width() > 0), "the view was never allocated");

    view.set_content_from_top(content("plain-text-simple"));
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(2)));
    assert!(
        until(|| view.document().is_some()),
        "nothing was ever shown"
    );
    let outcome = view.document().expect("a snapshot").outcome;
    window.close();
    assert_eq!(
        outcome,
        Outcome::Rendered,
        "a render that had finished was replaced by the fallback"
    );
}

const WORDS: &str = "The survey starts Monday at the north gate, and the crew \
                     meets at seven with the long tapes, the level and enough \
                     water for the morning.\n\nBring the field books from last \
                     season; the corner stakes moved in the spring floods and \
                     every one of them needs checking against the old notes.";

fn reader(name: &str, treated: bool, width: i32) -> (gtk::Window, Reader) {
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        std::env::temp_dir().join(format!(
            "postio-fallback-{name}-{}.json",
            std::process::id()
        )),
    );
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&reader.widget())
        .build();
    reader.flow_in(&scroller);
    if treated {
        reader.use_treatments();
    }
    let window = gtk::Window::builder()
        .default_width(width)
        .default_height(700)
        .child(&scroller)
        .build();
    window.present();
    (window, reader)
}

fn drawn_words(reader: &Reader) -> Vec<(f64, f64, f64, f64)> {
    let document = reader.view().document().expect("a snapshot");
    let text = &document.text.text;
    let at = text
        .find("The survey starts")
        .unwrap_or_else(|| panic!("the words are not drawn: {text:?}"));
    let at = text[..at].chars().count();
    let mut rects: Vec<(f64, f64, f64, f64)> = document
        .text
        .clusters
        .iter()
        .filter(|c| c.range.start >= at && c.range.start < at + 120)
        .map(|c| (c.rect.x0, c.rect.y0, c.rect.x1, c.rect.y1))
        .collect();
    rects.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    rects
}

/// Compared glyph for glyph against the same words sent as plain text: same column edges, face,
/// size and line rhythm.
pub fn a_fallback_is_laid_out_as_a_plain_text_body() {
    for treated in [true, false] {
        let (window, reader) = reader("layout", treated, 560);
        let sent_as_text = MessageBody {
            text: Some(WORDS.to_owned()),
            html: None,
        };
        reader.render(&sent_as_text, Some("ada@example.com"));
        assert!(
            until(|| reader.view().document().is_some_and(
                |d| d.outcome == Outcome::Rendered && d.text.text.contains("The survey starts")
            )),
            "the plain-text message was never drawn"
        );
        let plain = drawn_words(&reader);

        reader.view().hold_renders();
        reader.render(
            &MessageBody {
                text: Some(WORDS.to_owned()),
                html: Some(format!(
                    "<table style=\"background:#0b3d2e\"><tr><td style=\"color:#f5f0e1\">{}</td></tr></table>",
                    WORDS.replace("\n\n", "<br><br>")
                )),
            },
            Some("ada@example.com"),
        );
        assert!(
            until(|| reader
                .view()
                .document()
                .is_some_and(|d| d.outcome == Outcome::FellBack(FallbackReason::Deadline))),
            "the held render never fell back"
        );
        let fallback = drawn_words(&reader);
        let said = reader
            .view()
            .document()
            .expect("a snapshot")
            .text
            .text
            .clone();
        reader.view().release_renders();
        window.close();

        // Never a silent swap: with no line above the body to say so, the document does.
        assert_eq!(
            said.contains("Shown as plain text: this message took too long to lay out."),
            !treated,
            "treated {treated}: the notice is where it should not be, or missing: {said:?}"
        );

        assert_eq!(
            fallback.len(),
            plain.len(),
            "treated {treated}: the fallback draws the words in a different number of glyphs"
        );
        let shift = fallback[0].1 - plain[0].1;
        if treated {
            assert!(
                shift.abs() < 0.5,
                "the fallback's words start {shift}px lower than a plain body's"
            );
        }
        for (got, want) in fallback.iter().zip(&plain) {
            assert!(
                (got.0 - want.0).abs() < 0.5
                    && (got.2 - want.2).abs() < 0.5
                    && (got.1 - shift - want.1).abs() < 0.5
                    && (got.3 - shift - want.3).abs() < 0.5,
                "treated {treated}: a glyph of the fallback is at {got:?}, \
                 where a plain-text body draws it at {want:?} (shifted {shift})"
            );
        }
    }
}

/// The body carries no notice of its own, so the render-mode line is the only place it says so.
pub fn the_render_mode_line_says_a_body_fell_back() {
    let (window, reader) = reader("line", true, 700);
    reader.view().hold_renders();
    reader.render(
        &MessageBody {
            text: Some(WORDS.to_owned()),
            html: Some(format!("<p>{WORDS}</p>")),
        },
        Some("ada@example.com"),
    );
    assert!(
        until(|| reader
            .view()
            .document()
            .is_some_and(|d| d.outcome == Outcome::FellBack(FallbackReason::Deadline))),
        "the held render never fell back"
    );
    let line = reader
        .render_mode_line()
        .expect("a treated reader has a line");
    let said = line.text();
    let text = reader
        .view()
        .document()
        .expect("a snapshot")
        .text
        .text
        .clone();
    reader.view().release_renders();
    window.close();
    assert_eq!(said, "Plain text · this message took too long to lay out");
    assert!(
        !text.contains("Shown as plain text"),
        "the body says it fell back as well as the line: {text:?}"
    );
}
