//! A day heading over the list: "Today · Saturday 26 September" (FR-010).
//!
//! GTK names a section's heading by its first row, and that row can be a
//! placeholder when the heading is bound -- its page on its way. So the
//! heading follows the row, as a row widget does, and names the day the
//! moment the page lands, rather than staying blank until the next bind.

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::model::{FocusList, RowObject};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DayHeading {
        pub label: gtk::Label,
        pub bound: RefCell<Option<(RowObject, glib::SignalHandlerId)>>,
        pub list: glib::WeakRef<FocusList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayHeading {
        const NAME: &'static str = "PostioFocusDayHeading";
        type Type = super::DayHeading;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_layout_manager_type::<gtk::BinLayout>();
            class.set_css_name("focusdayheading");
            class.set_accessible_role(gtk::AccessibleRole::Heading);
        }
    }

    impl ObjectImpl for DayHeading {
        fn constructed(&self) {
            self.parent_constructed();
            self.label.add_css_class("focus-day-heading");
            self.label.set_xalign(0.0);
            self.label.set_parent(&*self.obj());
        }

        fn dispose(&self) {
            self.obj().unbind();
            self.label.unparent();
        }
    }

    impl WidgetImpl for DayHeading {}
}

glib::wrapper! {
    /// The heading over one day's rows.
    pub struct DayHeading(ObjectSubclass<imp::DayHeading>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for DayHeading {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl DayHeading {
    /// Name the day `row` arrived on -- or the heading `list` puts over
    /// everything while a filter is on -- and follow it as its page lands.
    pub fn bind(&self, row: &RowObject, list: &FocusList) {
        self.imp().list.set(Some(list));
        self.unbind();
        let handler = row.connect_changed({
            let heading = self.downgrade();
            move |row| {
                if let Some(heading) = heading.upgrade() {
                    heading.name(row);
                }
            }
        });
        self.imp().bound.replace(Some((row.clone(), handler)));
        self.name(row);
    }

    /// Stop following the row this heading was named from.
    pub fn unbind(&self) {
        if let Some((row, handler)) = self.imp().bound.take() {
            row.disconnect(handler);
        }
    }

    fn name(&self, row: &RowObject) {
        if let Some(single) = self
            .imp()
            .list
            .upgrade()
            .and_then(|list| list.single_heading())
        {
            self.imp().label.set_text(&single);
            return;
        }
        let today = chrono::Local::now().date_naive();
        let said = row
            .day()
            .map(|day| postio_ui::focus_row::day_heading(day, today))
            .unwrap_or_default();
        self.imp().label.set_text(&said);
    }
}
