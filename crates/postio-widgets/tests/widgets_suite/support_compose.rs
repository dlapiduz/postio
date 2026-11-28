use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use postio_core::{ActionId, CommandId, Context, Frontend, Keymap};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use postio_widgets::composer::{Composer, ComposerHost, Field};

pub fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

pub fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// A count is a guess: prefer `settle_until` where the case knows what it waits for.
pub fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.iteration(false) {}
    }
}

/// 120 seconds before scaling: these wait on WebKit loading a document.
pub fn settle_until(what: &str, done: impl Fn() -> bool) {
    postio_test_support::settle_until_within(
        postio_test_support::scaled(std::time::Duration::from_secs(120)),
        what,
        || {
            while glib::MainContext::default().iteration(false) {}
            // A document held by a dead web process never arrives; fail now, naming the death.
            if let Some(reason) = postio_widgets::composer::web_process::take_death() {
                panic!("a WebKit web process died ({reason}) while waiting for {what}");
            }
        },
        done,
    );
}

/// `decoded` means the pixels decoded, not that the header gave a size (#2157): WebKit takes
/// `naturalWidth` from the header and decodes on first paint, so `decode()` asks for it now.
pub(crate) fn image_state_js(find: &str) -> String {
    format!(
        "(() => {{ const i = {find}; if (!i) return 'absent'; \
           if (!i.complete) return 'loading'; \
           if (i.naturalWidth === 0) return 'broken'; \
           if (i.postioDecoded === undefined) {{ \
             i.postioDecoded = 'loading'; \
             i.decode().then(() => {{ i.postioDecoded = 'decoded'; }}, \
                             () => {{ i.postioDecoded = 'broken'; }}); \
           }} \
           return i.postioDecoded; }})()"
    )
}

/// A broken image is final, so waiting longer cannot help (#1716).
pub(crate) fn cid_image_decoded(state: &str, what: &str) -> bool {
    match state {
        "decoded" => true,
        "broken" => panic!(
            "{what}: the image finished loading with no pixels -- the postio-cid: \
             request failed or was refused, or the bytes it served do not decode, \
             so waiting longer cannot help"
        ),
        _ => false,
    }
}

/// `editor.js` sets the marker last (#1716); a test that types must wait for it, not merely an
/// editable body.
pub(crate) const EDITOR_LISTENING: &str = "String(window.postioEditorReady === true)";

pub mod app {
    use gtk::gdk;

    pub fn install_icons(display: &gdk::Display) {
        postio_widgets::style::install_icons(display);
    }
}

pub mod fonts {
    pub fn install() -> Result<(), std::convert::Infallible> {
        Ok(())
    }
}

pub mod style {
    use gtk::gdk;

    pub fn install(display: &gdk::Display) {
        postio_widgets::style::install(display);
    }

    pub fn track(_root: &impl gtk::prelude::IsA<gtk::Widget>) {}
}

pub mod composer {
    pub use postio_widgets::composer::*;

    use super::Window;

    pub fn install(window: &Window) -> Composer {
        window.install_composer()
    }

    pub fn install_native(window: &Window) -> Composer {
        surface::test_choose(Some(surface::SurfaceKind::Native));
        let composer = window.install_composer();
        surface::test_choose(None);
        composer
    }
}

fn takes_text(window: &gtk::Window) -> bool {
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
}

type Handlers = RefCell<Vec<Box<dyn Fn(CommandId)>>>;

struct Host {
    window: glib::WeakRef<gtk::Window>,
    pane: glib::WeakRef<gtk::Box>,
    actions: gio::SimpleActionGroup,
    commands: Handlers,
    resolver: RefCell<Resolver>,
    context: Cell<Context>,
    restore: Cell<Option<Context>>,
    /// A `contenteditable` is not a text widget, but it is typing.
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

    pub fn gtk(&self) -> &gtk::Window {
        &self.window
    }

    pub fn install_composer(&self) -> Composer {
        let composer = Composer::new();
        composer.mount_on(self.host.clone());
        self.host.composer.replace(composer.downgrade());
        composer
    }

    pub fn composer(&self) -> Composer {
        if let Some(composer) = self.composer.borrow().clone() {
            return composer;
        }
        let composer = self.install_composer();
        *self.composer.borrow_mut() = Some(composer.clone());
        composer
    }

    /// Typing wins, as in an app window: a single key is a character while a text field or the
    /// message body has the keyboard.
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
