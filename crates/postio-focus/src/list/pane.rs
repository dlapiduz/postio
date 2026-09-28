//! The list as it sits in the window: a `GtkListView` over the feed's
//! model, in a scroller, with Focus's rows.

use gtk::graphene;
use gtk::prelude::*;

use super::feed::Feed;
use super::heading::DayHeading;
use super::model::RowObject;
use super::row::RowWidget;

/// The list pane: what scrolls, what draws, and where its rows come from.
#[derive(Clone)]
pub struct ListPane {
    scrolled: gtk::ScrolledWindow,
    view: gtk::ListView,
    cursor: gtk::SingleSelection,
    feed: Feed,
}

impl ListPane {
    /// A pane drawing `feed`'s list.
    pub fn new(feed: Feed) -> Self {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("a list view's factory builds list items");
            item.set_child(Some(&RowWidget::default()));
        });
        factory.connect_bind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("a list view's factory builds list items");
            if let (Some(row), Some(widget)) = (
                item.item().and_downcast::<RowObject>(),
                item.child().and_downcast::<RowWidget>(),
            ) {
                widget.bind(&row);
            }
        });
        factory.connect_unbind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("a list view's factory builds list items");
            if let Some(widget) = item.child().and_downcast::<RowWidget>() {
                widget.unbind();
            }
        });

        // The cursor, not the selection: GTK's name, Postio's meaning
        // (`postio_ui::selection`). What `a` would archive is kept apart.
        let cursor = gtk::SingleSelection::new(Some(feed.list().clone()));
        cursor.set_autoselect(false);
        cursor.set_can_unselect(true);
        let view = gtk::ListView::new(Some(cursor.clone()), Some(factory));
        view.set_header_factory(Some(&day_headings()));
        // A page landing may move where a day starts.
        feed.list().connect_local("filled", false, {
            let list = feed.list().downgrade();
            move |_| {
                if let Some(list) = list.upgrade() {
                    list.days_moved();
                }
                None
            }
        });
        view.add_css_class("focus-list");
        view.update_property(&[gtk::accessible::Property::Label("Inbox")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();
        ListPane {
            scrolled,
            view,
            cursor,
            feed,
        }
    }

    /// The pane's outermost widget, to place in a layout.
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scrolled
    }

    /// The list view.
    pub fn view(&self) -> &gtk::ListView {
        &self.view
    }

    /// The cursor: where the keyboard is.
    pub fn cursor(&self) -> &gtk::SingleSelection {
        &self.cursor
    }

    /// The feed the rows come from.
    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    /// The row widgets a person can see, top to bottom: bound, mapped, and
    /// inside the scroller's viewport.
    pub fn rows_on_screen(&self) -> Vec<RowWidget> {
        let height = self.scrolled.height() as f32;
        let mut rows: Vec<(f32, RowWidget)> = Vec::new();
        let mut child = self.view.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Some(row) = widget.first_child().and_downcast::<RowWidget>() else {
                continue;
            };
            if !row.is_mapped() || row.item().is_none() {
                continue;
            }
            let Some(top) = row.compute_point(&self.scrolled, &graphene::Point::new(0.0, 0.0))
            else {
                continue;
            };
            let bottom = top.y() + row.height() as f32;
            if bottom <= 0.0 || top.y() >= height {
                continue;
            }
            rows.push((top.y(), row));
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, row)| row).collect()
    }
}

/// The day headings: "Today · Saturday 26 September" over a day's rows, a
/// section header each (32 px), named from the section's first row.
fn day_headings() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        if let Some(header) = header.downcast_ref::<gtk::ListHeader>() {
            header.set_child(Some(&DayHeading::default()));
        }
    });
    factory.connect_bind(|_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        if let (Some(row), Some(heading)) = (
            header.item().and_downcast::<RowObject>(),
            header.child().and_downcast::<DayHeading>(),
        ) {
            heading.bind(&row);
        }
    });
    factory.connect_unbind(|_, header| {
        if let Some(heading) = header
            .downcast_ref::<gtk::ListHeader>()
            .and_then(|header| header.child())
            .and_downcast::<DayHeading>()
        {
            heading.unbind();
        }
    });
    factory
}

impl std::fmt::Debug for ListPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListPane").finish_non_exhaustive()
    }
}
