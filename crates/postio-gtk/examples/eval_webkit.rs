//! Arm A of the Blitz-or-WebKit evaluation (spec 006 research R0, T022).
//!
//! ```sh
//! cargo run --release -p postio-gtk --example eval_webkit -- [out-dir]
//! ```
//!
//! Renders the evaluation's fixtures through the production pipeline in the
//! shipped hardened `Reader`, and measures what `docs/notes/2026-09-26-blitz-or-webkit.md`
//! asks of arm A. That covers two things:
//!
//! - **G1, legibility.** Each legibility fixture is drawn in dark. Every
//!   text run's rectangle and colour is read through an isolated-world
//!   script. The pixels behind it are sampled from a full-document snapshot
//!   and counted against the floor: once as the reader draws it today, and
//!   once after the prototype of research R10 is applied. The prototype is
//!   `postio_ui::reader::theme`'s classification and repair, with its
//!   overrides written back through the same script. High contrast is the
//!   dark render against 7:1, because libadwaita's high contrast cannot be
//!   forced from here.
//! - **S1, fidelity.** Each designed fixture is drawn in light, with the
//!   reader's container geometry neutralized (see the protocol's
//!   amendments), and compared with its reference.
//!
//! G2, G3 and S2–S8 are measured by their own tasks.
//!
//! It needs a display; run by hand. It is an evaluation harness, not product
//! code: it is deleted if WebKit is not chosen, and promoted only as far as
//! tasks ask if it is. Nothing here touches the network.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::fonts;
use postio_gtk::reader::{BlobSource, Reader, RemoteImageAllowList};
use postio_model::MessageBody;
use postio_model::test_corpus::{self, Category, Fixture};
use postio_test_support::fidelity::{self, Image};
use postio_ui::reader::theme::{self, MessageFacts, Presentation, Rgb, Theme};
use webkit6::prelude::*;

/// The reader's width for every render, in CSS pixels.
const WIDTH: i32 = 800;

/// The sender every fixture is rendered as, and the one consent allows.
const SENDER: &str = "eval@example.com";

/// The isolated world the harness reads and writes the document from.
const WORLD: &str = "postio-eval";

/// Every text run in the document, one per line:
/// `id \t x \t y \t w \t h \t colour \t ground`, in document CSS pixels.
const RUNS: &str = r#"(() => {
  const out = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let node, next = Number(document.body.dataset.postioEvalNext || 0);
  while ((node = walker.nextNode())) {
    if (!node.textContent.trim()) continue;
    const el = node.parentElement;
    if (!el) continue;
    const style = getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none') continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    const rects = [...range.getClientRects()].filter(r => r.width > 0 && r.height > 0);
    if (!rects.length) continue;
    let ground = '', e = el;
    while (e) {
      const bc = getComputedStyle(e).backgroundColor;
      if (bc && bc !== 'transparent' && !/,\s*0\)$/.test(bc)) { ground = bc; break; }
      e = e.parentElement;
    }
    if (!el.dataset.postioEval) el.dataset.postioEval = String(next++);
    for (const r of rects) {
      out.push([el.dataset.postioEval, Math.floor(r.left + scrollX), Math.floor(r.top + scrollY),
        Math.ceil(r.width), Math.ceil(r.height), style.color, ground].join('\t'));
    }
  }
  document.body.dataset.postioEvalNext = String(next);
  return out.join('\n');
})()"#;

