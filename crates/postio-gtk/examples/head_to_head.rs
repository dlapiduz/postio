//! Performance head to head of the two reading engines (spec 006 research
//! R0; the protocol is in `docs/notes/2026-09-26-blitz-or-webkit.md`).
//!
//! ```sh
//! cargo run --release -p postio-gtk --example head_to_head -- webkit
//! cargo run --release -p postio-gtk --example head_to_head -- blitz
//! ```
//!
//! One engine per process, so neither is billed for the other's helpers.
//! Both draw into the same GTK window through the same measure:
//! - **webkit**: the shipped hardened `Reader`;
//! - **blitz**: a minimal reading widget that lays a document out once and
//!   paints only the visible viewport into a texture, with fonts
//!   memory-mapped rather than read.
//!
//! "Presented" means the first frame the window's frame clock paints after
//! the content is ready: for WebKit, after its load finished; for Blitz,
//! after its texture was set. It needs a display, is run by hand, and
//! touches no network.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::fonts;
use postio_gtk::reader::view::ThreadMessage;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::MessageBody;
use postio_model::test_corpus;
use postio_ui::reader::document::{self, Rendering, Sheet};
use postio_ui::reader::thread::{Entry, conversation_document};
use webkit6::prelude::*;

const WIDTH: i32 = 800;
const HEIGHT: i32 = 900;

/// The openings H2 times, in order: designed, theme, legacy, plain and
/// international mail, then two threads.
const MESSAGES: &[&str] = &[
    "html-newsletter",
    "html-designed-three-column",
    "html-transactional-receipt",
    "html-class-styled",
    "html-responsive-media",
    "transactional-shipping-notice",
    "html-white-page-reply",
    "html-dark-aware",
    "html-dark-text-no-background",
    "html-transparent-logo",
    "html-legacy-font-center",
    "html-legacy-table-attrs",
    "plain-text-simple",
    "multipart-alternative",
    "html-cjk-emoji",
    "html-rtl-mixed",
    "inline-image-cid",
    "html-very-tall",
];

