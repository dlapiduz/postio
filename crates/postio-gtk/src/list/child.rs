//! A child row: one message of the expanded thread, drawn under its row. One `snapshot()`; the row
//! widget in its child form, since a list view's rows are recycled between kinds.

use gtk::graphene;
use gtk::pango;
use gtk::prelude::*;
use postio_ui::label_colour::label_colour;
use postio_ui::thread_list::ChildRowView;

use super::row::{
    DOT, Drawn, GAP, GUTTER_CENTRE, ICON, KEYCAP_PAD, Palette, RowWidget, TRAILING, rgba,
};

pub const CHILD_HEIGHT: f32 = 38.0;
const LINE_WIDTH: f32 = 1.5;
const AVATAR: f32 = 20.0;
const AVATAR_X: f32 = 44.0;
const SENDER_X: f32 = AVATAR_X + AVATAR + 8.0;

pub fn spoken(child: &ChildRowView) -> String {
    let mut said = child.sender.clone();
    if let Some(tag) = &child.tag {
        said.push_str(", ");
        said.push_str(tag);
    }
    said.push_str(": ");
    said.push_str(&child.line);
    said.push_str(", ");
    said.push_str(&child.time);
    if child.bold {
        said.push_str(", unread");
    }
    said
}

impl RowWidget {
    fn child_layout(&self, text: &str, bold: bool, italic: bool, scale: f64) -> pango::Layout {
        let layout = self.create_pango_layout(Some(text));
        let attributes = pango::AttrList::new();
        if bold {
            attributes.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
        }
        if italic {
            attributes.insert(pango::AttrInt::new_style(pango::Style::Italic));
        }
        if (scale - 1.0).abs() > f64::EPSILON {
            attributes.insert(pango::AttrFloat::new_scale(scale));
        }
        layout.set_attributes(Some(&attributes));
        layout.set_single_paragraph_mode(true);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        layout
    }

    pub(super) fn draw_child(&self, snapshot: &gtk::Snapshot, child: &ChildRowView, last: bool) {
        let palette = Palette::of(self);
        let width = self.width() as f32;
        let height = self.scaled(CHILD_HEIGHT);
        let middle = height / 2.0;
        let columns = postio_ui::focus_row::row_columns(width);
        let mut drawn = Drawn {
            bold: child.bold,
            ink: palette.ink,
            ..Drawn::default()
        };

        snapshot.append_color(
            &palette.raised,
            &graphene::Rect::new(0.0, 0.0, width, height),
        );
        let line_bottom = if last { middle } else { height };
        snapshot.append_color(
            &palette.rule,
            &graphene::Rect::new(
                GUTTER_CENTRE - LINE_WIDTH / 2.0,
                0.0,
                LINE_WIDTH,
                line_bottom,
            ),
        );
        let dot = graphene::Rect::new(GUTTER_CENTRE - DOT / 2.0, middle - DOT / 2.0, DOT, DOT);
        let dot_colour = if child.bold {
            palette.accent
        } else {
            palette.rule
        };
        snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(dot, DOT / 2.0));
        snapshot.append_color(&dot_colour, &dot);
        snapshot.pop();
        drawn.dot = child.bold;

        let time = self.child_layout(&child.time, child.bold, false, 1.0);
        let (time_width, _) = time.pixel_size();
        let mut trailing = width - TRAILING - time_width as f32;
        self.put(
            snapshot,
            &time,
            trailing,
            middle,
            time_width as f32,
            &palette.ink,
        );
        if child.attachment {
            trailing -= GAP + ICON;
            self.draw_icon(
                snapshot,
                "mail-attachment-symbolic",
                trailing,
                middle - ICON / 2.0,
                &palette.dim,
            );
        }
        let mut hint_texts = Vec::new();
        if self.is_cursor_here()
            && let Some(hint) = &child.focused_hint
        {
            let label = self.child_layout(&hint.label, false, false, 0.85);
            let (label_width, _) = label.pixel_size();
            trailing -= GAP + label_width as f32;
            self.put(
                snapshot,
                &label,
                trailing,
                middle,
                label_width as f32,
                &palette.dim,
            );
            hint_texts.push(hint.label.clone());
            for key in hint.keys.iter().rev() {
                let cap = self.child_layout(key, false, false, 0.8);
                let (cap_width, cap_height) = cap.pixel_size();
                let boxed = (cap_width as f32 + KEYCAP_PAD * 2.0).max(cap_height as f32 + 4.0);
                trailing -= 4.0 + boxed;
                let frame = graphene::Rect::new(
                    trailing,
                    middle - (cap_height as f32 + 4.0) / 2.0,
                    boxed,
                    cap_height as f32 + 4.0,
                );
                snapshot.append_border(
                    &gtk::gsk::RoundedRect::from_rect(frame, 4.0),
                    &[1.0; 4],
                    &[palette.rule; 4],
                );
                self.put(
                    snapshot,
                    &cap,
                    trailing + (boxed - cap_width as f32) / 2.0,
                    middle,
                    cap_width as f32,
                    &palette.dim,
                );
                hint_texts.push(key.clone());
            }
        }
        let end = trailing - GAP;

        let avatar = graphene::Rect::new(AVATAR_X, middle - AVATAR / 2.0, AVATAR, AVATAR);
        let fill = rgba(label_colour(&child.sender, None, palette.accent_hue));
        snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(avatar, AVATAR / 2.0));
        snapshot.append_color(&fill, &avatar);
        snapshot.pop();
        let initials = self.child_layout(&child.initials, true, false, 0.62);
        let (initials_width, _) = initials.pixel_size();
        self.put(
            snapshot,
            &initials,
            AVATAR_X + (AVATAR - initials_width as f32) / 2.0,
            middle,
            initials_width as f32,
            &palette.ground,
        );
        let sender = self.child_layout(&child.sender, child.bold, child.draft, 1.0);
        self.put(
            snapshot,
            &sender,
            SENDER_X,
            middle,
            (columns.subject_x - GAP - SENDER_X).min(end - SENDER_X),
            &palette.ink,
        );
        drawn.texts.push(child.sender.clone());

        let mut x = columns.subject_x;
        if let Some(tag) = &child.tag {
            let layout = self.child_layout(tag, false, false, 0.85);
            let (tag_width, tag_height) = layout.pixel_size();
            let boxed = tag_width as f32 + 12.0;
            if x + boxed < end {
                let frame = graphene::Rect::new(
                    x,
                    middle - (tag_height as f32 + 4.0) / 2.0,
                    boxed,
                    tag_height as f32 + 4.0,
                );
                snapshot.append_border(
                    &gtk::gsk::RoundedRect::from_rect(frame, 4.0),
                    &[1.0; 4],
                    &[palette.rule; 4],
                );
                self.put(
                    snapshot,
                    &layout,
                    x + 6.0,
                    middle,
                    tag_width as f32,
                    &palette.dim,
                );
                drawn.texts.push(tag.clone());
                x += boxed + GAP;
            }
        }
        let line = self.child_layout(&child.line, false, child.draft, 1.0);
        let colour = if child.placeholder || child.draft {
            &palette.dim
        } else {
            &palette.ink
        };
        self.put(snapshot, &line, x, middle, end - x, colour);
        drawn.texts.push(child.line.clone());
        drawn.texts.push(child.time.clone());
        drawn.texts.extend(hint_texts.into_iter().rev());
        self.set_drawn(drawn);
    }
}
