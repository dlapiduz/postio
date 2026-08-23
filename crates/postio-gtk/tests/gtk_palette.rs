//! The `Ctrl+K` palette on a real display: opening, filtering, choosing, and
//! the budget it all has to fit inside.
//!
//! One test function, in order, for the reason `gtk_style.rs` gives: GTK is
//! single-threaded and initialised once. Without a display it skips. Nothing
//! here touches the network.
//!
//! The ranking and the context filter are unit-tested in `src/palette.rs` with
//! no display at all; what needs a display is the widget around them — that the
//! rows are really built, that `Ctrl+K` really opens it, and that Enter really
//! reaches a handler.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context, Keymap};
use postio_gtk::palette::Palette;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

/// The interaction budget from CLAUDE.md. Generous multiples of it are used
/// below: this is a smoke test against an accidental O(n·m), not a benchmark —
/// `cargo bench` owns the real numbers.
const INTERACTION_BUDGET: std::time::Duration = std::time::Duration::from_millis(16);

fn defaults() -> Keymap {
    Keymap::resolve(&postio_config::KeyBindings::default())
}

/// Runs the main loop until everything queued has been done.
fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn the_palette_opens_filters_and_runs() {
    // Before any glib call: `g_get_user_state_dir` caches its answer, and a
    // test has no business writing into the developer's real state directory.
    let state_dir = std::env::temp_dir().join(format!("postio-palette-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the widget on its own ─────────────────────────────────────────────
    let palette = Palette::new();
    palette.set_keymap(defaults());
    palette.set_context(Context::List);
    settle();

    let everything = palette.visible();
    assert!(
        everything.contains(&CommandId::Archive),
        "an empty query lists what the context allows"
    );
    assert!(
        !everything.contains(&CommandId::Send),
        "and only what it allows"
    );

    palette.set_query("arch");
    settle();
    assert_eq!(
        palette.visible().first(),
        Some(&CommandId::Archive),
        "the best match leads: {:?}",
        palette.visible()
    );
    assert_eq!(
        palette.selected(),
        Some(CommandId::Archive),
        "and it is selected, so Enter runs it without touching an arrow key"
    );

    palette.move_selection(1);
    settle();
    assert_eq!(
        palette.selected(),
        Some(CommandId::ArchiveThread),
        "Down walks the list while the cursor stays in the entry"
    );
    palette.move_selection(-5);
    settle();
    assert_eq!(
        palette.selected(),
        Some(CommandId::Archive),
        "and it stops at the top rather than wrapping"
    );

    // ── choosing runs the command ─────────────────────────────────────────
    let chosen: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    palette.connect_activated({
        let chosen = Rc::clone(&chosen);
        move |id| chosen.borrow_mut().push(id)
    });
    palette.activate_selected();
    settle();
    assert_eq!(*chosen.borrow(), vec![CommandId::Archive]);

    // ── a query that matches nothing ──────────────────────────────────────
    palette.set_query("zzzzzz");
    settle();
    assert!(palette.visible().is_empty());
    assert_eq!(palette.selected(), None, "and Enter does nothing");
    palette.activate_selected();
    assert_eq!(*chosen.borrow(), vec![CommandId::Archive], "still just one");

    // ── in the window, over the workspace ─────────────────────────────────
    let window = Window::default();
    window.present();
    settle();

    assert!(
        !window.palette().is_visible(),
        "the palette is not in the way until it is asked for"
    );

    let ran: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_command({
        let ran = Rc::clone(&ran);
        move |id| ran.borrow_mut().push(id)
    });

    // Ctrl+K, through the resolver, exactly as the controller delivers it.
    assert_eq!(
        window.handle_key(
            gdk::Key::from_name("k").unwrap(),
            gdk::ModifierType::CONTROL_MASK
        ),
        glib::Propagation::Stop,
        "the palette's key is consumed rather than reaching the workspace"
    );
    settle();
    assert!(window.palette().is_visible(), "Ctrl+K opens it");
    assert_eq!(
        window.palette().query(),
        "",
        "it opens empty: a palette showing the last search has to be cleared first"
    );

    // Picking a row runs the command and closes the overlay.
    window.palette().set_query("compose");
    settle();
    window.palette().activate_selected();
    settle();
    assert_eq!(*ran.borrow(), vec![CommandId::Compose]);
    assert!(
        !window.palette().is_visible(),
        "and the workspace gets the keyboard back"
    );

    // Escape closes it without running anything.
    window.handle_key(
        gdk::Key::from_name("k").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    settle();
    assert!(window.palette().is_visible());
    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(!window.palette().is_visible(), "Esc closes the palette");
    assert_eq!(*ran.borrow(), vec![CommandId::Compose], "and runs nothing");

    // An ordinary key the workspace should keep.
    assert_eq!(
        window.handle_key(
            gdk::Key::from_name("z").unwrap(),
            gdk::ModifierType::empty()
        ),
        glib::Propagation::Proceed,
        "an unbound key is left for the widget underneath"
    );

    // ── the palette follows a rebind ──────────────────────────────────────
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_owned(), "y".to_owned());
    window.apply_keymap(Keymap::resolve(&overrides));
    settle();

    window.open_palette();
    window.palette().set_query("archive");
    settle();
    let listed =
        postio_gtk::palette::entries(&Keymap::resolve(&overrides), Context::List, "archive");
    assert_eq!(
        listed
            .iter()
            .find(|entry| entry.id == CommandId::Archive)
            .and_then(|entry| entry.binding.as_deref()),
        Some("y"),
        "a rebind reaches the palette with no code edit"
    );
    window.close_palette();
    settle();

    // ── the budget ────────────────────────────────────────────────────────
    // Rebuilding the whole list on every keystroke is only defensible if it is
    // genuinely cheap. Ten rebuilds, well inside one frame each.
    let start = Instant::now();
    for query in [
        "a", "ar", "arc", "arch", "archi", "", "c", "co", "com", "comp",
    ] {
        palette.set_query(query);
        settle();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < INTERACTION_BUDGET * 10,
        "ten palette rebuilds took {elapsed:?}, over ten frames"
    );

    let start = Instant::now();
    window.open_palette();
    settle();
    let opened = start.elapsed();
    assert!(
        opened < INTERACTION_BUDGET * 4,
        "opening the palette took {opened:?}"
    );
    window.close_palette();

    window.close();
    settle();
}
