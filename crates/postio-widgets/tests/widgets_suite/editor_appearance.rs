use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_widgets::composer::editor;
use postio_widgets::reader::BlobSource;
use webkit6::prelude::*;

use crate::support_compose::{settle, settle_until};

struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

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
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    settle();

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let view = editor::editing_view(Rc::new(NoBlobs) as Rc<dyn BlobSource>);
    window.set_child(Some(&view));
    window.present();
    settle();

    // The white flash: between loads there is no document to paint from, so the widget's own
    // background is all that shows.
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

    let family = computed(&view, "body", "font-family");
    assert!(
        family.contains("Barlow"),
        "the surface is not using the application's typeface: {family:?}"
    );

    // A scheme change restyles and must not reload: reloading would lose the caret and the whole
    // undo history.
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
