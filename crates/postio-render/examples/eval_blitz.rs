//! Arm B of the Blitz-or-WebKit evaluation (spec 006 research R0, T023).
//!
//! ```sh
//! cargo run --release -p postio-render --example eval_blitz -- [out-dir]
//! ```
//!
//! The same fixtures as arm A, through the same production pipeline, drawn by
//! Blitz in-process on the CPU. No display, no network crate, and fonts
//! through `fontdb` rather than fontconfig. Measures, as arm A does:
//!
//! - **G1**: every text run's rectangle and colour from Blitz's own layout,
//!   the pixels behind it sampled, before and after research R10's rule
//!   (`postio_render::theme`), applied as style overrides and a restyle.
//! - **S1**: each designed fixture in light, with the same neutralized
//!   container geometry as arm A, against its reference.
//!
//! Prototype depth: it is an evaluation harness, not the renderer the plan
//! describes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyrender::ImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext, LocalName, NodeId};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, GenericFamily};
use parley::layout::PositionedLayoutItem;
use postio_body::RemoteImages;
use postio_model::MessageBody;
use postio_model::test_corpus::{self, Category, Fixture};
use postio_render::theme::{self, MessageFacts, Presentation, Rgb, Theme};
use postio_test_support::fidelity::{self, Image};
use postio_ui::reader::document::{self, Rendering};

/// The reader's width for every render, in CSS pixels.
const WIDTH: u32 = 800;

/// What Blitz's user-agent sheet leaves out that every browser has.
const HEAD_IS_NOT_CONTENT: &str = "head, title, meta, link, style, script { display: none }";

