//! The editing surface is drawn in the application's colours (spec 002,
//! FR-073/FR-074), against a live WebKit.
//!
//! `postio-ui`'s own unit tests prove the *document* carries a sheet and that
//! the ground resolves in both schemes. They cannot prove the thing that was
//! actually wrong: the surface rendered in the engine's defaults, so a dark
//! application had a white page in the middle of it — and for the interval
//! before the document parsed, a white *frame* even once the sheet was right.
//! Both need a display and a real engine, which is why this file exists.
//!
//! What is asserted is the widget's own background and the computed style of
//! the loaded document, not a screenshot. A screenshot here would be a
//! comparison against the accelerated path this runner does not use (#1307),
//! and would fail for reasons that have nothing to do with the colours.
//!
//! One test function: GTK is single-threaded and initialised once.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::editor;
use postio_gtk::reader::scheme::BlobSource;
use webkit6::prelude::*;

use crate::{settle, settle_until};

/// The composer opens with no inline parts to resolve, which is the ordinary
/// case and all this test needs.
struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

/// The document's computed style for `property` on `selector`.
fn computed(view: &webkit6::WebView, selector: &str, property: &str) -> String {
    let script = format!(
        "getComputedStyle(document.querySelector('{selector}')).getPropertyValue('{property}')"
    );
    let answer = Rc::new(RefCell::new(String::new()));
    let done = Rc::new(RefCell::new(false));
    view.evaluate_javascript(
        &script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        glib::clone!(
            #[strong]
            answer,
            #[strong]
            done,
            move |result| {
                if let Ok(value) = result {
                    *answer.borrow_mut() = value.to_str().to_string();
                }
                *done.borrow_mut() = true;
            }
        ),
    );
    settle_until("the document to answer", || *done.borrow());
    answer.borrow().clone()
}

/// `rgb(43, 43, 45)` and friends, as the three numbers.
fn channels(colour: &str) -> Option<(u32, u32, u32)> {
    let inside = colour.split_once('(')?.1.split_once(')')?.0;
    let mut parts = inside
        .split([',', ' ', '/'])
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.trim().parse::<f32>().ok());
    Some((
        parts.next()? as u32,
        parts.next()? as u32,
        parts.next()? as u32,
    ))
}

pub fn the_editing_surface_is_dark_in_dark_mode_and_never_white() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    settle();

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let view = editor::editing_view(Rc::new(NoBlobs) as Rc<dyn BlobSource>);
    window.set_child(Some(&view));
    window.present();
    settle();

    // ── Before any document: the widget itself must already be dark ───────
    //
    // This is the white flash, and it is not a document question at all --
    // between one load and the next there is no document to paint from, so
    // the widget's own background is the only thing on screen. The reader
    // learned this as `paint_ground`; the editor had never been told.
    let ground = view.background_color();
    let luminance = 0.2126 * ground.red() + 0.7152 * ground.green() + 0.0722 * ground.blue();
    assert!(
        ground.alpha() > 0.0,
        "the editing view paints no ground at all, so the frame before the \
         first paint is whatever the toolkit left there"
    );
    assert!(
        luminance < 0.5,
        "the editing view's own ground is light while the application is \
         dark: {ground:?}"
    );

    // ── With a document: the page and its text follow the same palette ────
    let loaded = Rc::new(RefCell::new(false));
    let loads = Rc::new(RefCell::new(0u32));
    view.connect_load_changed({
        let loaded = loaded.clone();
        let loads = loads.clone();
        move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *loaded.borrow_mut() = true;
                *loads.borrow_mut() += 1;
            }
        }
    });
    editor::seed(&view, "<p>Morning.</p>");
    settle_until("the editing shell to load", || *loaded.borrow());

    let background = computed(&view, "body", "background-color");
    let (br, bg, bb) = channels(&background)
        .unwrap_or_else(|| panic!("not a colour this test can read: {background:?}"));
    assert!(
        br < 128 && bg < 128 && bb < 128,
        "the page is light inside a dark application: {background:?}"
    );

    let colour = computed(&view, "body", "color");
    let (cr, cg, cb) =
        channels(&colour).unwrap_or_else(|| panic!("not a colour this test can read: {colour:?}"));
    assert!(
        cr > 128 && cg > 128 && cb > 128,
        "the text is dark on a dark page, which is the failure that looks \
         like nothing rendered at all: {colour:?}"
    );

    // The typeface, which is the other half of "drawn in the application's
    // colours" and the half a ground check cannot see.
    let family = computed(&view, "body", "font-family");
    assert!(
        family.contains("Barlow"),
        "the surface is not using the application's typeface: {family:?}"
    );

    // ── A scheme change restyles, and must not reload ────────────────────
    //
    // FR-075. Reloading would be the easy way to get the colours right and
    // would take the caret and the whole undo history with it -- which is the
    // kind of fix that is invisible in a screenshot and infuriating in use.
    // So what is asserted is not only that the colours followed, but that the
    // document they followed in is the same one.
    let loads_before = *loads.borrow();
    view.evaluate_javascript(
        "document.body.setAttribute('data-witness', 'still here')",
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        |_| {},
    );
    settle();

    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    settle_until("the light sheet to apply", || {
        channels(&computed(&view, "body", "background-color"))
            .is_some_and(|(r, g, b)| r > 128 && g > 128 && b > 128)
    });

    assert_eq!(
        *loads.borrow(),
        loads_before,
        "the scheme change reloaded the document, which loses the caret and \
         the undo history"
    );
    assert_eq!(
        computed(&view, "body", "data-witness-probe"),
        "",
        "sanity: an unknown property reads empty, so the check below means \
         something"
    );
    let witness = {
        let answer = Rc::new(RefCell::new(String::new()));
        let done = Rc::new(RefCell::new(false));
        view.evaluate_javascript(
            "document.body.getAttribute('data-witness') || ''",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            glib::clone!(
                #[strong]
                answer,
                #[strong]
                done,
                move |result| {
                    if let Ok(value) = result {
                        *answer.borrow_mut() = value.to_str().to_string();
                    }
                    *done.borrow_mut() = true;
                }
            ),
        );
        settle_until("the witness to answer", || *done.borrow());
        answer.borrow().clone()
    };
    assert_eq!(
        witness, "still here",
        "the DOM was rebuilt by the scheme change -- everything in the \
         document, caret and undo included, went with it"
    );

    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
}
