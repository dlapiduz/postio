//! Which banner, and what it says, is `postio_ui::focus_state`'s. The banner is always revealed and
//! its slot shown or hidden, so it appears at once (motion budget). Markup is escaped: a server's
//! name is not ours to interpret.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_ui::focus_state::{Banner, BannerAction};

type Handler = Rc<dyn Fn(BannerAction)>;

pub struct StateBanner {
    root: gtk::Box,
    banner: adw::Banner,
    progress: gtk::ProgressBar,
    shown: RefCell<Option<Banner>>,
    handler: RefCell<Option<Handler>>,
    key: RefCell<Option<String>>,
}

impl StateBanner {
    pub fn new() -> Rc<Self> {
        let banner = adw::Banner::new("");
        banner.set_use_markup(true);
        banner.set_revealed(true);
        banner.add_css_class("focus-banner");
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("focus-banner-progress");
        progress.set_visible(false);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-banner-slot");
        root.append(&banner);
        root.append(&progress);
        root.set_visible(false);
        let slot = Rc::new(StateBanner {
            root,
            banner: banner.clone(),
            progress,
            shown: RefCell::default(),
            handler: RefCell::default(),
            key: RefCell::default(),
        });
        let weak = Rc::downgrade(&slot);
        banner.connect_button_clicked(move |_| {
            let Some(slot) = weak.upgrade() else {
                return;
            };
            let action = slot
                .shown
                .borrow()
                .as_ref()
                .and_then(Banner::action)
                .map(|(_, action)| action);
            let handler = slot.handler.borrow().clone();
            if let (Some(action), Some(handler)) = (action, handler) {
                handler(action);
            }
        });
        slot
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_action(&self, handler: impl Fn(BannerAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    pub fn show(&self, banner: Option<&Banner>, key: Option<String>) {
        if self.shown.borrow().as_ref() == banner && *self.key.borrow() == key {
            return;
        }
        self.shown.replace(banner.cloned());
        self.key.replace(key.clone());
        let Some(banner) = banner else {
            self.root.set_visible(false);
            return;
        };
        self.banner.set_title(&format!(
            "<b>{}</b>  {}",
            glib::markup_escape_text(&banner.heading()),
            glib::markup_escape_text(&banner.sentence())
        ));
        self.banner
            .set_button_label(banner.action().map(|(label, _)| label));
        if let Some(key) = key.as_deref() {
            postio_widgets::widgets::keyhint::dress_labelled_buttons(self.banner.upcast_ref(), key);
        }
        if banner.is_error() {
            self.banner.add_css_class("focus-banner-error");
        } else {
            self.banner.remove_css_class("focus-banner-error");
        }
        match banner.progress() {
            Some(fraction) => {
                self.progress.set_fraction(fraction);
                self.progress.set_visible(true);
            }
            None => self.progress.set_visible(false),
        }
        self.root.set_visible(true);
    }

    pub fn showing(&self) -> Option<(String, Option<String>, Option<f64>)> {
        if !self.root.is_visible() {
            return None;
        }
        Some((
            self.shown.borrow().as_ref()?.title(),
            self.shown
                .borrow()
                .as_ref()?
                .action()
                .map(|(label, _)| label.to_owned()),
            self.progress.is_visible().then(|| self.progress.fraction()),
        ))
    }
}
