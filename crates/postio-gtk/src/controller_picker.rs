//! The move picker the controller drives (spec 013 D2): `MoveThisMessage`
//! aims a move at one message, which GTK's own [`MovePicker`](crate::move_picker)
//! -- aimed at the cursor's row and the selection -- cannot say. The
//! controller keeps what it acts on and what it lists; this draws its
//! [`PickerView`] in the shared picker frame and hands back what a person
//! typed or chose.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Keymap;
use postio_focus::{Input, PickerView};
use postio_ui::pickers;
use postio_widgets::widgets::pickers::{Field, Picked, Picker, Row};

type Handler = Rc<dyn Fn(Input)>;

/// The picker. See the module.
pub struct ControllerPicker {
    picker: Rc<Picker>,
    /// The token each row of the list stands for, in the rows' order.
    tokens: RefCell<Vec<u64>>,
    handler: RefCell<Option<Handler>>,
    /// Raised while the controller is closing it, so that its going is not
    /// said back as a person's.
    closing: Cell<bool>,
}

impl ControllerPicker {
    /// A closed picker, its keys from `keymap`.
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let picker = Picker::new(
            keymap,
            pickers::MOVE_TITLE,
            Field::Filter(pickers::MOVE_FILTER),
            &pickers::move_footnote(keymap),
        );
        let this = Rc::new(ControllerPicker {
            picker,
            tokens: RefCell::default(),
            handler: RefCell::default(),
            closing: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.picker.connect_picked({
            let weak = weak.clone();
            move |picked| {
                if let Some(this) = weak.upgrade() {
                    let index = match picked {
                        Picked::Choose(index) => Some(index),
                        Picked::Confirm { selected, .. } => selected,
                        Picked::Toggle(_) => None,
                    };
                    let token = index.and_then(|index| this.tokens.borrow().get(index).copied());
                    if let Some(token) = token {
                        this.say(Input::PickerChoose(token));
                    }
                }
            }
        });
        this.picker.connect_field_changed({
            let weak = weak.clone();
            move |text| {
                if let Some(this) = weak.upgrade() {
                    this.say(Input::PickerTyped {
                        text: text.to_owned(),
                    });
                }
            }
        });
        this.picker.connect_closed(move || {
            if let Some(this) = weak.upgrade()
                && !this.closing.get()
            {
                this.say(Input::SurfaceClosed(postio_focus::SurfaceKind::Picker));
            }
        });
        this
    }

    /// The frame, for keys and tests.
    pub fn picker(&self) -> &Rc<Picker> {
        &self.picker
    }

    /// Run `handler` with what a person did.
    pub fn connect_input(&self, handler: impl Fn(Input) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    fn say(&self, input: Input) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(input);
        }
    }

    /// Draw `view` and open it from `parent`, pointing at `rect`.
    pub fn open(
        &self,
        view: &PickerView,
        parent: &impl IsA<gtk::Widget>,
        rect: Option<&gdk::Rectangle>,
    ) {
        self.picker.set_target(&view.target);
        self.picker.set_footnote(&view.footnote);
        self.show_rows(view);
        self.picker.open(parent, rect);
    }

    /// Redraw the rows: what the read brought, or what typing kept.
    pub fn show_rows(&self, view: &PickerView) {
        self.tokens
            .replace(view.rows.iter().map(|row| row.token).collect());
        self.picker.set_rows(
            view.rows
                .iter()
                .map(|row| Row {
                    section: row.section.clone(),
                    name: row.name.clone(),
                    detail: row.detail.clone(),
                    numbered: row.key.is_some(),
                    ..Row::default()
                })
                .collect(),
        );
    }

    /// Put it away, as the controller says.
    pub fn close(&self) {
        self.closing.set(true);
        self.picker.close();
        self.closing.set(false);
    }
}