/// What the message says about its own colours: `canvas \t inner \t declares`.
const FACTS: &str = r#"(() => {
  const box = document.querySelector('.postio-body');
  if (!box) return '\t0\t0';
  const canvas = box.querySelector(':scope > .postio-canvas');
  const clear = c => !c || c === 'transparent' || /,\s*0\)$/.test(c);
  const page = canvas ? getComputedStyle(canvas).backgroundColor : getComputedStyle(box).backgroundColor;
  let inner = 0;
  for (const el of box.querySelectorAll('*')) {
    if (el === canvas) continue;
    const s = getComputedStyle(el);
    if (!clear(s.backgroundColor) || (s.backgroundImage && s.backgroundImage !== 'none')) { inner = 1; break; }
  }
  let declares = canvas && (canvas.dataset.postioColorScheme || '').includes('dark') ? 1 : 0;
  for (const sheet of document.styleSheets) {
    let rules; try { rules = sheet.cssRules; } catch (e) { continue; }
    for (const rule of rules) {
      // The sender's own rules only: Postio's reader stylesheet has dark
      // rules of its own, and every sender rule is scoped under the box.
      if (rule.conditionText && /prefers-color-scheme:\s*dark/.test(rule.conditionText)
          && [...rule.cssRules].some(r => (r.selectorText || '').includes('.postio-body'))) declares = 1;
    }
  }
  return [clear(page) ? '' : page, inner, declares].join('\t');
})()"#;

fn main() -> glib::ExitCode {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/eval/webkit"));
    std::fs::create_dir_all(&out).expect("the output directory");
    gtk::init().expect("a display to render on");
    adw::init().expect("libadwaita");
    if let Err(error) = fonts::install() {
        eprintln!("bundled fonts were not installed: {error}");
        return glib::ExitCode::FAILURE;
    }

    let mut report = String::from("# arm A (WebKit)\n\n");
    if std::env::var_os("POSTIO_EVAL_GATES_ONLY").is_some() {
        gates(&mut report);
        std::fs::write(out.join("gates.md"), &report).expect("the report");
        print!("{report}");
        return glib::ExitCode::SUCCESS;
    }

    // ── S1: fidelity, light, chrome geometry neutralized ────────────────────
    report.push_str("## S1 fidelity\n\n| fixture | agreeing | lost block | height ok | match |\n|---|---|---|---|---|\n");
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    let references = PathBuf::from("crates/postio-test-support/data/reference");
    let mut matched = 0;
    let mut judged = 0;
    for fixture in test_corpus::by_category(Category::Designed) {
        let reference_path = references.join(format!("{}.png", fixture.name()));
        if !reference_path.exists() {
            continue;
        }
        // Short: a full-document snapshot is never shorter than the viewport.
        let harness = Harness::new(fixture, 64);
        harness.render();
        harness.eval(NEUTRAL_CHROME);
        let candidate = harness.snapshot();
        candidate.save_png(&out.join(format!("{}.light.png", fixture.name())));
        let comparison = fidelity::compare(&Image::load_png(&reference_path), &candidate);
        judged += 1;
        if comparison.matches() {
            matched += 1;
        }
        report.push_str(&format!(
            "| `{}` | {:.1}% | {} | {} | {} |\n",
            fixture.name(),
            100.0 * comparison.agreeing as f64 / comparison.cells.max(1) as f64,
            comparison.lost_block,
            comparison.height_ok,
            if comparison.matches() {
                "yes"
            } else {
                "**no**"
            }
        ));
        harness.close();
    }
    report.push_str(&format!(
        "\n{matched} of {judged} designed fixtures match.\n\n"
    ));

    if std::env::var_os("POSTIO_EVAL_S1_ONLY").is_some() {
        std::fs::write(out.join("report.md"), &report).expect("the report");
        print!("{report}");
        return glib::ExitCode::SUCCESS;
    }
    // ── G1: legibility, dark (and dark against the high-contrast floor) ─────
    report.push_str(
        "## G1 legibility (dark)\n\n| fixture | presentation | runs | below 4.5 before | below 4.5 after | below 7 after |\n|---|---|---|---|---|---|\n",
    );
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    // Arm A's owed fix, prototyped: WebKit resolves `prefers-color-scheme`
    // from GTK's own setting, not from libadwaita, so the reader's document
    // stays light inside a dark app (spec 006 Context; the editor had the
    // same bug). Measured without it first, below, and reported.
    let follows_libadwaita = {
        let harness = Harness::new(test_corpus::load("plain-text-simple"), 900);
        harness.render();
        let dark = harness.document_is_dark();
        harness.close();
        dark
    };
    // Deprecated since GTK 4.20, and that is part of the finding: the only
    // lever WebKit follows is one GTK is retiring. Arm A's real fix would
    // be the editor's -- the dark flag passed into the document itself --
    // and this stands in for it only to measure the rule.
    #[allow(deprecated)]
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    report.push_str(&format!(
        "Without a fix, the reader's document follows libadwaita's dark: **{follows_libadwaita}**. \
         Measured below with GTK's prefer-dark set, the fix arm A would owe.\n\n"
    ));
    let mut totals = [0usize; 4];
    for fixture in legibility_fixtures() {
        let harness = Harness::new(fixture, 900);
        harness.render();
        let dark = harness.document_is_dark();
        let before = harness.snapshot();
        let runs = harness.runs();
        let below_before = count_below(&before, &runs, 4.5);

        let facts = harness.facts();
        let presentation = theme::classify(
            facts,
            Theme {
                dark: true,
                high_contrast: false,
            },
            false,
        );
        harness.present(presentation, facts);
        let runs = harness.runs();
        harness.repair(&runs, 4.5);
        let after = harness.snapshot();
        let runs = harness.runs();
        let below_after = count_below(&after, &runs, 4.5);
        harness.repair(&runs, 7.0);
        let after_hc = harness.snapshot();
        let below_hc = count_below(&after_hc, &harness.runs(), 7.0);
        after.save_png(&out.join(format!("{}.dark.png", fixture.name())));

        totals[0] += runs.len();
        totals[1] += below_before;
        totals[2] += below_after;
        totals[3] += below_hc;
        report.push_str(&format!(
            "| `{}` | {presentation:?}{} | {} | {below_before} | {below_after} | {below_hc} |\n",
            fixture.name(),
            if dark { "" } else { " (document not dark)" },
            runs.len()
        ));
        harness.close();
    }
    report.push_str(&format!(
        "\nTotal: {} runs; below 4.5 before {}, after {}; below 7 after the high-contrast repair {}.\n",
        totals[0], totals[1], totals[2], totals[3]
    ));

    std::fs::write(out.join("report.md"), &report).expect("the report");
    print!("{report}");
    glib::ExitCode::SUCCESS
}