/// The same neutral geometry arm A injects for S1.
const NEUTRAL_CHROME: &str = "html, body { background: #ffffff !important; margin: 0 !important; padding: 0 !important; }\
 .postio-body { padding: 8px !important; border: 0 !important; border-radius: 0 !important; margin: 0 !important; max-width: none !important; min-height: 0 !important; }\
 .postio-body:has(> .postio-canvas) { padding: 0 !important; }\
 .postio-canvas { margin: 8px; padding: 0; border-radius: 0; }";

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/eval/blitz"));
    std::fs::create_dir_all(&out).expect("the output directory");
    let fonts = font_set();
    // Sharpness: one fixture at 2x natively, and at 1x enlarged to 2x as a
    // compositor would -- the spike's texture -- cropped to the same region.
    if let Ok(name) = std::env::var("POSTIO_EVAL_SHARPNESS") {
        let message = Message::of(test_corpus::load(&name));
        SCALE.with(|s| s.set(2.0));
        let native = paint(&mut message.lay_out(&fonts, false, None));
        SCALE.with(|s| s.set(1.0));
        let stretched = enlarge(&paint(&mut message.lay_out(&fonts, false, None)));
        let (x, y, w, h) = (180, 40, 620, 300);
        crop(&native, x, y, w, h).save_png(&out.join("sharp-2x-native.png"));
        crop(&stretched, x, y, w, h).save_png(&out.join("sharp-1x-stretched.png"));
        println!(
            "wrote {}/sharp-2x-native.png and sharp-1x-stretched.png",
            out.display()
        );
        return;
    }
    // A minimal document, drawn on its own: how an engine defect is shown to
    // be the engine's rather than the fixture's.
    if let Ok(html) = std::env::var("POSTIO_EVAL_PROBE") {
        let message = Message {
            body: MessageBody::default(),
            parts: HashMap::new(),
            styles: String::new(),
            html,
        };
        let mut doc = message.lay_out(&fonts, false, None);
        let image = paint(&mut doc);
        let dark = image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] < 80 && p[1] < 80 && p[2] < 80)
            .count();
        println!(
            "probe: {}x{}, {dark} dark pixels",
            image.width, image.height
        );
        image.save_png(&out.join("probe.png"));
        return;
    }
    let mut report = String::from("# arm B (Blitz)\n\n");
    if std::env::var_os("POSTIO_EVAL_COST_ONLY").is_some() {
        cost(&fonts, &mut report);
        std::fs::write(out.join("cost.md"), &report).expect("the report");
        print!("{report}");
        return;
    }
    if std::env::var_os("POSTIO_EVAL_GATES_ONLY").is_some() {
        gates(&fonts, &mut report);
        std::fs::write(out.join("gates.md"), &report).expect("the report");
        print!("{report}");
        return;
    }

    // ── S1 ────────────────────────────────────────────────────────────────
    report.push_str("## S1 fidelity\n\n| fixture | agreeing | lost block | height ok | match |\n|---|---|---|---|---|\n");
    let references = PathBuf::from("crates/postio-test-support/data/reference");
    let (mut matched, mut judged) = (0, 0);
    for fixture in test_corpus::by_category(Category::Designed) {
        let reference_path = references.join(format!("{}.png", fixture.name()));
        if !reference_path.exists() {
            continue;
        }
        let message = Message::of(fixture);
        // Counterfactual only, and labelled so in the report: what S1 would
        // be if blitz#504 (collapsed borders on borderless tables) were fixed.
        let chrome = if std::env::var_os("POSTIO_EVAL_COUNTERFACTUAL_504").is_some() {
            format!("{NEUTRAL_CHROME} table {{ border-collapse: separate !important; }}")
        } else {
            NEUTRAL_CHROME.to_owned()
        };
        let mut doc = message.lay_out(&fonts, false, Some(&chrome));
        let candidate = paint(&mut doc);
        candidate.save_png(&out.join(format!("{}.light.png", fixture.name())));
        let comparison = fidelity::compare(&Image::load_png(&reference_path), &candidate);
        judged += 1;
        if comparison.matches() {
            matched += 1;
        } else {
            fidelity::diff_image(&Image::load_png(&reference_path), &comparison)
                .save_png(&out.join(format!("{}.diff.png", fixture.name())));
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
    }
    report.push_str(&format!(
        "\n{matched} of {judged} designed fixtures match.\n\n"
    ));

    // ── G1 ────────────────────────────────────────────────────────────────
    report.push_str(
        "## G1 legibility (dark)\n\n| fixture | presentation | runs | below 4.5 before | below 4.5 after | below 7 after |\n|---|---|---|---|---|---|\n",
    );
    let mut totals = [0usize; 4];
    for fixture in legibility_fixtures() {
        let message = Message::of(fixture);
        let mut doc = message.lay_out(&fonts, true, None);
        let before = paint(&mut doc);
        let found = runs_of(&doc);
        let below_before = count_below(&before, &found, 4.5);

        // Each step's overrides are a stylesheet on a fresh layout (see
        // `Overrides`): the presentation, then the repair against the ground
        // the presentation left, then the high-contrast repair on top.
        let facts = facts(&doc, &message);
        let presentation = theme::classify(
            facts,
            Theme {
                dark: true,
                high_contrast: false,
            },
            false,
        );
        let mut overrides = Overrides::default();
        overrides.present(presentation, facts);
        let doc = message.lay_out(&fonts, true, Some(&overrides.css()));
        overrides.repair(&doc, &runs_of(&doc), 4.5);
        let mut doc = message.lay_out(&fonts, true, Some(&overrides.css()));
        let after = paint(&mut doc);
        let found = runs_of(&doc);
        let below_after = count_below(&after, &found, 4.5);
        overrides.repair(&doc, &found, 7.0);
        let mut doc = message.lay_out(&fonts, true, Some(&overrides.css()));
        let after_hc = paint(&mut doc);
        let below_hc = count_below(&after_hc, &runs_of(&doc), 7.0);
        after.save_png(&out.join(format!("{}.dark.png", fixture.name())));

        totals[0] += found.len();
        totals[1] += below_before;
        totals[2] += below_after;
        totals[3] += below_hc;
        report.push_str(&format!(
            "| `{}` | {presentation:?} | {} | {below_before} | {below_after} | {below_hc} |\n",
            fixture.name(),
            found.len()
        ));
    }
    report.push_str(&format!(
        "\nTotal: {} runs; below 4.5 before {}, after {}; below 7 after the high-contrast repair {}.\n",
        totals[0], totals[1], totals[2], totals[3]
    ));

    std::fs::write(out.join("report.md"), &report).expect("the report");
    print!("{report}");
}

