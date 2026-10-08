//! The snooze and remind pickers (screens 11 and 12): four presets with
//! their times and number keys, a date typed in words, and a footnote.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Local};
use gtk::gdk;
use postio_core::Keymap;
use postio_ui::pickers;

use super::{Field, Picked, Picker, Row};

/// Which of the two a picker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// "Snooze until": the mail leaves the inbox and comes back.
    Snooze,
    /// "Remind me if no one replies by".
    Remind,
}

type Handler = Rc<dyn Fn(DateTime<Local>)>;

/// A snooze or remind picker. See the module.
pub struct WhenPicker {
    picker: Rc<Picker>,
    when: When,
    keymap: RefCell<Keymap>,
    /// The presets' times, as of the last open.
    presets: RefCell<Vec<DateTime<Local>>>,
    /// The clock the picker last opened at: what a typed date counts from.
    now: RefCell<Option<DateTime<Local>>>,
    handler: RefCell<Option<Handler>>,
}

impl WhenPicker {
    /// A closed picker of kind `when`, its keys read from `keymap`.
    pub fn new(keymap: &Keymap, when: When) -> Rc<Self> {
        let title = match when {
            When::Snooze => pickers::SNOOZE_TITLE,
            When::Remind => pickers::REMIND_TITLE,
        };
        let picker = Picker::new(keymap, title, Field::Date, "");
        let this = Rc::new(WhenPicker {
            picker,
            when,
            keymap: RefCell::new(keymap.clone()),
            presets: RefCell::default(),
            now: RefCell::default(),
            handler: RefCell::default(),
        });
        let weak = Rc::downgrade(&this);
        this.picker.connect_picked({
            let weak = weak.clone();
            move |picked| {
                if let Some(this) = weak.upgrade() {
                    this.picked(picked);
                }
            }
        });
        this.picker.connect_field_changed(move |text| {
            if let Some(this) = weak.upgrade() {
                this.preview(text);
            }
        });
        this
    }

    /// Run `handler` with the moment chosen.
    pub fn connect_chosen(&self, handler: impl Fn(DateTime<Local>) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.picker.set_keymap(keymap);
    }

    /// The frame, for keys, tests and placing.
    pub fn picker(&self) -> &Rc<Picker> {
        &self.picker
    }

    /// Open from `parent` at `rect`, naming `target`, with the presets as
    /// they are at `now`.
    pub fn open(
        &self,
        parent: &impl gtk::prelude::IsA<gtk::Widget>,
        rect: Option<&gdk::Rectangle>,
        target: &str,
        now: DateTime<Local>,
    ) {
        let presets = match self.when {
            When::Snooze => postio_ui::schedule::snooze_presets(now),
            When::Remind => postio_ui::schedule::remind_presets(now),
        };
        self.presets
            .replace(presets.iter().map(|(_, at)| *at).collect());
        self.now.replace(Some(now));
        self.picker.set_rows(
            presets
                .iter()
                .map(|(name, at)| Row {
                    name: (*name).to_owned(),
                    detail: pickers::when_label(*at, now),
                    numbered: true,
                    ..Row::default()
                })
                .collect(),
        );
        self.picker.set_target(target);
        self.picker.set_footnote(&match self.when {
            When::Snooze => pickers::snooze_footnote(&self.keymap.borrow()),
            When::Remind => pickers::remind_footnote(now),
        });
        self.picker.open(parent, rect);
        self.preview("");
    }

    /// What typing says under the field: when the words land, or how to
    /// start.
    fn preview(&self, text: &str) {
        let Some(now) = *self.now.borrow() else {
            return;
        };
        let hint = if text.trim().is_empty() {
            "Tab to type".to_owned()
        } else {
            match pickers::typed(text, now) {
                Some(at) => pickers::when_label(at, now),
                None => "A day and a time: \u{201c}tue 9am\u{201d}".to_owned(),
            }
        };
        self.picker.set_field_hint(&hint);
    }

    fn picked(&self, picked: Picked) {
        let at = match picked {
            Picked::Choose(index) => self.presets.borrow().get(index).copied(),
            Picked::Confirm { typed, selected } if typed.trim().is_empty() => {
                selected.and_then(|index| self.presets.borrow().get(index).copied())
            }
            Picked::Confirm { typed, .. } => {
                let now = *self.now.borrow();
                now.and_then(|now| pickers::typed(&typed, now))
            }
            Picked::Toggle(_) => None,
        };
        let Some(at) = at else {
            return;
        };
        self.picker.close();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(at);
        }
    }
}
