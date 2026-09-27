//! The closed set of resources one render may use (spec FR-003,
//! data-model.md "Resource table"): the message's own parts and the images
//! it embeds. Built by the caller before the render; the engine is handed
//! this as its only `NetProvider`, so a lookup outside it resolves to
//! nothing -- counted, never fetched.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

use crate::Scope;

/// The scheme the composed document's `@font-face` rules use for the
/// bundled faces (`postio_ui::reader::document::FONT_SCHEME`).
pub const FONT_SCHEME: &str = "postio-font";

/// The resource table.
#[derive(Debug, Default)]
pub struct Resources {
    /// `(scope, content-id)` → the part's bytes. An unscoped document (one
    /// message, composed without a scope) keys its parts by `None`.
    parts: Mutex<HashMap<(Option<Scope>, String), Bytes>>,
    /// `postio-font:<name>` → a bundled face (ADR 0023). The URL scheme is
    /// fixed; the faces are the caller's, because the composed document's
    /// `@font-face` rules name them.
    fonts: Mutex<HashMap<String, Bytes>>,
    /// A remote image the app fetched for a consenting sender (FR-025),
    /// by its URL. Only postio-runtime's fetcher fills this: the engine
    /// never reaches the network.
    remote: Mutex<HashMap<String, Bytes>>,
    resolved: AtomicU32,
    unresolved: AtomicU32,
    placeholdered: AtomicU32,
    #[cfg(feature = "test-hooks")]
    hooks: hooks::Hooks,
}

impl Resources {
    /// An empty table.
    pub fn new() -> Resources {
        Resources::default()
    }

    /// Add one of a message's own parts, by its `Content-ID` (with or
    /// without the angle brackets).
    pub fn insert_part(&self, scope: Option<&str>, content_id: &str, bytes: Vec<u8>) {
        let id = content_id
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        self.parts
            .lock()
            .expect("the resource table is never poisoned")
            .insert(
                (scope.map(str::to_owned), id.to_owned()),
                Bytes::from(bytes),
            );
    }

    /// Add an image the app fetched from `url` for this message's sender.
    pub fn insert_remote(&self, url: &str, bytes: Vec<u8>) {
        self.remote
            .lock()
            .expect("the resource table is never poisoned")
            .insert(url.to_owned(), Bytes::from(bytes));
    }

    /// Add a bundled face, answered for `postio-font:<name>`.
    pub fn insert_font(&self, name: &str, bytes: &'static [u8]) {
        self.fonts
            .lock()
            .expect("the resource table is never poisoned")
            .insert(name.to_owned(), Bytes::from_static(bytes));
    }

    /// Lookups answered, and not, since the table was built.
    pub(crate) fn counts(&self) -> (u32, u32) {
        (
            self.resolved.load(Ordering::Relaxed),
            self.unresolved.load(Ordering::Relaxed),
        )
    }

    /// Images drawn as a placeholder since the table was built.
    pub(crate) fn placeholdered(&self) -> u32 {
        self.placeholdered.load(Ordering::Relaxed)
    }

    /// `bytes` as the engine may see them: SVG contained, rasters probed.
    fn admit(&self, bytes: Bytes) -> Option<Bytes> {
        let bytes = contained(bytes)?;
        if bytes.first() == Some(&b'<') {
            return Some(bytes);
        }
        match probe(bytes) {
            Probed::Image(bytes) => Some(bytes),
            Probed::Placeholder { width, height } => {
                self.placeholdered.fetch_add(1, Ordering::Relaxed);
                Some(placeholder(width, height))
            }
        }
    }

    fn lookup(&self, url: &str) -> Option<Bytes> {
        #[cfg(feature = "test-hooks")]
        self.hooks.run();
        if let Some(rest) = url
            .strip_prefix(postio_body::CID_SCHEME)
            .and_then(|rest| rest.strip_prefix(':'))
        {
            // `postio-cid:<scope>/<id>`, or `postio-cid:<id>` unscoped. The
            // encoded id never holds a `/` (postio-body's `part_uri`), and a
            // part of one scope is never returned to another (FR-004).
            let (scope, id) = match rest.split_once('/') {
                Some((scope, id)) => (Some(scope.to_owned()), id),
                None => (None, rest),
            };
            let id = postio_body::sanitize::percent_decode(id);
            let part = self
                .parts
                .lock()
                .expect("the resource table is never poisoned")
                .get(&(scope, id))
                .cloned()?;
            return self.admit(part);
        }
        if url.starts_with("http://") || url.starts_with("https://") {
            let fetched = self
                .remote
                .lock()
                .expect("the resource table is never poisoned")
                .get(url)
                .cloned()?;
            return self.admit(fetched);
        }
        if let Some(name) = url.strip_prefix(FONT_SCHEME) {
            return self
                .fonts
                .lock()
                .expect("the resource table is never poisoned")
                .get(name.trim_start_matches([':', '/']))
                .cloned();
        }
        // An image the message embeds in itself: decoded here, never
        // fetched. Any other `data:` was refused by the sanitizer.
        let payload = url.strip_prefix("data:image/")?.split_once(";base64,")?.1;
        let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
            .ok()
            .map(Bytes::from)
            .and_then(|bytes| self.admit(bytes))
    }
}