/// G2 and G3, as arm A measures them, plus the local-file probe research
/// R5 predicts this arm needs: usvg's default resolver reads any path an
/// SVG's `<image>` names.
fn gates(fonts: &FontSet, report: &mut String) {
    use postio_test_support::listener::Listener;
    use std::time::Instant;
    report.push_str(
        "## G2 egress and G3 survival\n\n| fixture | consent | laid out and painted in | panicked | connections | probe pixels |\n|---|---|---|---|---|---|\n",
    );
    // The probe: a magenta PNG where the hostile SVG looks. If any of it is
    // painted, a message read a local file.
    let probe = std::path::Path::new("/tmp/postio-svg-local-file-probe.png");
    let magenta = Image::from_rgba(40, 40, [255u8, 0, 255, 255].repeat(40 * 40));
    magenta.save_png(probe);
    let listener = Listener::start();
    listener.control();
    for fixture in test_corpus::by_category(Category::Hostile) {
        for remote in [RemoteImages::Blocked, RemoteImages::Allowed] {
            let before = listener.count();
            let message = Message::with(fixture, |html| listener.rewrite(html), remote);
            let started = Instant::now();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut doc = message.lay_out(fonts, false, None);
                paint(&mut doc)
            }));
            let elapsed = started.elapsed();
            std::thread::sleep(std::time::Duration::from_millis(300));
            let probe_pixels = outcome.as_ref().map_or(0, |image| {
                image
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[0] > 240 && p[1] < 20 && p[2] > 240)
                    .count()
            });
            report.push_str(&format!(
                "| `{}` | {} | {} ms | {} | {} | {} |\n",
                fixture.name(),
                if remote == RemoteImages::Allowed {
                    "allowed"
                } else {
                    "blocked"
                },
                elapsed.as_millis(),
                outcome.is_err(),
                listener.count() - before,
                probe_pixels
            ));
        }
    }
    let _ = std::fs::remove_file(probe);
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

/// One fixture, composed exactly as the reader composes a single message.
struct Message {
    body: MessageBody,
    parts: HashMap<String, Vec<u8>>,
    styles: String,
    html: String,
}

impl Message {
    fn of(fixture: &Fixture) -> Message {
        Message::with(fixture, |html| html.to_owned(), RemoteImages::Blocked)
    }

    /// As [`of`](Self::of), with the HTML rewritten first and composed
    /// under `remote`.
    fn with(fixture: &Fixture, rewrite: impl Fn(&str) -> String, remote: RemoteImages) -> Message {
        let mut parsed = postio_model::mime::parse(fixture.bytes());
        parsed.body.html = parsed.body.html.map(|html| rewrite(&html));
        let parts = parsed
            .parts
            .iter()
            .filter_map(|part| Some((part.attachment.content_id.clone()?, part.content.clone())))
            .collect();
        let body = parsed.body;
        let rendered = document::body_html_in(&body, remote, document::opening_rendering(), None);
        let sheet = document::sheet_for(Rendering::Original, document::suits_reader_view(&body));
        let html = stamp(&document::document_for(
            &rendered.html,
            &rendered.styles,
            remote,
            sheet,
        ));
        Message {
            body,
            parts,
            styles: rendered.styles,
            html,
        }
    }