/// For S1 only: take away what the reader draws around a message, so the
/// comparison measures the sanitizer and the engine, not Postio's chrome.
const NEUTRAL_CHROME: &str = r#"(() => {
  // The canvas is the sender's <body>: it takes the UA's 8px body margin,
  // which the sender's own inline style (lifted onto it) still overrides.
  // A message with no canvas gets the same 8px as padding on its box.
  const s = document.createElement('style');
  s.textContent = 'html, body { background: #ffffff !important; margin: 0 !important; padding: 0 !important; }'
    + ' .postio-body { padding: 8px !important; border: 0 !important; border-radius: 0 !important; margin: 0 !important; max-width: none !important; min-height: 0 !important; }'
    + ' .postio-body:has(> .postio-canvas) { padding: 0 !important; }'
    + ' .postio-canvas { margin: 8px; padding: 0; border-radius: 0; }';
  document.head.appendChild(s);
  return '';
})()"#;

/// G2 and G3: every hostile fixture, unconsented and consented, with its
/// remote URLs pointed at a loopback listener; connections counted, and the
/// time to a finished load taken.
fn gates(report: &mut String) {
    use postio_test_support::listener::Listener;
    report.push_str(
        "## G2 egress and G3 survival\n\n| fixture | consent | loaded in | connections | paths |\n|---|---|---|---|---|\n",
    );
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    let listener = Listener::start();
    listener.control();
    for fixture in test_corpus::by_category(Category::Hostile) {
        for consented in [false, true] {
            let before = listener.count();
            let paths_before = listener.paths().len();
            let harness = Harness::with(
                fixture,
                900,
                |mut body| {
                    body.html = body.html.map(|html| listener.rewrite(&html));
                    body
                },
                consented,
            );
            let loaded = harness.render_timed(Duration::from_secs(20));
            // Time for anything that would fetch to have fetched.
            let until = Instant::now() + Duration::from_millis(800);
            while Instant::now() < until {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(5));
            }
            let paths: Vec<String> = listener.paths()[paths_before..].to_vec();
            report.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                fixture.name(),
                if consented { "allowed" } else { "blocked" },
                loaded.map_or("**never**".to_owned(), |d| format!("{} ms", d.as_millis())),
                listener.count() - before,
                paths.join(" ")
            ));
            harness.close();
        }
    }
}

