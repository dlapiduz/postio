//! One-shot WebKit reference renders for rendering fidelity (spec 006, T007).
//!
//! ```sh
//! cargo run -p postio-gtk --example capture_reference
//! cargo run -p postio-gtk --example capture_reference -- <out-dir> [fixture …]
//! ```
//!
//! Renders every `designed` fixture in the corpus, as its sender wrote it,
//! and writes one PNG per fixture to
//! `crates/postio-test-support/data/reference/`, with a line per fixture in
//! that directory's `README.md`. Both engines in spec 006's evaluation
//! (research R0), and whichever of them ships, are compared against these
//! with `postio_test_support::fidelity`. The rules live in
//! `specs/006-email-rendering/contracts/fidelity-metric.md`:
//!
//! - **unsanitized**: the reference is what the sender built, not what the
//!   sanitizer lets through. Measuring what the pipeline loses is the point;
//! - **no network, no script**: a CSP `<meta>` admits `data:` images and
//!   inline style only, JavaScript is off, and the session is ephemeral. The
//!   fixtures' own `cid:` parts are inlined as `data:` URIs, so they are
//!   served from the fixture itself;
//! - **800 CSS px, scale 1, light**: a render taken on a scaled output is
//!   area-averaged down to 800 px wide, so the file does not depend on
//!   which monitor took it;
//! - **bundled faces as the default families**: the Blitz arm registers the
//!   same faces, so font choice is not what the comparison measures.
//!
//! It needs a display. It is run by hand, once, while WebKit is still the
//! reader's engine; the PNGs are checked in. It is a development tool:
//! examples are not built into the shipped binary.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::fonts;
use postio_model::test_corpus::{self, Category, Fixture};
use webkit6::prelude::*;

/// The width every reference is taken at, in CSS pixels.
const WIDTH: i32 = 800;

/// The window's height. Kept below every fixture's own height so the
/// snapshot's height is the document's.
const VIEWPORT_HEIGHT: i32 = 64;

/// How long one fixture may take to load and snapshot before the run fails.
const PATIENCE: Duration = Duration::from_secs(15);

/// Admits exactly what a reference needs: inline style and `data:` images.
const CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
                   img-src data:; style-src 'unsafe-inline'; font-src data:\">";

fn main() -> glib::ExitCode {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/postio-test-support/data/reference"));
    let names: Vec<String> = args.collect();
    let fixtures: Vec<&'static Fixture> = if names.is_empty() {
        test_corpus::by_category(Category::Designed)
    } else {
        names.iter().map(|name| test_corpus::load(name)).collect()
    };

    gtk::init().expect("a display to render on");
    adw::init().expect("libadwaita");
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    if let Err(error) = fonts::install() {
        eprintln!("bundled fonts were not installed: {error}");
        return glib::ExitCode::FAILURE;
    }
    std::fs::create_dir_all(&out).expect("the output directory");

    let view = webkit6::WebView::builder()
        .network_session(&webkit6::NetworkSession::new_ephemeral())
        .build();
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_enable_javascript(false);
        settings.set_enable_javascript_markup(false);
        settings.set_auto_load_images(true);
        settings.set_default_font_family(fonts::FAMILIES[0]);
        settings.set_sans_serif_font_family(fonts::FAMILIES[0]);
        settings.set_serif_font_family(fonts::FAMILIES[0]);
        settings.set_monospace_font_family(fonts::FAMILIES[2]);
    }
    let window = gtk::Window::builder()
        .default_width(WIDTH)
        // Short on purpose: a full-document snapshot is never shorter than the
        // viewport, so a tall window would pad every reference with ground.
        .default_height(VIEWPORT_HEIGHT)
        .child(&view)
        .build();
    window.present();

    let run = Rc::new(Run {
        view: view.clone(),
        out: out.clone(),
        queue: RefCell::new(fixtures.into_iter()),
        current: RefCell::new(None),
        written: RefCell::new(Vec::new()),
        failed: RefCell::new(None),
        main_loop: glib::MainLoop::new(None, false),
    });
    {
        let run = run.clone();
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                run.snapshot();
            }
        });
    }
    run.advance();
    run.main_loop.run();

    if let Some(reason) = run.failed.borrow().as_ref() {
        eprintln!("capture failed: {reason}");
        return glib::ExitCode::FAILURE;
    }
    write_readme(&out, &run.written.borrow());
    glib::ExitCode::SUCCESS
}

/// One capture run: the fixtures still to go, and what has been written.
struct Run {
    view: webkit6::WebView,
    out: PathBuf,
    queue: RefCell<std::vec::IntoIter<&'static Fixture>>,
    current: RefCell<Option<&'static Fixture>>,
    written: RefCell<Vec<(String, u32, u32)>>,
    failed: RefCell<Option<String>>,
    main_loop: glib::MainLoop,
}

impl Run {
    /// Load the next fixture, or stop when there is none.
    fn advance(self: &Rc<Self>) {
        let next = self.queue.borrow_mut().next();
        let Some(fixture) = next else {
            self.main_loop.quit();
            return;
        };
        *self.current.borrow_mut() = Some(fixture);
        self.view.load_html(&document(fixture), Some("about:blank"));
        let run = self.clone();
        glib::timeout_add_local_once(PATIENCE, move || {
            if run
                .current
                .borrow()
                .is_some_and(|f| f.name() == fixture.name())
            {
                run.fail(format!(
                    "{}: no snapshot within {PATIENCE:?}",
                    fixture.name()
                ));
            }
        });
    }