    fn lay_out(&self, fonts: &FontSet, dark: bool, extra_css: Option<&str>) -> BaseDocument {
        let html = match extra_css {
            Some(css) => self
                .html
                .replacen("</head>", &format!("<style>{css}</style></head>"), 1),
            None => self.html.clone(),
        };
        let config = DocumentConfig {
            font_ctx: Some(fonts.context()),
            viewport: Some(Viewport::new(
                (WIDTH as f32 * scale()) as u32,
                (900.0 * scale()) as u32,
                scale(),
                if dark {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                },
            )),
            ua_stylesheets: Some(vec![HEAD_IS_NOT_CONTENT.to_owned()]),
            // A base a relative URL can resolve against. Without one, Blitz
            // resolves against a `data:` URL, which cannot be a base, and
            // panics on the first `<img src="x">` (the evaluation's G3). Under
            // `POSTIO_EVAL_NO_BASE` the harness leaves it unset, to show that.
            base_url: std::env::var_os("POSTIO_EVAL_NO_BASE")
                .is_none()
                .then(|| "postio-message://message/".to_owned()),
            net_provider: Some(Arc::new(Resources {
                parts: self.parts.clone(),
            })),
            ..Default::default()
        };
        let mut doc = blitz_html::HtmlDocument::from_html(&html, config).into_inner();
        doc.resolve(0.0);
        let _ = &self.body;
        doc
    }
}

/// The document's only source of sub-resources: its own parts and the
/// bundled faces. Anything else is never requested.
struct Resources {
    parts: HashMap<String, Vec<u8>>,
}

impl NetProvider for Resources {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        if std::env::var_os("POSTIO_EVAL_DEBUG").is_some() {
            eprintln!("fetch: {}", &url[..url.len().min(60)]);
        }
        if let Some(rest) = url.strip_prefix("postio-cid:") {
            let id = postio_body::sanitize::percent_decode(rest.rsplit('/').next().unwrap_or(rest));
            if let Some(bytes) = self.parts.get(&id) {
                handler.bytes(url, Bytes::from(bytes.clone()));
            }
            return;
        }
        // An image the message embeds in itself (FR-003): decoded here, never
        // fetched. Anything else in a `data:` URL was refused upstream.
        if let Some(rest) = url.strip_prefix("data:image/")
            && let Some((_, payload)) = rest.split_once(";base64,")
            && let Some(bytes) = decode_base64(payload)
        {
            handler.bytes(url, Bytes::from(bytes));
            return;
        }
        if let Some(face) = url.strip_prefix(document::FONT_SCHEME)
            && let Some(bytes) = document::font_bytes(face.trim_start_matches([':', '/']))
        {
            handler.bytes(url, Bytes::from_static(bytes));
        }
    }
}

/// Standard base64, tolerating whitespace; `None` on anything else.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let clean: Vec<u8> = text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let v: Vec<u8> = chunk.iter().map(|&c| value(c)).collect::<Option<_>>()?;
        let n = v.iter().fold(0u32, |acc, &d| acc << 6 | u32::from(d)) << (6 * (4 - v.len()));
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..v.len().saturating_sub(1)]);
    }
    Some(out)
}

/// Fonts without fontconfig (research R3): the bundled faces first, as the
/// generic families -- as in the reference capture -- then every installed
/// face `fontdb` finds, registered from its bytes.
struct FontSet {
    collection: Collection,
}

impl FontSet {
    fn context(&self) -> FontContext {
        FontContext {
            collection: self.collection.clone(),
            source_cache: Default::default(),
        }
    }
}