fn legibility_fixtures() -> Vec<&'static Fixture> {
    if let Ok(only) = std::env::var("POSTIO_EVAL_ONLY") {
        return vec![test_corpus::load(&only)];
    }
    let mut fixtures = test_corpus::by_category(Category::ThemeContrast);
    for name in [
        "html-newsletter",
        "html-designed-three-column",
        "html-legacy-font-center",
        "plain-text-simple",
        "multipart-alternative",
    ] {
        fixtures.push(test_corpus::load(name));
    }
    fixtures
}

/// One text run: its element, its rectangle in CSS px, and its colour.
struct Run {
    id: String,
    rect: [f64; 4],
    colour: Rgb,
    ground: Option<Rgb>,
}

/// Runs whose colour is below `floor` against the pixels behind them.
fn count_below(image: &Image, runs: &[Run], floor: f64) -> usize {
    let scale = image.width as f64 / f64::from(WIDTH);
    if std::env::var_os("POSTIO_EVAL_DEBUG").is_some() {
        eprintln!("image {}x{} scale {scale}", image.width, image.height);
        for run in runs {
            let [x, y, w, h] = run.rect.map(|v| (v * scale).max(0.0) as usize);
            let ground = fidelity::ground_behind(image, x, y, w.max(1), h.max(1));
            let ratio = ground.map(|[r, g, b]| theme::contrast(run.colour, Rgb::from_u8(r, g, b)));
            if ratio.is_some_and(|r| r < floor) {
                eprintln!(
                    "  BELOW {floor}: rect {x},{y} {w}x{h} ground {ground:?} colour {:?} ratio {ratio:?}",
                    run.colour.to_u8()
                );
            }
        }
    }
    runs.iter()
        .filter(|run| {
            let [x, y, w, h] = run.rect.map(|v| (v * scale).max(0.0) as usize);
            fidelity::ground_behind(image, x, y, w.max(1), h.max(1))
                .map(|[r, g, b]| theme::contrast(run.colour, Rgb::from_u8(r, g, b)) < floor - 1e-6)
                .unwrap_or(false)
        })
        .count()
}

/// Its inline parts, for the reader to resolve `cid:` against.
struct Parts(Vec<(String, Vec<u8>, String)>);

impl BlobSource for Parts {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self.0
            .iter()
            .find(|(id, _, _)| id == content_id)
            .map(|(_, bytes, mime)| (bytes.clone(), mime.clone()))
    }
}

/// One fixture in one hardened reader, in a window of its own.
struct Harness {
    window: gtk::Window,
    reader: Reader,
    body: MessageBody,
}

impl Harness {
    fn new(fixture: &Fixture, height: i32) -> Harness {
        Harness::with(fixture, height, |body| body, false)
    }

    /// As [`new`](Self::new), with the body rewritten first and, if
    /// `consented`, the sender allowed remote images.
    fn with(
        fixture: &Fixture,
        height: i32,
        rewrite: impl FnOnce(MessageBody) -> MessageBody,
        consented: bool,
    ) -> Harness {
        let parsed = postio_model::mime::parse(fixture.bytes());
        let parts = parsed
            .parts
            .iter()
            .filter_map(|part| {
                let id = part.attachment.content_id.clone()?;
                Some((id, part.content.clone(), part.attachment.mime_type.clone()))
            })
            .collect();
        let scratch = std::env::temp_dir().join("postio-eval-allowlist");
        let mut allowlist = RemoteImageAllowList::default();
        if consented {
            allowlist.allow(SENDER);
        }
        let reader = Reader::with_allowlist(Rc::new(Parts(parts)), allowlist, scratch);
        let window = gtk::Window::builder()
            .default_width(WIDTH)
            .default_height(height)
            .child(&reader.widget())
            .build();
        window.present();
        Harness {
            window,
            reader,
            body: rewrite(parsed.body),
        }
    }

