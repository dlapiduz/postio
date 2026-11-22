//! The closed set of resources one render may use: the engine's only `NetProvider`, so a lookup
//! outside it resolves to nothing, counted and never fetched.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

use crate::Scope;

pub const FONT_SCHEME: &str = "postio-font";

/// A placeholder answers so the picture's box says a picture is there rather than lying blank
/// (#1764).
pub(crate) const MISSING_PART: &str = "postio-missing-part:";

const MISSING_PART_SIDE: u32 = 24;

#[derive(Debug, Default)]
pub struct Resources {
    /// `(scope, content-id)` → the part's bytes. An unscoped document (one
    /// message, composed without a scope) keys its parts by `None`.
    parts: Mutex<HashMap<(Option<Scope>, String), Bytes>>,
    /// The faces are the caller's, because the composed document's `@font-face` rules name them.
    fonts: Mutex<HashMap<String, Bytes>>,
    /// Only postio-runtime's fetcher fills this: the engine never reaches the network.
    remote: Mutex<HashMap<String, Bytes>>,
    resolved: AtomicU32,
    unresolved: AtomicU32,
    placeholdered: AtomicU32,
    #[cfg(feature = "test-hooks")]
    hooks: hooks::Hooks,
}

impl Resources {
    pub fn new() -> Resources {
        Resources::default()
    }

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

    /// Kept under the parsed URL, as the engine asks for it: `https://IMAGES.example.net/a b.png`
    /// is requested as `https://images.example.net/a%20b.png`.
    pub fn insert_remote(&self, url: &str, bytes: Vec<u8>) {
        let key = url::Url::parse(url).map_or_else(|_| url.to_owned(), |parsed| parsed.to_string());
        self.remote
            .lock()
            .expect("the resource table is never poisoned")
            .insert(key, Bytes::from(bytes));
    }

    pub fn insert_font(&self, name: &str, bytes: &'static [u8]) {
        self.fonts
            .lock()
            .expect("the resource table is never poisoned")
            .insert(name.to_owned(), Bytes::from_static(bytes));
    }

    pub(crate) fn counts(&self) -> (u32, u32) {
        (
            self.resolved.load(Ordering::Relaxed),
            self.unresolved.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn placeholdered(&self) -> u32 {
        self.placeholdered.load(Ordering::Relaxed)
    }

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

    /// The part a `postio-cid:` URL names: `None` when `url` is not one,
    /// `Some(None)` when the message has no such part.
    fn part(&self, url: &str) -> Option<Option<Bytes>> {
        let rest = url
            .strip_prefix(postio_body::CID_SCHEME)
            .and_then(|rest| rest.strip_prefix(':'))?;
        // `postio-cid:<scope>/<id>`, or `postio-cid:<id>` unscoped. The
        // encoded id never holds a `/` (postio-body's `part_uri`), and a
        // part of one scope is never returned to another (FR-004).
        let (scope, id) = match rest.split_once('/') {
            Some((scope, id)) => (Some(scope.to_owned()), id),
            None => (None, rest),
        };
        let id = postio_body::sanitize::percent_decode(id);
        Some(
            self.parts
                .lock()
                .expect("the resource table is never poisoned")
                .get(&(scope, id))
                .cloned(),
        )
    }

    pub(crate) fn is_missing_part(&self, url: &str) -> bool {
        matches!(self.part(url), Some(None))
    }

    fn lookup(&self, url: &str) -> Option<Bytes> {
        #[cfg(feature = "test-hooks")]
        self.hooks.run();
        if let Some(part) = self.part(url) {
            return self.admit(part?);
        }
        if url.starts_with(MISSING_PART) {
            self.placeholdered.fetch_add(1, Ordering::Relaxed);
            return Some(stretched_placeholder(MISSING_PART_SIDE));
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
        let payload = url.strip_prefix("data:image/")?.split_once(";base64,")?.1;
        let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
            .ok()
            .map(Bytes::from)
            .and_then(|bytes| self.admit(bytes))
    }
}

impl Resources {
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

#[derive(Debug, PartialEq)]
enum Probed {
    Image(Bytes),
    Placeholder { width: u32, height: u32 },
}

const MAX_SIDE: u32 = 8_192;
const MAX_PIXELS: u64 = 40_000_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Blitz calls the decoder itself and passes no limits, so this is the only place they can be held.
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

/// A raster, not an SVG: the engine draws an SVG at its own aspect ratio inside the box, and a
/// raster fills it.
fn stretched_placeholder(side: u32) -> Bytes {
    static PNG: std::sync::OnceLock<Bytes> = std::sync::OnceLock::new();
    PNG.get_or_init(|| {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(side, side, image::Rgba([0x88, 0x88, 0x88, 0x88]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("a flat PNG encodes");
        Bytes::from(out)
    })
    .clone()
}

/// An SVG is re-serialised so it names nothing outside itself: usvg's default
/// resolver hands any `<image href>` to `std::fs::read`, so an SVG part could
/// paint a local file into the page.
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

/// An SVG inside an SVG is refused, not recursed into.
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

    fn gif_header(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
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
    fn a_remote_image_resolves_by_its_parsed_url() {
        let table = Resources::new();
        table.insert_remote("https://IMAGES.example.net/a b.png", png(2, 2));
        assert_eq!(
            table
                .resolve("https://images.example.net/a%20b.png")
                .as_deref(),
            Some(png(2, 2).as_slice()),
            "the image inserted, not merely something"
        );
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

#[cfg(feature = "test-hooks")]
mod hooks {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

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

        #[doc(hidden)]
        pub fn release(&self) {
            let mut state = self.state.lock().expect("the gate is never poisoned");
            state.1 = true;
            self.changed.notify_all();
        }

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
    pub fn panic_on_lookup(&self) {
        self.hooks.panic.store(true, Ordering::Relaxed);
    }

    pub fn hold_lookup(&self) -> std::sync::Arc<Gate> {
        let gate = std::sync::Arc::new(Gate::default());
        *self.hooks.hold.lock().expect("hooks are never poisoned") = Some(gate.clone());
        gate
    }
}
