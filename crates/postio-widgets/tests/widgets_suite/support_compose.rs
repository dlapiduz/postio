//! What the composer, editor, onboarding and settings cases share: the
//! main-loop helpers every GTK suite writes, the editor's script markers, and
//! a window that hosts a composer with nothing of either app in it.
//!
//! The composer is the shared widget (ADR 0043) and runs on any
//! [`ComposerHost`]. [`Window`] is the smallest host that still has the
//! three things a composer's cases lean on: a keyboard that resolves keys
//! through the real keymap and broadcasts the command to the composer
//! (`handle_key`), a keyboard context the composer takes and gives back
//! (`context`), and the `win.compose` action. Nothing else of an app window
//! is here -- no sidebar, no list, no reading pane's occupants.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use postio_widgets::composer::{Composer, ComposerHost, Field};

// -- Main-loop helpers --------------------------------------------------------

/// Drain the main loop until nothing is pending.
pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// Turn the main loop a fixed number of times, draining it each time. A count
/// is a guess: prefer [`settle_until`] where the case knows what it waits for.
pub fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.iteration(false) {}
    }
}

/// Run the main loop until `done`, failing with `what` at a deadline that
/// answers to `POSTIO_TEST_PATIENCE`. 120 seconds before scaling: these wait
/// on WebKit loading a document.
pub fn settle_until(what: &str, done: impl Fn() -> bool) {
    postio_test_support::settle_until_within(
        postio_test_support::scaled(std::time::Duration::from_secs(120)),
        what,
        || {
            while glib::MainContext::default().iteration(false) {}
            // A document held by a dead web process is never going to
            // arrive; fail now, naming the death.
            if let Some(reason) = postio_widgets::composer::web_process::take_death() {
                panic!("a WebKit web process died ({reason}) while waiting for {what}");
            }
        },
        done,
    );
}

/// JavaScript for an `<img>`'s state, given an expression that finds it:
/// `absent`, `loading`, `decoded`, or `broken` -- finished, with no pixels.
pub(crate) fn image_state_js(find: &str) -> String {
    format!(
        "(() => {{ const i = {find}; if (!i) return 'absent'; \
           if (!i.complete) return 'loading'; \
           return i.naturalWidth > 0 ? 'decoded' : 'broken'; }})()"
    )
}

/// Whether a `postio-cid:` image has decoded, failing at once if it never
/// will (#1716): a broken image is final, so waiting longer cannot help.
pub(crate) fn cid_image_decoded(state: &str, what: &str) -> bool {
    match state {
        "decoded" => true,
        "broken" => panic!(
            "{what}: the image finished loading with no pixels -- the postio-cid: \
             request failed or was refused, so waiting longer cannot help"
        ),
        _ => false,
    }
}

/// JavaScript that answers `"true"` once the editor page's script has
/// attached every listener -- the marker `editor.js` sets last (#1716). A
/// test that types must wait for it, not merely for an editable body.
pub(crate) const EDITOR_LISTENING: &str = "String(window.postioEditorReady === true)";

// -- The window's set-up, as the cases spell it ---------------------------------

/// The app-level set-up the cases ask for, the shared sheet and the bundled icon.
/// There are no fonts to install.
pub mod app {
    use gtk::gdk;

    pub fn install_icons(display: &gdk::Display) {
        postio_widgets::style::install_icons(display);
    }
}

pub mod fonts {
    /// The shared widgets draw with the system's fonts, so there is nothing
    /// to install.
    pub fn install() -> Result<(), std::convert::Infallible> {
        Ok(())
    }
}

pub mod style {
    use gtk::gdk;

    pub fn install(display: &gdk::Display) {
        postio_widgets::style::install(display);
    }

    /// The shared sheet follows libadwaita's light/dark scheme, so there
    /// is nothing to track.
    pub fn track(_root: &impl gtk::prelude::IsA<gtk::Widget>) {}
}

/// `postio_widgets::composer`, plus the one function it adds: mounting a composer on a [`Window`].
pub mod composer {
    pub use postio_widgets::composer::*;

    use super::Window;

    pub fn install(window: &Window) -> Composer {
        window.install_composer()
    }
}

// -- A host for the composer ----------------------------------------------------

/// Whether the focused widget takes typed characters.
fn takes_text(window: &gtk::Window) -> bool {
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
}

/// Who hears the commands the window dispatches.
type Handlers = RefCell<Vec<Box<dyn Fn(CommandId)>>>;

struct Host {
    window: glib::WeakRef<gtk::Window>,
    pane: glib::WeakRef<gtk::Box>,
    actions: gio::SimpleActionGroup,
    commands: Handlers,
    resolver: RefCell<Resolver>,
    /// The context the keyboard is in, and what to put back when the
    /// composer gives the pane up.
    context: Cell<Context>,
    restore: Cell<Option<Context>>,
    /// The composer the window last built, for the body's keyboard: a
    /// `contenteditable` is not a text widget, but it is typing.
    composer: RefCell<glib::WeakRef<Composer>>,
}

