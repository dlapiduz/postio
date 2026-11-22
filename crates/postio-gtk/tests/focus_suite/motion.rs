//! No transition in Focus runs longer than 100 ms, and with `gtk-enable-animations` off nothing
//! animates.
//! libadwaita's own dialog and toast motion cannot be read; it follows that setting.

use gtk::prelude::*;
use postio_core::{ConnectionState, Event};

use crate::support::{self, Fixture};

fn over_budget(root: &gtk::Widget) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(revealer) = widget.downcast_ref::<gtk::Revealer>()
            && revealer.transition_type() != gtk::RevealerTransitionType::None
            && revealer.transition_duration() > 100
        {
            found.push(format!(
                "a revealer in {} animates for {} ms",
                describe(&widget),
                revealer.transition_duration()
            ));
        }
        if let Some(pages) = widget.downcast_ref::<gtk::Stack>()
            && pages.transition_type() != gtk::StackTransitionType::None
            && pages.transition_duration() > 100
        {
            found.push(format!(
                "a stack in {} animates for {} ms",
                describe(&widget),
                pages.transition_duration()
            ));
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    found
}

fn describe(widget: &gtk::Widget) -> String {
    let mut up = Some(widget.clone());
    while let Some(current) = up {
        let classes = current.css_classes();
        if !classes.is_empty() {
            return format!("{} ({})", current.type_().name(), classes.join(" "));
        }
        up = current.parent();
    }
    widget.type_().name().to_owned()
}

fn durations(css: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for line in css.lines() {
        let line = line.trim();
        if line.starts_with("/*") || line.starts_with('*') {
            continue;
        }
        if !(line.contains("transition") || line.contains("animation")) {
            continue;
        }
        for token in line.split([' ', ',', ':', ';']) {
            if let Some(ms) = token.strip_suffix("ms")
                && let Ok(value) = ms.parse::<f64>()
            {
                out.push((line.to_owned(), value));
            } else if let Some(seconds) = token.strip_suffix('s')
                && let Ok(value) = seconds.parse::<f64>()
            {
                out.push((line.to_owned(), value * 1000.0));
            }
        }
    }
    out
}

pub fn no_transition_outruns_the_budget_and_reduced_motion_stills_them() {
    crate::gtk_case(async {
        for (name, css) in [
            ("focus.css", postio_gtk::style::SURFACES),
            ("focus-colours.css", postio_gtk::style::COLOURS),
            (
                "widgets.css",
                include_str!("../../../postio-widgets/data/widgets.css"),
            ),
        ] {
            for (line, ms) in durations(css) {
                assert!(ms <= 100.0, "{name}: {ms} ms is over the budget: {line}");
            }
        }

        let fixture = Fixture::empty().await;
        for (subject, minutes) in [("Budget", 5), ("Harbor draft", 9)] {
            let (message, _) = fixture.file_from_ada(subject, "Hello.", minutes).await;
            fixture.write_body(message, "Hello.").await;
        }
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        support::rows_shown(&window, 2).await;
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Offline,
        }));
        assert!(crate::settle_until(async || window.banner_showing().is_some()).await);
        let mut problems = over_budget(window.upcast_ref());
        support::keys(&window, &["j", "b"]);
        let picker = window.open_picker().expect("b opened a picker");
        assert!(crate::settle_until(async || picker.is_shown()).await);
        problems.extend(over_budget(picker.popover().upcast_ref()));
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        assert!(crate::settle_until(async || reading.is_open()).await);
        problems.extend(over_budget(reading.dialog().upcast_ref()));
        problems.sort();
        problems.dedup();
        assert!(
            problems.is_empty(),
            "transitions over the 100 ms budget:\n  {}",
            problems.join("\n  ")
        );

        let settings = gtk::Settings::default().expect("GTK settings");
        let animated = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            !reading.is_open(),
            "with reduced motion, the dialog is gone at once"
        );
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Online,
        }));
        crate::settle();
        let revealers_moving = {
            let mut moving = Vec::new();
            let mut stack = vec![window.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = stack.pop() {
                if let Some(revealer) = widget.downcast_ref::<gtk::Revealer>()
                    && revealer.reveals_child() != revealer.is_child_revealed()
                {
                    moving.push(describe(&widget));
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    child = next.next_sibling();
                    stack.push(next);
                }
            }
            moving
        };
        settings.set_gtk_enable_animations(animated);
        assert!(
            revealers_moving.is_empty(),
            "with reduced motion, still moving: {revealers_moving:?}"
        );
    });
}

fn rule_body<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let head = rest[..open]
            .rsplit('}')
            .next()
            .and_then(|head| head.rsplit("*/").next())
            .unwrap_or("")
            .trim();
        let close = rest[open..].find('}')? + open;
        if head == selector {
            return Some(&rest[open + 1..close]);
        }
        rest = &rest[close + 1..];
    }
    None
}

pub fn the_list_rows_cursor_has_no_transition() {
    let css = postio_gtk::style::SURFACES;
    let row = rule_body(css, ".focus-list > row").expect("the list row's rule");
    assert!(
        row.lines()
            .any(|line| line.trim().trim_end_matches(';').replace("  ", " ") == "transition: none"),
        "the list row must say `transition: none`; it says: {row}"
    );
    for state in [
        ".focus-list > row:selected",
        ".focus-list > row:focus-visible",
    ] {
        let body = rule_body(css, state).expect("the cursor's rule");
        assert!(
            !body.contains("transition") || body.contains("transition: none"),
            "{state} transitions: {body}"
        );
    }
}
