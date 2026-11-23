use crate::support_compose::Window;
use crate::support_compose::settle;
use crate::support_compose::{fonts, style};
use gtk::gdk;

pub fn the_editing_surface_can_be_warmed_before_anyone_composes() {
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = window.composer();
    assert!(
        !composer.is_warm(),
        "a composer nobody has warmed cannot already be warm, or this proves nothing"
    );
    assert!(
        !composer.is_open(),
        "warming is not opening: the pane must stay where it is"
    );

    composer.warm();
    settle();

    assert!(composer.is_warm(), "warming it did not warm it");
    assert!(
        !composer.is_open(),
        "warming opened the composer over the reading pane"
    );

    // Idempotent: the scheduler need not know whether it ran, and a second load would discard a
    // draft typed meanwhile.
    composer.warm();
    settle();
    assert!(composer.is_warm());

    window.composer().open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();
    assert!(
        composer.is_open(),
        "the composer would not open after warming"
    );
    assert!(composer.is_warm(), "opening it un-warmed it");
}