impl Host {
    fn command_for(
        &self,
        key: gdk::Key,
        state: gdk::ModifierType,
        context: Context,
        typing: bool,
    ) -> Outcome {
        let Some(chord) = postio_widgets::keys::chord(key, state) else {
            return Outcome::Unhandled;
        };
        self.resolver
            .borrow_mut()
            .press(&chord, KeyContext::from(context), typing, Instant::now())
    }
}

impl ComposerHost for Host {
    fn parent(&self) -> Option<gtk::Window> {
        self.window.upgrade()
    }

    fn install(&self, composer: &Composer) {
        composer.set_vexpand(true);
        composer.set_visible(false);
        if let Some(pane) = self.pane.upgrade() {
            pane.append(composer);
        }
    }

    fn restore(&self, composer: &Composer) {
        if let Some(pane) = self.pane.upgrade() {
            pane.append(composer);
        }
    }

    fn remove(&self, composer: &Composer) {
        if let Some(pane) = self.pane.upgrade() {
            pane.remove(composer);
        }
    }

    fn take_pane(&self) {
        self.restore.set(Some(self.context.get()));
        self.context.set(Context::Composer);
    }

    fn release_pane(&self) {
        if let Some(context) = self.restore.take() {
            self.context.set(context);
        }
        if let Some(window) = self.window.upgrade() {
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        }
    }

    fn command_for_key(
        &self,
        key: gdk::Key,
        state: gdk::ModifierType,
        window: &gtk::Window,
    ) -> Option<CommandId> {
        match self.command_for(key, state, Context::Composer, takes_text(window)) {
            Outcome::Command(id) => match id.parse::<ActionId>() {
                Ok(ActionId::Builtin(id)) => Some(id),
                _ => None,
            },
            _ => None,
        }
    }

    fn handle_key(
        &self,
        _key: gdk::Key,
        _state: gdk::ModifierType,
        _window: &gtk::Window,
    ) -> glib::Propagation {
        glib::Propagation::Proceed
    }

    fn add_action(&self, action: &gio::SimpleAction) {
        self.actions.add_action(action);
    }

    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>) {
        self.commands.borrow_mut().push(handler);
    }

    fn keymap(&self) -> Keymap {
        Keymap::defaults().clone()
    }
}

/// A window with a pane for a composer, and a keyboard.
///
/// Derefs to the `gtk::Window`, so `present`, `destroy` and the widget calls
/// are the toolkit's own.
pub struct Window {
    window: gtk::Window,
    host: Rc<Host>,
    composer: RefCell<Option<Composer>>,
}

impl Default for Window {
    fn default() -> Self {
        let adw_window = adw::Window::new();
        adw_window.set_default_size(1280, 800);
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        adw_window.set_content(Some(&pane));
        let window: gtk::Window = adw_window.upcast();
        let actions = gio::SimpleActionGroup::new();
        window.insert_action_group("win", Some(&actions));
        let (resolver, _problems) =
            Resolver::from_commands_for(&Keymap::defaults().clone(), Frontend::Focus);
        let host = Rc::new(Host {
            window: window.downgrade(),
            pane: pane.downgrade(),
            actions,
            commands: RefCell::default(),
            resolver: RefCell::new(resolver),
            context: Cell::new(Context::List),
            restore: Cell::new(None),
            composer: RefCell::default(),
        });
        Window {
            window,
            host,
            composer: RefCell::new(None),
        }
    }
}

impl std::ops::Deref for Window {
    type Target = gtk::Window;
    fn deref(&self) -> &gtk::Window {
        &self.window
    }
}

impl Window {
    pub fn present(&self) {
        self.window.present();
    }

    pub fn destroy(&self) {
        self.window.destroy();
    }

    pub fn downgrade(&self) -> glib::WeakRef<gtk::Window> {
        self.window.downgrade()
    }

    /// The toolkit window, for the calls that take one.
    pub fn gtk(&self) -> &gtk::Window {
        &self.window
    }

    /// A composer mounted on this window.
    pub fn install_composer(&self) -> Composer {
        let composer = Composer::new();
        composer.mount_on(self.host.clone());
        self.host.composer.replace(composer.downgrade());
        composer
    }

    /// The window's composer, built the first time it is asked for.
    pub fn composer(&self) -> Composer {
        if let Some(composer) = self.composer.borrow().clone() {
            return composer;
        }
        let composer = self.install_composer();
        *self.composer.borrow_mut() = Some(composer.clone());
        composer
    }

    /// One key press into the window: resolved through the keymap in the
    /// window's context, and the command it names broadcast to the composer.
    /// Typing wins, as it does in an app window: a single key is a character
    /// while a text field, or the message body, has the keyboard.
    pub fn handle_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let body_has_keyboard = self
            .host
            .composer
            .borrow()
            .upgrade()
            .is_some_and(|composer| composer.focused_field() == Some(Field::Body));
        let typing = takes_text(&self.window) || body_has_keyboard;
        match self
            .host
            .command_for(key, state, self.host.context.get(), typing)
        {
            Outcome::Command(id) => match id.parse::<ActionId>() {
                Ok(ActionId::Builtin(id)) => {
                    for handler in self.host.commands.borrow().iter() {
                        handler(id);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            },
            Outcome::Pending(_) => glib::Propagation::Stop,
            Outcome::Unhandled => glib::Propagation::Proceed,
        }
    }
}