fn main() -> glib::ExitCode {
    let started = Instant::now();
    let engine = std::env::args().nth(1).unwrap_or_default();
    gtk::init().expect("a display");
    adw::init().expect("libadwaita");
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    fonts::install().expect("bundled fonts");

    let window = gtk::Window::builder()
        .default_width(WIDTH)
        .default_height(HEIGHT)
        .build();
    let surface: Rc<dyn Surface> = match engine.as_str() {
        "webkit" => Rc::new(WebKitSurface::new()),
        "blitz" => Rc::new(BlitzSurface::new()),
        _ => {
            eprintln!("usage: head_to_head webkit|blitz");
            return glib::ExitCode::FAILURE;
        }
    };
    window.set_child(Some(&surface.widget()));
    window.present();
    let clock = Presented::watch(&window);

    // ── H1: cold start ──────────────────────────────────────────────────
    let first = message(MESSAGES[0]);
    surface.show(&first);
    wait_until(|| surface.ready(), "the first message");
    clock.next_frame();
    let cold = started.elapsed();
    let idle_memory = memory_mib();

    // ── H2: opening messages ────────────────────────────────────────────
    let mut openings: Vec<(String, Duration)> = Vec::new();
    let mut shows: Vec<Shown> = MESSAGES.iter().skip(1).map(|name| message(name)).collect();
    shows.push(thread(10));
    shows.push(thread(50));
    for shown in &shows {
        let asked = Instant::now();
        surface.show(shown);
        wait_until(|| surface.ready(), &shown.name);
        clock.next_frame();
        openings.push((shown.name.clone(), asked.elapsed()));
    }
    let session_memory = memory_mib();

    // ── H3: scrolling the very tall message ─────────────────────────────
    surface.show(&message("html-very-tall"));
    wait_until(|| surface.ready(), "the tall message");
    clock.next_frame();
    let intervals = {
        // Scroll one step per frame from a tick callback, which keeps the
        // frame clock running for both engines; time the frames it paints.
        let stamps = Rc::new(RefCell::new(Vec::<Instant>::new()));
        let recorder = stamps.clone();
        let handler = clock.clock.connect_after_paint(move |_| {
            recorder.borrow_mut().push(Instant::now());
        });
        let steps = Rc::new(Cell::new(0u32));
        let counted = steps.clone();
        let driver = surface.clone();
        let tick = window.add_tick_callback(move |_, _| {
            driver.scroll_by(300.0);
            counted.set(counted.get() + 1);
            glib::ControlFlow::Continue
        });
        wait_until(|| steps.get() > 120, "120 scrolled frames");
        tick.remove();
        clock.clock.disconnect(handler);
        let stamps = stamps.borrow();
        stamps.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>()
    };

    // ── H4: theme switch ────────────────────────────────────────────────
    surface.show(&message("html-newsletter"));
    wait_until(|| surface.ready(), "the newsletter");
    clock.next_frame();
    let asked = Instant::now();
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    #[allow(deprecated)]
    if let Some(settings) = gtk::Settings::default() {
        // WebKit follows only this, deprecated, setting (see the note).
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    surface.theme_changed(true);
    wait_until(|| surface.ready(), "the dark repaint");
    clock.next_frame();
    let theme = asked.elapsed();

    let cpu = cpu_seconds();
    let final_memory = memory_mib();

    // ── report ───────────────────────────────────────────────────────────
    let mut times: Vec<Duration> = openings.iter().map(|(_, d)| *d).collect();
    times.sort();
    let mut frames = intervals.clone();
    frames.sort();
    let p = |v: &[Duration], q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!("# {engine}\n");
    println!("| measure | value |\n|---|---|");
    println!("| H1 cold start | {:.0} ms |", ms(cold));
    println!("| H2 open, median | {:.1} ms |", ms(p(&times, 0.5)));
    println!("| H2 open, worst | {:.1} ms |", ms(*times.last().unwrap()));
    for (name, d) in &openings {
        if name.starts_with("thread") || name == "html-very-tall" {
            println!("| H2 {name} | {:.1} ms |", ms(*d));
        }
    }
    println!(
        "| H3 frame interval, median | {:.1} ms |",
        ms(p(&frames, 0.5))
    );
    println!(
        "| H3 frame interval, p95 | {:.1} ms |",
        ms(p(&frames, 0.95))
    );
    println!(
        "| H3 frame interval, worst | {:.1} ms |",
        ms(*frames.last().unwrap())
    );
    println!("| H4 theme switch | {:.1} ms |", ms(theme));
    println!(
        "| H5 memory at idle after the first message | {:.0} MiB |",
        idle_memory.0
    );
    println!(
        "| H5 memory after the session | {:.0} MiB |",
        session_memory.0
    );
    println!(
        "| H5 memory at the end | {:.0} MiB across {} processes |",
        final_memory.0, final_memory.1
    );
    println!("| H6 CPU over the run | {cpu:.2} s |");
    window.destroy();
    glib::ExitCode::SUCCESS
}

/// Something to show: a single message's body, or a thread.
struct Shown {
    name: String,
    body: MessageBody,
    parts: HashMap<String, (Vec<u8>, String)>,
    thread: Option<usize>,
}

fn message(name: &str) -> Shown {
    let parsed = postio_model::mime::parse(test_corpus::load(name).bytes());
    let parts = parsed
        .parts
        .iter()
        .filter_map(|p| {
            Some((
                p.attachment.content_id.clone()?,
                (p.content.clone(), p.attachment.mime_type.clone()),
            ))
        })
        .collect();
    Shown {
        name: name.to_owned(),
        body: parsed.body,
        parts,
        thread: None,
    }
}

fn thread(n: usize) -> Shown {
    Shown {
        name: format!("thread of {n}"),
        body: MessageBody::default(),
        parts: HashMap::new(),
        thread: Some(n),
    }
}

fn thread_body(index: usize) -> MessageBody {
    MessageBody {
        text: None,
        html: Some(format!(
            "<p>Message {index} from Ren Ishida. The scope has three parts and I will go through \
             each so there are no surprises on the day.</p><p><b>Diagnostics.</b> We place two \
             continuous monitors for 48 hours and pull a soil-gas reading at the slab.</p>\
             <blockquote><p>Quoted from the message before it, so the folding path has something \
             to fold.</p></blockquote>"
        )),
    }
}

/// A reading surface under test.
trait Surface {
    fn widget(&self) -> gtk::Widget;
    fn show(&self, shown: &Shown);
    /// Whether what was last shown is ready to be presented.
    fn ready(&self) -> bool;
    fn scroll_by(&self, dy: f64);
    fn theme_changed(&self, dark: bool);
}

// ── WebKit ──────────────────────────────────────────────────────────────

struct Parts(RefCell<HashMap<String, (Vec<u8>, String)>>);

impl BlobSource for Parts {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self.0.borrow().get(content_id).cloned()
    }
}

struct WebKitSurface {
    reader: Reader,
    parts: Rc<Parts>,
    loaded: Rc<Cell<bool>>,
}