impl Resources {
    /// The bytes `url` stands for in this render, counted either way.
    fn resolve(&self, url: &str) -> Option<Bytes> {
        let found = self.lookup(url);
        let counter = if found.is_some() {
            &self.resolved
        } else {
            &self.unresolved
        };
        counter.fetch_add(1, Ordering::Relaxed);
        found
    }
}

impl NetProvider for Resources {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        if let Some(bytes) = self.resolve(&url) {
            handler.bytes(url, bytes);
        }
    }
}

/// What a resource's bytes turned out to be.
#[derive(Debug, PartialEq)]
enum Probed {
    /// Allowed, within the limits, and safe to hand the engine as it is.
    Image(Bytes),
    /// Refused or over a limit: drawn as a box of this size (FR-024).
    Placeholder { width: u32, height: u32 },
}

/// The most pixels an image may have on a side (research R4).
const MAX_SIDE: u32 = 8_192;
/// The most pixels an image may have in all.
const MAX_PIXELS: u64 = 40_000_000;
/// The most encoded bytes an image may have.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Read what an image is from its header alone, before any decoder sees
/// the pixels: formats the renderer does not decode, and images over the
/// limits, become placeholders. Blitz calls the decoder itself and passes
/// no limits, so this is the only place they can be held (research R4).
fn probe(bytes: Bytes) -> Probed {
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes[..])).with_guessed_format();
    let format = reader.as_ref().ok().and_then(|reader| reader.format());
    let allowed = matches!(
        format,
        Some(
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
        )
    );
    let dimensions = if allowed {
        reader.ok().and_then(|reader| reader.into_dimensions().ok())
    } else {
        None
    };
    let Some((width, height)) = dimensions else {
        return Probed::Placeholder {
            width: 0,
            height: 0,
        };
    };
    let over = bytes.len() > MAX_BYTES
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS;
    if over {
        Probed::Placeholder { width, height }
    } else {
        Probed::Image(bytes)
    }
}

/// A placeholder the engine can draw: a flat box of the image's size, or
/// one that takes the size its `<img>` declares when the size is unknown.
fn placeholder(width: u32, height: u32) -> Bytes {
    let size = if width > 0 && height > 0 {
        format!("width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"")
    } else {
        "width=\"1\" height=\"1\" viewBox=\"0 0 1 1\" preserveAspectRatio=\"none\"".to_owned()
    };
    Bytes::from(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" {size}>\
         <rect width=\"100%\" height=\"100%\" fill=\"#8888\"/></svg>"
    ))
}

/// `bytes` as the engine may see them: a raster as it is, and an SVG
/// re-serialised so it names nothing outside itself (research R5). usvg's
/// default resolver hands any `<image href>` path to `std::fs::read`, and
/// Blitz parses SVG images with the defaults, so an SVG part could paint a
/// local file into the page. An SVG that does not parse is dropped.
fn contained(bytes: Bytes) -> Option<Bytes> {
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let first = text.iter().find(|b| !b.is_ascii_whitespace());
    // No raster format starts with `<`; gzip is SVGZ, which usvg inflates.
    let svg = first == Some(&b'<') || bytes.starts_with(&[0x1f, 0x8b]);
    if !svg {
        return Some(bytes);
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|mime, data, _| raster(mime, data)),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&bytes, &options).ok()?;
    Some(Bytes::from(tree.to_string(&usvg::WriteOptions::default())))
}

