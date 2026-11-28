//! Objects in the text: an inline image is a picture of its blob's bytes, resolved here and never
//! through a scheme or a URL; a rule and a remote image's chip are widgets at their anchors.

use gtk::prelude::*;
use postio_body::Run;

use super::Inner;

/// Anchors every object run in `range` that is still a bare U+FFFC, which is what a patch inserts.
/// Returns how many it made.
pub(super) fn anchor(inner: &Inner, range: std::ops::Range<u32>) -> u32 {
    let objects: Vec<(u32, Run)> = inner
        .projection
        .borrow()
        .runs_in(range)
        .filter(|(_, run)| {
            matches!(
                run,
                Run::Image { .. }
                    | Run::RemoteImage { .. }
                    | Run::Atomic {
                        kind: postio_body::projection::AtomicKind::Rule,
                        ..
                    }
            )
        })
        .map(|(at, run)| (at, run.clone()))
        .collect();
    let buffer = &inner.buffer;
    let mut made = 0;
    for (at, run) in objects {
        if buffer.iter_at_offset(at as i32).child_anchor().is_some() {
            continue;
        }
        let mut from = buffer.iter_at_offset(at as i32);
        let mut to = buffer.iter_at_offset(at as i32 + 1);
        buffer.delete(&mut from, &mut to);
        let mut here = buffer.iter_at_offset(at as i32);
        let anchor = buffer.create_child_anchor(&mut here);
        inner
            .view
            .add_child_at_anchor(&widget(inner, &run), &anchor);
        made += 1;
    }
    made
}

fn widget(inner: &Inner, run: &Run) -> gtk::Widget {
    match run {
        Run::Image { content_id, alt } => {
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_alternative_text(Some(alt));
            picture.add_css_class("postio-compose-image");
            if let Some((bytes, _)) = inner.source.resolve(content_id.as_str())
                && let Ok(texture) =
                    gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(bytes))
            {
                picture.set_paintable(Some(&texture));
            }
            picture.update_property(&[gtk::accessible::Property::Label(alt)]);
            picture.upcast()
        }
        Run::RemoteImage { alt, href } => {
            let words = if alt.is_empty() { href.as_str() } else { alt };
            let chip = gtk::Label::new(Some(&format!("Image: {words}")));
            chip.add_css_class("postio-compose-remote-image");
            chip.upcast()
        }
        _ => {
            let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
            rule.set_size_request(480, -1);
            rule.update_property(&[gtk::accessible::Property::Label("Divider")]);
            rule.upcast()
        }
    }
}
