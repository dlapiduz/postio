use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub fn work_deferred_to_the_first_frame_runs_even_if_the_window_is_up() {
    let early = gtk::Window::new();
    let ran_early = Rc::new(Cell::new(false));
    {
        let ran = Rc::clone(&ran_early);
        postio_widgets::startup::on_first_frame(&early, move || ran.set(true));
    }
    early.present();
    crate::support_reader::settle_until("the early hook to run", || ran_early.get());
    early.destroy();

    let late = gtk::Window::new();
    late.present();
    crate::support_reader::settle_until("the window to be mapped", || late.is_mapped());

    let ran_late = Rc::new(Cell::new(false));
    {
        let ran = Rc::clone(&ran_late);
        postio_widgets::startup::on_first_frame(&late, move || ran.set(true));
    }
    crate::support_reader::settle_until("the late hook to run", || ran_late.get());
    assert!(
        ran_late.get(),
        "work deferred to the first frame never ran, because the window was \
         already mapped and `map` does not fire twice. Startup defers opening \
         the account this way, so this failing means no account opens at all \
         (#1434)"
    );

    late.destroy();
}
