use crate::support_compose::settle;
use std::time::{Duration, Instant};

use crate::support_compose::Window;
use crate::support_compose::composer;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::{AccountId, Draft, DraftId};

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        glib::MainContext::default().iteration(true);
    }
}

pub fn typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    let saves: std::rc::Rc<std::cell::RefCell<Vec<Draft>>> = Default::default();
    composer.connect_save({
        let saves = std::rc::Rc::clone(&saves);
        move |draft| saves.borrow_mut().push(draft.clone())
    });

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();
    assert!(composer.is_open());

    for text in ["h", "he", "hel", "hell", "hello"] {
        composer.test_set_subject(text);
        settle();
    }
    assert!(
        saves.borrow().is_empty(),
        "autosave must not fire on every keystroke — that is what blocking would look like"
    );

    let fired = wait_until(Duration::from_secs(5), || !saves.borrow().is_empty());
    assert!(fired, "the debounced autosave never ran");
    assert_eq!(saves.borrow().last().unwrap().subject, "hello");
    saves.borrow_mut().clear();

    composer.test_set_subject("hello!");
    settle();
    assert!(
        saves.borrow().is_empty(),
        "the second edit's debounce has not elapsed yet"
    );
    // `Esc` keeps the draft: the flush below is what stands between "hello!" and nothing if the app
    // is killed.
    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(
        !saves.borrow().is_empty(),
        "closing must flush a pending autosave rather than leave it in a timer a crash could lose"
    );
    assert_eq!(saves.borrow().last().unwrap().subject, "hello!");
}

/// `connect_save` hands its handler `&mut Draft` and `save` writes back the id it assigns; without
/// that, each debounce tick would insert a fresh row instead of updating.
pub fn saving_twice_carries_the_assigned_id_forward_into_the_second_save() {
    let state_dir = std::env::temp_dir().join(format!(
        "postio-composer-autosave-id-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&state_dir).unwrap();

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    let seen_ids: std::rc::Rc<std::cell::RefCell<Vec<DraftId>>> = Default::default();
    composer.connect_save({
        let seen_ids = std::rc::Rc::clone(&seen_ids);
        move |draft| {
            if !draft.id.is_assigned() {
                draft.id = DraftId::new(42);
            }
            seen_ids.borrow_mut().push(draft.id);
        }
    });

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    // A composer with nothing in it and no id is not saved at all (#1608).
    composer.test_set_subject("first");
    settle();
    composer.save();
    composer.test_set_subject("revised");
    settle();
    composer.save();

    assert_eq!(
        *seen_ids.borrow(),
        vec![DraftId::new(42), DraftId::new(42)],
        "the second save should see the id the first save assigned, not start over"
    );
}
