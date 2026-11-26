use std::time::Instant;

use adw::prelude::*;
use gtk::gdk;
use postio_ui::source::{BodyState, RawState, SourceInput, Tab};
use postio_widgets::source::SourceDialog;

const RAW: &[u8] = b"From: Ada <ada@example.com>\r\n\
Subject: Harbor\r\n\
X-Long: one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twenty-one twenty-two twenty-three\r\n\
\r\n\
Hello\r\n";

fn settle_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + postio_test_support::patience();
    while !done() && Instant::now() < deadline {
        context.iteration(false);
    }
}

fn headers_text(widget: &gtk::Widget) -> Option<gtk::TextView> {
    if let Some(text) = widget.downcast_ref::<gtk::TextView>()
        && text
            .buffer()
            .text(
                &text.buffer().start_iter(),
                &text.buffer().end_iter(),
                false,
            )
            .contains("X-Long")
    {
        return Some(text.clone());
    }
    let mut child = widget.first_child();
    while let Some(each) = child {
        if let Some(found) = headers_text(&each) {
            return Some(found);
        }
        child = each.next_sibling();
    }
    None
}

pub fn a_long_header_value_wraps_under_its_own_column_not_under_the_names() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = postio_ui::source::view(SourceInput {
        subject: "Harbor",
        from: &[],
        headers: None,
        envelope: None,
        raw: Some(RAW.to_vec()),
        raw_state: RawState::Here,
        body: BodyState::Here,
        provider: None,
        sent_by_you: false,
        account: "Work",
        signature: None,
    });
    let keymap = postio_core::Keymap::resolve(&postio_config::KeyBindings::default());
    let dialog = SourceDialog::new(view, &keymap, |_| {});
    let window = gtk::Window::new();
    window.set_default_size(720, 600);
    window.present();
    dialog.dialog().present(Some(&window));
    dialog.show_tab(Tab::Headers);
    let text = headers_text(dialog.dialog().upcast_ref()).expect("the Headers tab's text");
    settle_until(|| text.is_mapped() && text.width() > 0 && text.height() > 0);
    let buffer = text.buffer();
    let all = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let name_at = all.find("X-Long").expect("the long header's name");
    let value_at = all.find("one two").expect("its value");
    let x = |offset: usize| {
        let iter = buffer.iter_at_offset(i32::try_from(all[..offset].chars().count()).unwrap());
        (text.iter_location(&iter).x(), iter)
    };
    let (name_x, _) = x(name_at);
    let (value_x, mut value) = x(value_at);
    assert!(
        text.forward_display_line(&mut value),
        "a value this long wraps in a 720 px window"
    );
    let wrapped_x = text.iter_location(&value).x();
    assert_eq!(
        name_x,
        text.left_margin(),
        "a header's name starts at the tab's edge, with no blank column before it"
    );
    assert!(
        value_x > name_x,
        "the value starts in its own column, right of the name ({value_x} vs {name_x})"
    );
    assert_eq!(
        wrapped_x, value_x,
        "the value's next line starts under the value, not back under the names"
    );
    window.close();
}