impl WebKitSurface {
    fn new() -> WebKitSurface {
        let parts = Rc::new(Parts(RefCell::new(HashMap::new())));
        let reader = Reader::with_allowlist(
            parts.clone(),
            RemoteImageAllowList::default(),
            std::env::temp_dir().join("postio-h2h-allowlist"),
        );
        let loaded = Rc::new(Cell::new(false));
        let flag = loaded.clone();
        reader.view().connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                flag.set(true);
            }
        });
        WebKitSurface {
            reader,
            parts,
            loaded,
        }
    }
}

impl Surface for WebKitSurface {
    fn widget(&self) -> gtk::Widget {
        self.reader.widget()
    }

    fn show(&self, shown: &Shown) {
        self.loaded.set(false);
        *self.parts.0.borrow_mut() = shown.parts.clone();
        match shown.thread {
            None => self.reader.render(&shown.body, Some("h2h@example.com")),
            Some(n) => {
                let messages: Vec<ThreadMessage> = (0..n)
                    .map(|i| ThreadMessage {
                        scope: (i + 1).to_string(),
                        sender: "Ren Ishida".to_owned(),
                        address: "ren.ishida@example.net".to_owned(),
                        when: "24 Aug".to_owned(),
                        recipients: String::new(),
                        cc: String::new(),
                        preview: "preview".to_owned(),
                        expanded: true,
                        absent: false,
                        latest: i + 1 == n,
                        draft: false,
                        mine: false,
                        body: thread_body(i),
                    })
                    .collect();
                self.reader.render_thread(&messages);
            }
        }
    }

    fn ready(&self) -> bool {
        self.loaded.get()
    }

    fn scroll_by(&self, dy: f64) {
        self.reader.view().evaluate_javascript(
            &format!("window.scrollBy(0, {dy})"),
            Some("postio-h2h"),
            None,
            None::<&gtk::gio::Cancellable>,
            |_| {},
        );
    }

