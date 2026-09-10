//! Asking the rendering engine what a document actually means.
//!
//! Two questions a test can put to WebKit: [`computed`] for a cascade
//! ("what colour is this element"), and [`measure`] for anything else,
//! including hit tests ("what is drawn at this point").
//!
//! Shared by `gtk_reader.rs` and `gtk_suite` through `#[path]` rather than
//! copied into each, because the interesting part is not the question — it is
//! the lifecycle discipline around it (#794), and two copies of that is one
//! copy that will be missing a step.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use webkit6::prelude::*;

/// A flag that flips once `reader`'s `WebView` finishes its current load —
/// success or failure both count, since a `load-failed` is still "done" for
/// the purpose of "stop pumping and check the result".
/// What a rendering engine actually computes for `selector`'s `property`,
/// given `document`.
///
/// The string assertions elsewhere prove the right CSS reached the document.
/// They cannot prove the CSS *applies* — a selector that loses on specificity,
/// or a custom property that does not inherit where it was assumed to, writes
/// exactly the same text into exactly the same place and paints nothing. So
/// this asks the engine.
///
/// A scratch `WebView` with JavaScript **on**, and deliberately not the
/// reader's: the reading pane runs with script off by construction (ADR 0003)
/// and must keep doing so, which is precisely why a test cannot interrogate
/// it and needs an instrument of its own. Nothing sender-authored is ever
/// loaded here — the argument is a document Postio composed.
pub fn computed(document: &str, selector: &str, property: &str) -> String {
    computed_pseudo(document, selector, "", property)
}

/// As [`computed`], for a pseudo-element.
///
/// `querySelector` cannot return one -- there is no element to return -- so a
/// selector ending in `::before` silently matches nothing and every property
/// comes back as the empty string, which reads exactly like "this rule does
/// not apply". `getComputedStyle` takes the pseudo as its second argument
/// instead, and that is the only difference between the two.
///
/// Pass `""` for a real element.
pub fn computed_pseudo(document: &str, selector: &str, pseudo: &str, property: &str) -> String {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(true);

    // Weakly held past the end, because a `WebView` still alive at `exit()`
    // is #794: WebKit reports `WebProcess didn't exit as expected after the
    // UI process connection was closed` and the binary dies on the way out,
    // *after* reporting every test as passed. An instrument that leaks one
    // per call would reintroduce exactly that, so this releases it and says
    // so — the same mechanism `gtk_reader_teardown` asserts, at the one place
    // in this binary that builds views of its own.
    let (value, weak) = {
        // Its own ephemeral session and context, dropped with the view: the
        // default `WebContext` is process-global and its WebProcess outlives
        // every scope this helper has.
        let network_session = webkit6::NetworkSession::new_ephemeral();
        let context = webkit6::WebContext::new();
        let view = webkit6::WebView::builder()
            .settings(&settings)
            .web_context(&context)
            .network_session(&network_session)
            .build();
        let window = gtk::Window::new();
        window.set_child(Some(&view));
        window.present();

        let loaded = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&loaded);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        view.load_html(document, None);
        wait_for(&loaded, Duration::from_secs(5));

        let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        view.evaluate_javascript(
            &format!(
                "getComputedStyle(document.querySelector('{selector}'), '{pseudo}')\
                 .getPropertyValue('{property}')"
            ),
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                *slot.borrow_mut() = Some(
                    outcome
                        .map(|value| value.to_str().to_string())
                        .unwrap_or_default(),
                );
            },
        );
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
        while answer.borrow().is_none() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        let value = answer.borrow_mut().take().unwrap_or_default();

        let weak = view.downgrade();
        window.set_child(None::<&gtk::Widget>);
        window.destroy();
        (value, weak)
    };
    // GTK finalizes on the main loop, not at the closing brace.
    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        weak.upgrade().is_none(),
        "the measuring WebView outlived its window, so its WebProcess is \
         still attached at exit -- #794 all over again"
    );
    value
}

/// Evaluate `expression` against `document` and return it as a string.
///
/// [`computed`]'s twin, for questions that are not a computed style. Same
/// lifecycle discipline, and the same reason for it (#794).
pub fn measure(document: &str, expression: &str) -> String {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(true);

    let (value, weak) = {
        let network_session = webkit6::NetworkSession::new_ephemeral();
        let context = webkit6::WebContext::new();
        let view = webkit6::WebView::builder()
            .settings(&settings)
            .web_context(&context)
            .network_session(&network_session)
            .build();
        let window = gtk::Window::new();
        window.set_default_size(600, 500);
        window.set_child(Some(&view));
        window.present();

        let loaded = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&loaded);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        view.load_html(document, None);
        wait_for(&loaded, Duration::from_secs(5));
        // Layout has to have happened before a width means anything.
        pump_for(Duration::from_millis(200));

        let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);
        view.evaluate_javascript(
            expression,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                *slot.borrow_mut() = Some(
                    outcome
                        .map(|value| value.to_str().to_string())
                        .unwrap_or_default(),
                );
            },
        );
        let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
        while answer.borrow().is_none() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        let value = answer.borrow_mut().take().unwrap_or_default();

        let weak = view.downgrade();
        window.set_child(None::<&gtk::Widget>);
        window.destroy();
        (value, weak)
    };
    for _ in 0..200 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        weak.upgrade().is_none(),
        "the measuring WebView outlived its window -- #794 all over again"
    );
    value
}

/// Pump the main loop until `flag` is set, or the deadline passes.
fn wait_for(flag: &Rc<RefCell<bool>>, timeout: Duration) {
    let deadline = Instant::now() + postio_test_support::scaled(timeout);
    while !*flag.borrow() && Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(*flag.borrow(), "the WebView never finished loading");
}

/// Iterate the main loop for `duration`, so GTK finalizes what a scope
/// dropped. The closing brace releases a reference; the loop is what acts on
/// it.
fn pump_for(duration: Duration) {
    let deadline = Instant::now() + postio_test_support::scaled(duration);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}