    /// Take the loaded fixture's full-document snapshot, then move on.
    fn snapshot(self: &Rc<Self>) {
        let Some(fixture) = *self.current.borrow() else {
            return;
        };
        let run = self.clone();
        self.view.snapshot(
            webkit6::SnapshotRegion::FullDocument,
            webkit6::SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |result| match result {
                Ok(texture) => {
                    let (width, height) = save(&texture, &run.out, fixture.name());
                    println!("{}: {width}x{height}", fixture.name());
                    run.written
                        .borrow_mut()
                        .push((fixture.name().to_owned(), width, height));
                    *run.current.borrow_mut() = None;
                    run.advance();
                }
                Err(error) => run.fail(format!("{}: {error}", fixture.name())),
            },
        );
    }

    fn fail(&self, reason: String) {
        *self.failed.borrow_mut() = Some(reason);
        self.main_loop.quit();
    }
}

/// The fixture's HTML, unsanitized, with its `cid:` parts inlined and the CSP
/// that keeps the render off the network.
fn document(fixture: &Fixture) -> String {
    let parsed = postio_model::mime::parse(fixture.bytes());
    let mut html = parsed.body.html.clone().unwrap_or_else(|| {
        panic!(
            "{} has no HTML body; only designed HTML is captured",
            fixture.name()
        )
    });
    for part in &parsed.parts {
        let Some(content_id) = part.attachment.content_id.as_deref() else {
            continue;
        };
        let data = format!(
            "data:{};base64,{}",
            part.attachment.mime_type,
            glib::base64_encode(&part.content)
        );
        html = replace_ignoring_case(&html, &format!("cid:{content_id}"), &data);
    }
    inject_after_head(&html, CSP)
}

/// Case-insensitive replace, for `cid:` references a sender wrote in any case.
fn replace_ignoring_case(haystack: &str, needle: &str, with: &str) -> String {
    let lower = haystack.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (at, _) in lower.match_indices(&needle_lower) {
        out.push_str(&haystack[last..at]);
        out.push_str(with);
        last = at + needle.len();
    }
    out.push_str(&haystack[last..]);
    out
}

/// Put `meta` inside `<head>`, creating one if the sender wrote none, without
/// disturbing a doctype (which decides quirks mode, and so the layout).
fn inject_after_head(html: &str, meta: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let after_tag = |tag: &str| {
        lower
            .find(tag)
            .and_then(|start| lower[start..].find('>').map(|end| start + end + 1))
    };
    if let Some(at) = after_tag("<head") {
        return format!("{}{meta}{}", &html[..at], &html[at..]);
    }
    if let Some(at) = after_tag("<html") {
        return format!("{}<head>{meta}</head>{}", &html[..at], &html[at..]);
    }
    format!("<head>{meta}</head>{html}")
}

/// Write `texture` as `<name>.png`, area-averaged to [`WIDTH`] pixels wide if
/// it was taken on a scaled output. Returns the written size.
fn save(texture: &gdk::Texture, out: &Path, name: &str) -> (u32, u32) {
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let (pixels, out_width, out_height) = if width == WIDTH as usize {
        let mut tight = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            tight.extend_from_slice(&bytes[row * stride..row * stride + width * 4]);
        }
        (tight, width, height)
    } else {
        downscale(&bytes, stride, width, height, WIDTH as usize)
    };
    let written = gdk::MemoryTexture::new(
        out_width as i32,
        out_height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        out_width * 4,
    );
    let path = out.join(format!("{name}.png"));
    written
        .save_to_png(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    (out_width as u32, out_height as u32)
}

/// Area-average an RGBA image to `target` pixels wide, keeping the aspect.
fn downscale(
    src: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    target: usize,
) -> (Vec<u8>, usize, usize) {
    let scale = width as f64 / target as f64;
    let target_height = ((height as f64) / scale).round().max(1.0) as usize;
    let mut out = vec![0u8; target * target_height * 4];
    for ty in 0..target_height {
        let (y0, y1) = (
            (ty as f64 * scale) as usize,
            (((ty + 1) as f64 * scale) as usize)
                .min(height)
                .max((ty as f64 * scale) as usize + 1),
        );
        for tx in 0..target {
            let (x0, x1) = (
                (tx as f64 * scale) as usize,
                (((tx + 1) as f64 * scale) as usize)
                    .min(width)
                    .max((tx as f64 * scale) as usize + 1),
            );
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = y * stride + x * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u64::from(src[at + channel]);
                    }
                    count += 1;
                }
            }
            let at = (ty * target + tx) * 4;
            for channel in 0..4 {
                out[at + channel] = (sum[channel] / count.max(1)) as u8;
            }
        }
    }
    (out, target, target_height)
}

/// One line per capture: which fixture, which engine version, when.
fn write_readme(out: &Path, written: &[(String, u32, u32)]) {
    let version = format!(
        "{}.{}.{}",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version()
    );
    let date = chrono::Local::now().format("%Y-%m-%d");
    let mut text = String::from(
        "# Reference renders (spec 006, SC-002)\n\n\
         WebKitGTK renders of each `designed` corpus fixture, **unsanitized**, at 800 CSS px,\n\
         scale 1, light, with network and script off and the bundled faces as the default\n\
         families. Written by `cargo run -p postio-gtk --example capture_reference`; compared\n\
         with `postio_test_support::fidelity` under\n\
         `specs/006-email-rendering/contracts/fidelity-metric.md`. Do not regenerate to make\n\
         a comparison pass: a new capture is a new baseline, and says why in its commit.\n\n\
         | Fixture | Size | WebKitGTK | Captured |\n|---|---|---|---|\n",
    );
    for (name, width, height) in written {
        text.push_str(&format!(
            "| `{name}` | {width}x{height} | {version} | {date} |\n"
        ));
    }
    std::fs::write(out.join("README.md"), text).expect("the reference README");
}