/// Proportional set size of this process, in MiB: what #1348 measured web
/// processes by, for the process Blitz draws in.
fn pss_mib() -> f64 {
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("Pss:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

/// S2: the thread `pane_comparison` measures arm A on, composed through the
/// same pipeline, laid out and painted in-process at 2, 10 and 50 messages.
fn cost(fonts: &FontSet, report: &mut String) {
    use postio_ui::reader::thread::{Entry, conversation_document};
    use std::time::Instant;
    report.push_str(&format!(
        "## S2 cost\n\nProcess Pss after font discovery: {:.0} MiB.\n\n| messages | first render | warm render | height | Pss after | growth |\n|---|---|---|---|---|---|\n",
        pss_mib()
    ));
    for n in [2usize, 10, 50] {
        let bodies: Vec<(String, String, String)> = (0..n)
            .map(|index| {
                let html = format!(
                    "<p>Message {index} from Ren Ishida. The scope has three parts and I will \
                     go through each so there are no surprises on the day.</p>\
                     <p><b>Diagnostics.</b> We place two continuous monitors for 48 hours \
                     -- one in the lowest livable level, one a floor above -- and pull a \
                     soil-gas reading at the slab.</p>\
                     <blockquote><p>Quoted from the message before it, so the folding \
                     path has something to fold.</p></blockquote>"
                );
                let body = MessageBody {
                    text: None,
                    html: Some(html),
                };
                let scope = (index + 1).to_string();
                let rendered = document::body_html_in(
                    &body,
                    RemoteImages::Blocked,
                    document::opening_rendering(),
                    Some(&scope),
                );
                (scope, rendered.html, rendered.styles)
            })
            .collect();
        let entries: Vec<Entry<'_>> = bodies
            .iter()
            .enumerate()
            .map(|(index, (scope, body, styles))| Entry {
                scope,
                sender: "Ren Ishida",
                address: "ren.ishida@example.net",
                when: "24 Aug",
                preview: "preview",
                expanded: true,
                draft: false,
                mine: false,
                latest: index + 1 == n,
                blocked: 0,
                body,
                recipients: "",
                cc: "",
                sheet: postio_ui::reader::document::Sheet::Theme,
                styles,
            })
            .collect();
        let html = conversation_document(&entries, RemoteImages::Blocked, document::Sheet::Theme);
        let message = Message {
            body: MessageBody::default(),
            parts: HashMap::new(),
            styles: String::new(),
            html,
        };
        let before = pss_mib();
        let started = Instant::now();
        let mut doc = message.lay_out(fonts, false, None);
        let image = paint(&mut doc);
        let first = started.elapsed();
        let started = Instant::now();
        let mut again = message.lay_out(fonts, false, None);
        let _ = paint(&mut again);
        let warm = started.elapsed();
        let after = pss_mib();
        report.push_str(&format!(
            "| {n} | {:.1} ms | {:.1} ms | {} px | {after:.0} MiB | +{:.0} MiB |\n",
            first.as_secs_f64() * 1000.0,
            warm.as_secs_f64() * 1000.0,
            image.height,
            after - before
        ));
        drop((doc, again, image));
    }
    report.push_str(
        "\nOne process, no web process and no network process. The whole document is \
         rasterised into one buffer here (prototype depth); the plan's tiles cap that at \
         64 MiB whatever the height.\n",
    );
}

fn font_set() -> FontSet {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let mut sans = None;
    let mut mono = None;
    for face in document::FACES {
        for (family, _) in collection.register_fonts(Blob::new(Arc::new(face.bytes) as _), None) {
            let name = collection
                .family_name(family)
                .unwrap_or_default()
                .to_owned();
            if name == "Barlow" {
                sans = Some(family);
            }
            if name == "IBM Plex Mono" {
                mono = Some(family);
            }
        }
    }
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut seen = std::collections::HashSet::new();
    let mut aliases: Vec<(PathBuf, &str)> = Vec::new();
    for face in database.faces() {
        if let fontdb::Source::File(path) = &face.source
            && seen.insert(path.clone())
            && let Ok(bytes) = std::fs::read(path)
        {
            collection.register_fonts(Blob::new(Arc::new(bytes) as _), None);
            let family = face
                .families
                .first()
                .map(|(name, _)| name.as_str())
                .unwrap_or("");
            // What fontconfig would have substituted: the two names mail asks
            // for most, which no free system ships under that name.
            if family == "Liberation Sans" {
                aliases.push((path.clone(), "Helvetica"));
                aliases.push((path.clone(), "Arial"));
            }
        }
    }
    for (path, alias) in aliases {
        if let Ok(bytes) = std::fs::read(&path) {
            collection.register_fonts(
                Blob::new(Arc::new(bytes) as _),
                Some(FontInfoOverride {
                    family_name: Some(alias),
                    ..Default::default()
                }),
            );
        }
    }
    if let Some(sans) = sans {
        collection.set_generic_families(GenericFamily::SansSerif, std::iter::once(sans));
        collection.set_generic_families(GenericFamily::Serif, std::iter::once(sans));
        collection.set_generic_families(GenericFamily::SystemUi, std::iter::once(sans));
    }
    if let Some(mono) = mono {
        collection.set_generic_families(GenericFamily::Monospace, std::iter::once(mono));
    }
    FontSet { collection }
}

/// The device scale renders are made at: 1 unless `POSTIO_EVAL_SCALE` says
/// otherwise (the sharpness probe sets it per render).
fn scale() -> f32 {
    SCALE.with(|s| s.get())
}

thread_local! {
    static SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

fn paint(doc: &mut BaseDocument) -> Image {
    let s = scale();
    let height = (doc.root_element().final_layout().size.height.ceil() as u32).clamp(1, 30_000);
    let (w, h) = ((WIDTH as f32 * s) as u32, (height as f32 * s) as u32);
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(w, h);
    let mut buffer = vec![0u8; (w * h * 4) as usize];
    renderer.render(
        |scene| blitz_paint::paint_scene(scene, doc, f64::from(s), w, h, 0, 0),
        &mut buffer,
    );
    Image::from_rgba(w as usize, h as usize, buffer)
}

/// Bilinear 2x enlargement: what a compositor does to a 1x texture shown on
/// a 2x display.
fn enlarge(image: &Image) -> Image {
    let (w, h) = (image.width * 2, image.height * 2);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = ((x as f64 + 0.5) / 2.0 - 0.5, (y as f64 + 0.5) / 2.0 - 0.5);
            let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
            let (x1, y1) = (
                (x0 + 1).min(image.width - 1),
                (y0 + 1).min(image.height - 1),
            );
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            for c in 0..4 {
                let at =
                    |xx: usize, yy: usize| f64::from(image.rgba[(yy * image.width + xx) * 4 + c]);
                let v = at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + at(x1, y0) * tx * (1.0 - ty)
                    + at(x0, y1) * (1.0 - tx) * ty
                    + at(x1, y1) * tx * ty;
                rgba[(y * w + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image::from_rgba(w, h, rgba)
}

fn crop(image: &Image, x: usize, y: usize, w: usize, h: usize) -> Image {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in y..(y + h).min(image.height) {
        let start = (row * image.width + x) * 4;
        rgba.extend_from_slice(&image.rgba[start..start + w.min(image.width - x) * 4]);
    }
    let rows = rgba.len() / (w.min(image.width - x) * 4);
    Image::from_rgba(w.min(image.width - x), rows, rgba)
}

/// One text run: its element, its rectangle in CSS px, and its colour.
struct Run {
    node: NodeId,
    rect: [f64; 4],
    colour: Rgb,
    ground: Option<Rgb>,
}

fn runs_of(doc: &BaseDocument) -> Vec<Run> {
    let mut out = Vec::new();
    walk(doc, doc.root_node().id, &mut out);
    out
}

fn walk(doc: &BaseDocument, id: NodeId, out: &mut Vec<Run>) {
    let Some(node) = doc.get_node(id) else { return };
    if node.flags.is_inline_root()
        && let Some(element) = node.element_data()
        && let Some(text) = element.inline_layout_data.as_ref()
    {
        let origin = node.absolute_position(0.0, 0.0);
        let layout = node.final_layout();
        let (left, top) = (
            f64::from(origin.x + layout.padding.left + layout.border.left),
            f64::from(origin.y + layout.padding.top + layout.border.top),
        );
        for line in text.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let span = run.style().brush.id;
                let Some(colour) = colour_of(doc, span) else {
                    continue;
                };
                if run.advance() <= 0.0 {
                    continue;
                }
                let metrics = run.run().metrics();
                out.push(Run {
                    node: span,
                    rect: [
                        left + f64::from(run.offset()),
                        top + f64::from(run.baseline() - metrics.ascent),
                        f64::from(run.advance()),
                        f64::from(metrics.ascent + metrics.descent),
                    ],
                    colour,
                    ground: ground_of(doc, span),
                });
            }
        }
    }
    for child in node.children.iter().copied() {
        walk(doc, child, out);
    }
}

fn srgb(colour: &style::color::AbsoluteColor) -> (Rgb, f32) {
    let [r, g, b, a] = *colour
        .to_color_space(style::color::ColorSpace::Srgb)
        .raw_components();
    (
        Rgb {
            r: f64::from(r),
            g: f64::from(g),
            b: f64::from(b),
        },
        a,
    )
}

fn colour_of(doc: &BaseDocument, id: NodeId) -> Option<Rgb> {
    let styles = doc.get_node(id)?.primary_styles()?;
    Some(srgb(&styles.clone_color()).0)
}

/// The first opaque background among the element and its ancestors.
fn ground_of(doc: &BaseDocument, id: NodeId) -> Option<Rgb> {
    let mut next = Some(id);
    while let Some(id) = next {
        let node = doc.get_node(id)?;
        if let Some(styles) = node.primary_styles() {
            let current = styles.clone_color();
            let background = styles
                .get_background()
                .background_color
                .resolve_to_absolute(&current);
            let (rgb, alpha) = srgb(&background);
            if alpha > 0.5 {
                return Some(rgb);
            }
        }
        next = node.parent;
    }
    None
}

fn count_below(image: &Image, runs: &[Run], floor: f64) -> usize {
    runs.iter()
        .filter(|run| {
            let [x, y, w, h] = run.rect.map(|v| v.max(0.0) as usize);
            fidelity::ground_behind(image, x, y, w.max(1), h.max(1))
                .map(|[r, g, b]| theme::contrast(run.colour, Rgb::from_u8(r, g, b)) < floor - 1e-6)
                .unwrap_or(false)
        })
        .count()
}

fn first(doc: &BaseDocument, selector: &str) -> Option<NodeId> {
    doc.query_selector(selector).ok().flatten()
}

fn facts(doc: &BaseDocument, message: &Message) -> MessageFacts {
    let Some(body) = first(doc, ".postio-body") else {
        return MessageFacts::default();
    };
    let canvas = first(doc, ".postio-canvas");
    let background = |id: NodeId| {
        let styles = doc.get_node(id)?.primary_styles()?;
        let (rgb, alpha) = srgb(
            &styles
                .get_background()
                .background_color
                .resolve_to_absolute(&styles.clone_color()),
        );
        (alpha > 0.5).then_some(rgb)
    };
    let mut inner = false;
    let mut stack: Vec<NodeId> = doc
        .get_node(body)
        .map(|n| n.children.iter().copied().collect())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if Some(id) != canvas && node.is_element() {
            let image = node.primary_styles().is_some_and(|s| {
                s.get_background()
                    .background_image
                    .0
                    .iter()
                    .any(|i| !matches!(i, style::values::computed::image::Image::None))
            });
            if background(id).is_some() || image {
                inner = true;
                break;
            }
        }
        stack.extend(node.children.iter().copied());
    }
    let declares = message
        .html
        .contains("data-postio-color-scheme=\"light dark\"")
        || message.html.contains("data-postio-color-scheme=\"dark\"")
        || (message.styles.contains("prefers-color-scheme") && message.styles.contains("dark"));
    MessageFacts {
        canvas: canvas.and_then(background),
        inner_background: inner,
        declares_dark: declares,
    }
}

/// The attribute every element of a composed document is stamped with.
const STAMP: &str = "data-postio-eval";

/// `html` with every element stamped `data-postio-eval="<n>"`, in document
/// order, so overrides can be written as a stylesheet.
///
/// Why a stylesheet on a fresh layout rather than a mutation: in
/// blitz-dom 0.3.0-beta.2 a `style` attribute set through `DocumentMutator`
/// on a `<p>` reached neither computed style nor paint, through two resolves,
/// while the same call on another element did. A finding for the scorecard,
/// and routed around here rather than debugged inside a beta engine.
fn stamp(html: &str) -> String {
    use html5ever::driver::ParseOpts;
    use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
    use html5ever::tendril::TendrilSink;
    use html5ever::{Attribute, QualName as HtmlName, ns};
    use markup5ever_rcdom::{NodeData, RcDom, SerializableHandle};

    let dom = html5ever::parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut next = 0usize;
    let mut stack = vec![dom.document.clone()];
    while let Some(node) = stack.pop() {
        if let NodeData::Element { attrs, .. } = &node.data {
            attrs.borrow_mut().push(Attribute {
                name: HtmlName::new(None, ns!(), html5ever::LocalName::from(STAMP)),
                value: next.to_string().into(),
            });
            next += 1;
        }
        let children = node.children.borrow();
        for child in children.iter().rev() {
            stack.push(child.clone());
        }
    }
    let mut bytes = Vec::new();
    let handle: SerializableHandle = dom.document.clone().into();
    let _ = serialize(
        &mut bytes,
        &handle,
        SerializeOpts {
            traversal_scope: TraversalScope::ChildrenOnly(None),
            ..SerializeOpts::default()
        },
    );
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The overrides research R10's rule asks for, as a stylesheet.
#[derive(Default)]
struct Overrides {
    rules: Vec<String>,
    colours: HashMap<String, [u8; 3]>,
}

impl Overrides {
    fn present(&mut self, presentation: Presentation, facts: MessageFacts) {
        match presentation {
            Presentation::Paper => {
                let [r, g, b] = facts.canvas.unwrap_or(Rgb::from_u8(255, 255, 255)).to_u8();
                self.rules.push(format!(
                    ".postio-body {{ background: rgb({r},{g},{b}) !important }}"
                ));
            }
            Presentation::Adapted => {
                self.rules
                    .push(".postio-canvas { background: transparent !important }".to_owned());
            }
            _ => {}
        }
    }

    /// Repair every run whose colour misses `floor` against its ground,
    /// keyed by the stamp of the element holding its text.
    fn repair(&mut self, doc: &BaseDocument, runs: &[Run], floor: f64) {
        let fallback = first(doc, "body")
            .and_then(|b| ground_of(doc, b))
            .unwrap_or(Rgb::from_u8(255, 255, 255));
        for run in runs {
            let fixed = theme::repair(run.colour, run.ground.unwrap_or(fallback), floor);
            if fixed.to_u8() == run.colour.to_u8() {
                continue;
            }
            let element = match doc.get_node(run.node) {
                Some(n) if n.is_element() => n,
                Some(n) => match n.parent.and_then(|p| doc.get_node(p)) {
                    Some(parent) => parent,
                    None => continue,
                },
                None => continue,
            };
            if let Some(stamp) = element.attr(LocalName::from(STAMP)) {
                self.colours.insert(stamp.to_owned(), fixed.to_u8());
            }
        }
    }

    fn css(&self) -> String {
        let mut css = self.rules.join("\n");
        for (stamp, [r, g, b]) in &self.colours {
            // `:is(#…, [stamp])` takes the id's specificity, three times over:
            // enough to outrank a sender's own `!important` rule, as an inline
            // `!important` does in arm A. A rule that only wins on order loses
            // to `.fine { color: … !important }`.
            let one = format!(":is(#postio-eval-override, [{STAMP}=\"{stamp}\"])");
            css.push_str(&format!(
                "\n{one}{one}{one} {{ color: rgb({r},{g},{b}) !important }}"
            ));
        }
        css
    }
}