    /// Render, and how long until the document finished loading, or `None`
    /// if it never did (a crashed or hung web process).
    fn render_timed(&self, patience: Duration) -> Option<Duration> {
        let done = Rc::new(RefCell::new(false));
        let flag = done.clone();
        let handler = self.reader.view().connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        let started = Instant::now();
        self.reader.render(&self.body, Some(SENDER));
        let context = glib::MainContext::default();
        while !*done.borrow() && started.elapsed() < patience {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        self.reader.view().disconnect(handler);
        done.borrow().then(|| started.elapsed())
    }

    fn render(&self) {
        let done = Rc::new(RefCell::new(false));
        let flag = done.clone();
        let handler = self.reader.view().connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                *flag.borrow_mut() = true;
            }
        });
        self.reader.render(&self.body, Some(SENDER));
        wait(|| *done.borrow(), "the document to load");
        self.reader.view().disconnect(handler);
        settle();
    }

    fn eval(&self, script: &str) -> String {
        let result = Rc::new(RefCell::new(None::<String>));
        let slot = result.clone();
        self.reader.view().evaluate_javascript(
            script,
            Some(WORLD),
            None,
            None::<&gtk::gio::Cancellable>,
            move |value| {
                *slot.borrow_mut() = Some(match value {
                    Ok(value) => value.to_str().to_string(),
                    Err(error) => format!("ERROR: {error}"),
                });
            },
        );
        wait(|| result.borrow().is_some(), "a script result");
        let value = result.borrow_mut().take().unwrap_or_default();
        assert!(!value.starts_with("ERROR:"), "{value}");
        settle();
        value
    }

    fn document_is_dark(&self) -> bool {
        let ground = self.eval("getComputedStyle(document.body).backgroundColor");
        theme::parse_css_color(&ground).is_some_and(|(c, _)| theme::relative_luminance(c) < 0.2)
    }

    fn runs(&self) -> Vec<Run> {
        self.eval(RUNS)
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() < 7 {
                    return None;
                }
                Some(Run {
                    id: f[0].to_owned(),
                    rect: [f[1], f[2], f[3], f[4]].map(|v| v.parse().unwrap_or(0.0)),
                    colour: theme::parse_css_color(f[5])?.0,
                    ground: theme::parse_css_color(f[6]).map(|c| c.0),
                })
            })
            .collect()
    }

    fn facts(&self) -> MessageFacts {
        let facts = self.eval(FACTS);
        let f: Vec<&str> = facts.split('\t').collect();
        MessageFacts {
            canvas: f
                .first()
                .and_then(|c| theme::parse_css_color(c))
                .map(|c| c.0),
            inner_background: f.get(1) == Some(&"1"),
            declares_dark: f.get(2) == Some(&"1"),
        }
    }

    /// Put the presentation on the page: paper gets its canvas as a card,
    /// adapted mail loses its page so the reader's ground shows.
    fn present(&self, presentation: Presentation, facts: MessageFacts) {
        let script = match presentation {
            Presentation::Paper => {
                let [r, g, b] = facts.canvas.unwrap_or(Rgb::from_u8(255, 255, 255)).to_u8();
                format!(
                    "(() => {{ const b = document.querySelector('.postio-body'); if (b) b.style.setProperty('background', 'rgb({r},{g},{b})', 'important'); return ''; }})()"
                )
            }
            Presentation::Adapted => "(() => { const c = document.querySelector('.postio-canvas'); if (c) c.style.setProperty('background', 'transparent', 'important'); return ''; })()".to_owned(),
            _ => return,
        };
        self.eval(&script);
    }

    /// Repair every run whose colour misses `floor` against its ground.
    fn repair(&self, runs: &[Run], floor: f64) {
        let fallback = self.ground_fallback();
        let mut script = String::from("(() => {");
        for run in runs {
            let ground = run.ground.unwrap_or(fallback);
            let fixed = theme::repair(run.colour, ground, floor);
            if fixed.to_u8() != run.colour.to_u8() {
                let [r, g, b] = fixed.to_u8();
                script.push_str(&format!(
                    "for (const e of document.querySelectorAll('[data-postio-eval=\"{}\"]')) e.style.setProperty('color', 'rgb({r},{g},{b})', 'important');",
                    run.id
                ));
            }
        }
        script.push_str("return ''; })()");
        if std::env::var_os("POSTIO_EVAL_DEBUG").is_some() {
            eprintln!("repair script: {script}");
            for run in runs.iter().take(3) {
                eprintln!(
                    "run {} colour {:?} ground {:?}",
                    run.id,
                    run.colour.to_u8(),
                    run.ground.map(|g| g.to_u8())
                );
            }
        }
        self.eval(&script);
        if std::env::var_os("POSTIO_EVAL_DEBUG").is_some() {
            for run in self.runs().iter().take(3) {
                eprintln!("after: run {} colour {:?}", run.id, run.colour.to_u8());
            }
        }
    }

    fn ground_fallback(&self) -> Rgb {
        let ground = self.eval("getComputedStyle(document.body).backgroundColor");
        theme::parse_css_color(&ground)
            .map(|c| c.0)
            .unwrap_or(Rgb::from_u8(255, 255, 255))
    }

    fn snapshot(&self) -> Image {
        let result = Rc::new(RefCell::new(None));
        let slot = result.clone();
        self.reader.view().snapshot(
            webkit6::SnapshotRegion::FullDocument,
            webkit6::SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |texture| *slot.borrow_mut() = Some(texture),
        );
        wait(|| result.borrow().is_some(), "a snapshot");
        let texture = result
            .borrow_mut()
            .take()
            .expect("waited")
            .expect("a snapshot");
        image_of(&texture)
    }

    fn close(self) {
        self.window.destroy();
        settle();
    }
}