/// A `data:` raster inside an SVG: the four formats the renderer decodes,
/// and nothing nested -- an SVG inside an SVG is refused, not recursed into.
fn raster(mime: &str, data: std::sync::Arc<Vec<u8>>) -> Option<usvg::ImageKind> {
    match mime {
        "image/png" => Some(usvg::ImageKind::PNG(data)),
        "image/jpeg" | "image/jpg" => Some(usvg::ImageKind::JPEG(data)),
        "image/gif" => Some(usvg::ImageKind::GIF(data)),
        "image/webp" => Some(usvg::ImageKind::WEBP(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GIF whose header declares `width` × `height`: all a header-only
    /// probe reads, so an over-limit image costs nothing to make.
    fn gif_header(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        // No global colour table; then an image descriptor and trailer.
        bytes.extend([0x00, 0x00, 0x00]);
        bytes.extend([0x2c, 0, 0, 0, 0]);
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend([0x00, 0x02, 0x02, 0x4c, 0x01, 0x00, 0x3b]);
        bytes
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("a PNG encodes");
        out
    }

    #[test]
    fn a_part_never_resolves_for_another_message() {
        let table = Resources::new();
        table.insert_part(Some("7"), "<logo@example.com>", png(2, 2));
        assert!(table.resolve("postio-cid:7/logo%40example.com").is_some());
        assert!(table.resolve("postio-cid:11/logo%40example.com").is_none());
        assert!(table.resolve("postio-cid:logo%40example.com").is_none());
    }

    #[test]
    fn an_unknown_key_resolves_to_nothing_and_is_counted() {
        let table = Resources::new();
        assert!(
            table
                .resolve("postio-cid:7/missing%40example.com")
                .is_none()
        );
        assert!(table.resolve("https://tracker.example/pixel.gif").is_none());
        assert_eq!(table.counts(), (0, 2));
    }

    #[test]
    fn an_image_within_the_limits_passes_untouched() {
        let bytes = png(4, 3);
        assert_eq!(
            probe(Bytes::from(bytes.clone())),
            Probed::Image(Bytes::from(bytes))
        );
    }

    #[test]
    fn an_image_over_a_limit_becomes_a_placeholder_of_its_size() {
        assert_eq!(
            probe(Bytes::from(gif_header(8_193, 10))),
            Probed::Placeholder {
                width: 8_193,
                height: 10
            }
        );
        // 7,000 × 6,000 is 42 megapixels: each side is within 8,192.
        assert_eq!(
            probe(Bytes::from(gif_header(7_000, 6_000))),
            Probed::Placeholder {
                width: 7_000,
                height: 6_000
            }
        );
        let mut huge = png(1, 1);
        huge.resize(16 * 1024 * 1024 + 1, 0);
        assert!(matches!(
            probe(Bytes::from(huge)),
            Probed::Placeholder { .. }
        ));
    }

    #[test]
    fn a_refused_format_becomes_a_placeholder() {
        let refused: [(&str, &[u8]); 4] = [
            ("tiff", b"II*\0\x08\0\0\0"),
            ("bmp", b"BM\x3a\0\0\0\0\0\0\0\x36\0\0\0"),
            ("ico", b"\0\0\x01\0\x01\0\x10\x10"),
            ("avif", b"\0\0\0\x1cftypavif\0\0\0\0avifmif1"),
        ];
        for (name, bytes) in refused {
            assert!(
                matches!(probe(Bytes::from_static(bytes)), Probed::Placeholder { .. }),
                "{name} was not refused"
            );
        }
    }

    /// No animation (FR-024): what the engine decodes of an animated GIF is
    /// its first frame.
    #[test]
    fn an_animated_gif_yields_its_first_frame() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let frame = |rgb: [u8; 3]| {
            image::Frame::new(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
            ))
        };
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("a looping GIF");
            encoder
                .encode_frames([frame([255, 0, 0]), frame([0, 0, 255])])
                .expect("two frames encode");
        }
        let Probed::Image(out) = probe(Bytes::from(bytes)) else {
            panic!("an animated GIF within the limits is an image");
        };
        let still = image::load_from_memory(&out)
            .expect("it decodes")
            .to_rgba8();
        assert_eq!(
            still.get_pixel(0, 0).0[..3],
            [255, 0, 0],
            "not the first frame"
        );
    }
}

/// What the render-thread tests drive, behind the `test-hooks` feature.
#[cfg(feature = "test-hooks")]
mod hooks {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// A lookup that panics, and a lookup held until released.
    #[derive(Debug, Default)]
    pub(super) struct Hooks {
        pub(super) panic: AtomicBool,
        pub(super) hold: Mutex<Option<Arc<Gate>>>,
    }

    impl Hooks {
        pub(super) fn run(&self) {
            if self.panic.load(Ordering::Relaxed) {
                panic!("test hook: a resource lookup panicked");
            }
            let gate = self.hold.lock().expect("hooks are never poisoned").clone();
            if let Some(gate) = gate {
                gate.enter();
            }
        }
    }

    /// Where a held lookup waits, and how the test knows it got there.
    #[derive(Debug, Default)]
    pub struct Gate {
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    impl Gate {
        fn enter(&self) {
            let mut state = self.state.lock().expect("the gate is never poisoned");
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, Duration::from_secs(60))
                    .expect("the gate is never poisoned");
                state = next;
                if timeout.timed_out() {
                    break;
                }
            }
        }

        /// Let the held lookup go on.
        #[doc(hidden)]
        pub fn release(&self) {
            let mut state = self.state.lock().expect("the gate is never poisoned");
            state.1 = true;
            self.changed.notify_all();
        }

        /// Wait until a lookup is being held, or `patience` passes.
        #[doc(hidden)]
        pub fn wait_until_held(&self, patience: Duration) {
            let state = self.state.lock().expect("the gate is never poisoned");
            let _ = self
                .changed
                .wait_timeout_while(state, patience, |state| !state.0)
                .expect("the gate is never poisoned");
        }
    }
}

#[cfg(feature = "test-hooks")]
pub use hooks::Gate;

#[cfg(feature = "test-hooks")]
impl Resources {
    /// Make every lookup panic, as an engine bug would.
    pub fn panic_on_lookup(&self) {
        self.hooks.panic.store(true, Ordering::Relaxed);
    }

    /// Hold every lookup until the returned gate is released.
    pub fn hold_lookup(&self) -> std::sync::Arc<Gate> {
        let gate = std::sync::Arc::new(Gate::default());
        *self.hooks.hold.lock().expect("hooks are never poisoned") = Some(gate.clone());
        gate
    }
}