    fn theme_changed(&self, _dark: bool) {
        // WebKit restyles on its own when GTK's preference changes; ready
        // means the document's own ground has gone dark.
        self.loaded.set(false);
        let flag = self.loaded.clone();
        let view = self.reader.view().clone();
        glib::timeout_add_local(Duration::from_millis(1), move || {
            let answer = flag.clone();
            view.evaluate_javascript(
                "getComputedStyle(document.body).backgroundColor",
                Some("postio-h2h"),
                None,
                None::<&gtk::gio::Cancellable>,
                move |value| {
                    if let Ok(value) = value
                        && let Some((c, _)) =
                            postio_render::theme::parse_css_color(&value.to_str())
                        && postio_render::theme::relative_luminance(c) < 0.2
                    {
                        answer.set(true);
                    }
                },
            );
            if flag.get() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

// ── Blitz ───────────────────────────────────────────────────────────────

struct BlitzSurface {
    picture: gtk::Picture,
    fonts: parley::fontique::Collection,
    document: RefCell<Option<blitz_dom::BaseDocument>>,
    html: RefCell<String>,
    parts: RefCell<Arc<HashMap<String, Vec<u8>>>>,
    offset: Cell<f64>,
    dark: Cell<bool>,
    ready: Cell<bool>,
}

impl BlitzSurface {
    fn new() -> BlitzSurface {
        let picture = gtk::Picture::new();
        picture.set_size_request(WIDTH, HEIGHT);
        picture.set_can_shrink(true);
        BlitzSurface {
            picture,
            fonts: font_collection(),
            document: RefCell::new(None),
            html: RefCell::new(String::new()),
            parts: RefCell::new(Arc::new(HashMap::new())),
            offset: Cell::new(0.0),
            dark: Cell::new(false),
            ready: Cell::new(false),
        }
    }

    fn scale(&self) -> f64 {
        // `POSTIO_H2H_SCALE` forces a device scale, to see what a HiDPI
        // display costs a CPU raster when the test compositor is at 1x.
        if let Some(forced) = std::env::var("POSTIO_H2H_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            return forced;
        }
        self.picture
            .native()
            .and_then(|n| n.surface())
            .map_or(1.0, |s| s.scale())
    }

    fn lay_out(&self) {
        use blitz_traits::shell::{ColorScheme, Viewport};
        let scale = self.scale() as f32;
        let config = blitz_dom::DocumentConfig {
            font_ctx: Some(blitz_dom::FontContext {
                collection: self.fonts.clone(),
                source_cache: Default::default(),
            }),
            viewport: Some(Viewport::new(
                (WIDTH as f32 * scale) as u32,
                (HEIGHT as f32 * scale) as u32,
                scale,
                if self.dark.get() {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                },
            )),
            base_url: Some("postio-message://message/".to_owned()),
            ua_stylesheets: Some(vec![
                "head, title, meta, link, style, script { display: none }".to_owned(),
            ]),
            net_provider: Some(Arc::new(Resources {
                parts: self.parts.borrow().clone(),
            })),
            ..Default::default()
        };
        let mut doc = blitz_html::HtmlDocument::from_html(&self.html.borrow(), config).into_inner();
        doc.resolve(0.0);
        *self.document.borrow_mut() = Some(doc);
    }

    /// Paint the visible viewport only, at the display's scale.
    fn paint(&self) {
        use anyrender::ImageRenderer;
        let mut guard = self.document.borrow_mut();
        let Some(doc) = guard.as_mut() else { return };
        let scale = self.scale();
        let (w, h) = (
            (f64::from(WIDTH) * scale) as u32,
            (f64::from(HEIGHT) * scale) as u32,
        );
        let height = f64::from(doc.root_element().final_layout().size.height);
        let offset = self
            .offset
            .get()
            .clamp(0.0, (height - f64::from(HEIGHT)).max(0.0));
        self.offset.set(offset);
        let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(w, h);
        let mut buffer = vec![0u8; (w * h * 4) as usize];
        renderer.render(
            |scene| blitz_paint::paint_scene(scene, doc, scale, w, h, 0, (offset * scale) as u32),
            &mut buffer,
        );
        let texture = gdk::MemoryTexture::new(
            w as i32,
            h as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(buffer),
            (w * 4) as usize,
        );
        self.picture.set_paintable(Some(&texture));
    }
}

impl Surface for BlitzSurface {
    fn widget(&self) -> gtk::Widget {
        self.picture.clone().upcast()
    }

    fn show(&self, shown: &Shown) {
        self.ready.set(false);
        let html = match shown.thread {
            None => {
                let rendered = document::body_html_in(
                    &shown.body,
                    postio_body::RemoteImages::Blocked,
                    document::opening_rendering(),
                    None,
                );
                let sheet = document::sheet_for(
                    Rendering::Original,
                    document::suits_reader_view(&shown.body),
                );
                document::document_for(
                    &rendered.html,
                    &rendered.styles,
                    postio_body::RemoteImages::Blocked,
                    sheet,
                )
            }
            Some(n) => {
                let bodies: Vec<(String, String, String)> = (0..n)
                    .map(|i| {
                        let scope = (i + 1).to_string();
                        let r = document::body_html_in(
                            &thread_body(i),
                            postio_body::RemoteImages::Blocked,
                            document::opening_rendering(),
                            Some(&scope),
                        );
                        (scope, r.html, r.styles)
                    })
                    .collect();
                let entries: Vec<Entry<'_>> = bodies
                    .iter()
                    .enumerate()
                    .map(|(i, (scope, body, styles))| Entry {
                        scope,
                        sender: "Ren Ishida",
                        address: "ren.ishida@example.net",
                        when: "24 Aug",
                        preview: "preview",
                        expanded: true,
                        draft: false,
                        mine: false,
                        latest: i + 1 == n,
                        blocked: 0,
                        body,
                        recipients: "",
                        cc: "",
                        styles,
                    })
                    .collect();
                conversation_document(&entries, postio_body::RemoteImages::Blocked, Sheet::Theme)
            }
        };
        *self.html.borrow_mut() = html;
        *self.parts.borrow_mut() = Arc::new(
            shown
                .parts
                .iter()
                .map(|(k, (b, _))| (k.clone(), b.clone()))
                .collect(),
        );
        self.offset.set(0.0);
        self.lay_out();
        self.paint();
        self.ready.set(true);
    }

    fn ready(&self) -> bool {
        self.ready.get()
    }

    fn scroll_by(&self, dy: f64) {
        self.offset.set(self.offset.get() + dy);
        self.paint();
    }

    fn theme_changed(&self, dark: bool) {
        self.ready.set(false);
        self.dark.set(dark);
        self.lay_out();
        self.paint();
        self.ready.set(true);
    }
}

/// The document's only sources: its own parts, its embedded images, and the
/// bundled faces.
struct Resources {
    parts: Arc<HashMap<String, Vec<u8>>>,
}

impl blitz_traits::net::NetProvider for Resources {
    fn fetch(
        &self,
        _doc: usize,
        request: blitz_traits::net::Request,
        handler: Box<dyn blitz_traits::net::NetHandler>,
    ) {
        let url = request.url.as_str().to_owned();
        if let Some(rest) = url.strip_prefix("postio-cid:") {
            let id = postio_body::sanitize::percent_decode(rest.rsplit('/').next().unwrap_or(rest));
            if let Some(bytes) = self.parts.get(&id) {
                handler.bytes(url, blitz_traits::net::Bytes::from(bytes.clone()));
            }
            return;
        }
        if let Some(face) = url.strip_prefix(document::FONT_SCHEME)
            && let Some(bytes) = document::font_bytes(face.trim_start_matches([':', '/']))
        {
            handler.bytes(url, blitz_traits::net::Bytes::from_static(bytes));
        }
    }
}

/// A font file, memory-mapped.
#[allow(unsafe_code)]
fn map_font(file: &std::fs::File) -> std::io::Result<memmap2::Mmap> {
    // SAFETY: font files are read-only system files; one changed underneath
    // the map is the hazard every font stack that maps them accepts.
    unsafe { memmap2::Mmap::map(file) }
}

/// Bundled faces first as the generic families, then every installed face,
/// memory-mapped: only the pages a render touches become resident.
fn font_collection() -> parley::fontique::Collection {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily};
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let (mut sans, mut mono) = (None, None);
    for face in document::FACES {
        for (family, _) in collection.register_fonts(Blob::new(Arc::new(face.bytes) as _), None) {
            match collection.family_name(family) {
                Some("Barlow") => sans = Some(family),
                Some("IBM Plex Mono") => mono = Some(family),
                _ => {}
            }
        }
    }
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut seen = std::collections::HashSet::new();
    for face in database.faces() {
        if let fontdb::Source::File(path) = &face.source
            && seen.insert(path.clone())
            && let Ok(file) = std::fs::File::open(path)
            && let Ok(map) = map_font(&file)
        {
            collection.register_fonts(Blob::new(Arc::new(map) as _), None);
        }
    }
    if let Some(sans) = sans {
        for generic in [
            GenericFamily::SansSerif,
            GenericFamily::Serif,
            GenericFamily::SystemUi,
        ] {
            collection.set_generic_families(generic, std::iter::once(sans));
        }
    }
    if let Some(mono) = mono {
        collection.set_generic_families(GenericFamily::Monospace, std::iter::once(mono));
    }
    collection
}

// ── the instrument ──────────────────────────────────────────────────────

/// The window's frame clock, to wait for the next presented frame.
struct Presented {
    clock: gdk::FrameClock,
    frames: Rc<Cell<u64>>,
}

impl Presented {
    fn watch(window: &gtk::Window) -> Presented {
        wait_until(|| window.frame_clock().is_some(), "a frame clock");
        let clock = window.frame_clock().expect("waited");
        let frames = Rc::new(Cell::new(0));
        let counter = frames.clone();
        clock.connect_after_paint(move |_| counter.set(counter.get() + 1));
        Presented { clock, frames }
    }

    fn next_frame(&self) {
        let now = self.frames.get();
        self.clock.request_phase(gdk::FrameClockPhase::PAINT);
        wait_until(|| self.frames.get() > now, "a presented frame");
    }
}

fn wait_until(done: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let context = glib::MainContext::default();
    while !done() {
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        context.iteration(true);
    }
}

/// This process and every descendant, by pid.
fn family() -> Vec<u32> {
    let me = std::process::id();
    let mut parents: HashMap<u32, u32> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>()
                && let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                && let Some(after) = stat.rsplit_once(')').map(|(_, rest)| rest)
                && let Some(ppid) = after.split_whitespace().nth(1).and_then(|p| p.parse().ok())
            {
                parents.insert(pid, ppid);
            }
        }
    }
    let mut out = vec![me];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        out.extend(
            parents
                .iter()
                .filter(|(_, p)| **p == parent)
                .map(|(c, _)| *c),
        );
        i += 1;
    }
    out
}

/// Pss summed over this process and its descendants, and how many.
fn memory_mib() -> (f64, usize) {
    let pids = family();
    let kib: f64 = pids
        .iter()
        .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok())
        .filter_map(|text| {
            text.lines()
                .find(|line| line.starts_with("Pss:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
        })
        .sum();
    (kib / 1024.0, pids.len())
}

/// User + system CPU seconds over this process and its live descendants.
fn cpu_seconds() -> f64 {
    let ticks = 100.0;
    family()
        .iter()
        .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok())
        .filter_map(|stat| {
            let after = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            Some(after.get(11)?.parse::<f64>().ok()? + after.get(12)?.parse::<f64>().ok()?)
        })
        .sum::<f64>()
        / ticks
}