fn image_of(texture: &gdk::Texture) -> Image {
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        rgba.extend_from_slice(&bytes[row * stride..row * stride + width * 4]);
    }
    let image = Image::from_rgba(width, height, rgba);
    if width == WIDTH as usize {
        image
    } else {
        downscale(&image)
    }
}

/// Area-average to [`WIDTH`] wide, so a scaled output compares at 1x.
fn downscale(image: &Image) -> Image {
    let scale = image.width as f64 / f64::from(WIDTH);
    let (w, h) = (
        WIDTH as usize,
        ((image.height as f64) / scale).round().max(1.0) as usize,
    );
    let mut rgba = vec![0u8; w * h * 4];
    for ty in 0..h {
        let (y0, y1) = (
            (ty as f64 * scale) as usize,
            ((((ty + 1) as f64) * scale) as usize)
                .clamp((ty as f64 * scale) as usize + 1, image.height),
        );
        for tx in 0..w {
            let (x0, x1) = (
                (tx as f64 * scale) as usize,
                ((((tx + 1) as f64) * scale) as usize)
                    .clamp((tx as f64 * scale) as usize + 1, image.width),
            );
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * image.width + x) * 4;
                    for (total, value) in sum.iter_mut().zip(&image.rgba[at..at + 4]) {
                        *total += u64::from(*value);
                    }
                    n += 1;
                }
            }
            let at = (ty * w + tx) * 4;
            for (slot, total) in rgba[at..at + 4].iter_mut().zip(sum) {
                *slot = (total / n.max(1)) as u8;
            }
        }
    }
    Image::from_rgba(w, h, rgba)
}

/// Turn the main loop until `done`, or give up loudly.
fn wait(done: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let context = glib::MainContext::default();
    while !done() {
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        context.iteration(true);
    }
}

/// A few frames for style and paint to land.
fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(120);
    while Instant::now() < until {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
}
