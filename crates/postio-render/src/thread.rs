//! The render thread (research R6). Each reader owns one; it owns every
//! Blitz document, and only the immutable snapshot leaves it.
//!
//! - A request carries a generation; a result for a generation that has
//!   been superseded or abandoned is never delivered -- its receiver just
//!   disconnects.
//! - A render runs inside `catch_unwind`. After a panic the document is
//!   dropped, never reused, and the plain-text fallback is delivered.
//! - There is no way to interrupt a layout. `abandon` detaches the
//!   thread running it: the next request goes to a fresh thread, and the
//!   old one finishes, delivers nothing, and exits. A newer request does
//!   the same to a render it supersedes, and a superseded render stops at
//!   the next pass it would have started.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use crate::fonts::FontSet;
use crate::{FallbackReason, RenderRequest, RenderedDocument};

/// The production render bound (spec FR-023). Callers take their deadline
/// as a parameter and default to this; tests inject a scaled or tiny one.
pub const DEFAULT_RENDER_DEADLINE: Duration = Duration::from_millis(400);

/// The render thread's stack. Deep markup is flattened by the input caps;
/// this is the margin under them, because an overflow cannot be caught.
const STACK: usize = 64 * 1024 * 1024;

/// What names a render thread, so the panic hook knows its own.
const THREAD_NAME: &str = "postio-render";

struct Job {
    request: RenderRequest,
    reply: Sender<RenderedDocument>,
}

/// One render thread, and what it is working on.
struct Worker {
    jobs: Sender<Job>,
    /// The generation inside a render right now; 0 when idle.
    running: Arc<AtomicU64>,
}

/// Generations as the reader sees them, shared with every thread it has had.
#[derive(Default)]
struct Generations {
    /// The newest generation asked for: anything older is stale.
    latest: AtomicU64,
    /// The newest generation given up on.
    abandoned: AtomicU64,
}

impl Generations {
    fn wanted(&self, generation: u64) -> bool {
        generation >= self.latest.load(Ordering::Acquire)
            && generation > self.abandoned.load(Ordering::Acquire)
    }
}

/// One per reader. Owns a render thread (64 MiB stack) and every Blitz
/// document on it. Dropping it detaches the thread, which exits when its
/// queue is empty.
pub struct Renderer {
    fonts: FontSet,
    generations: Arc<Generations>,
    worker: Mutex<Worker>,
}

impl Renderer {
    /// A renderer drawing with `fonts`.
    pub fn new(fonts: &FontSet) -> Renderer {
        install_panic_hook();
        let generations = Arc::new(Generations::default());
        Renderer {
            worker: Mutex::new(spawn(fonts.clone(), generations.clone())),
            fonts: fonts.clone(),
            generations,
        }
    }

    /// Ask for a render. Non-blocking: the result arrives on the receiver,
    /// or the receiver disconnects if the request is superseded or
    /// abandoned first. The caller runs the deadline.
    pub fn request(&self, request: RenderRequest) -> Receiver<RenderedDocument> {
        self.generations
            .latest
            .fetch_max(request.generation, Ordering::AcqRel);
        let generation = request.generation;
        let (reply, result) = mpsc::channel();
        let job = Job { request, reply };
        let mut worker = self.worker.lock().expect("the worker is never poisoned");
        // A render still running for an older generation is one nobody will
        // see: queueing behind it would spend this request's deadline on it
        // (T218). The thread is left to notice it is stale and stop, and a
        // fresh one takes this request at once.
        let running = worker.running.load(Ordering::Acquire);
        if running != 0 && running < generation {
            *worker = spawn(self.fonts.clone(), self.generations.clone());
        }
        if let Err(mpsc::SendError(job)) = worker.jobs.send(job) {
            // The thread is gone (it cannot be: panics are caught). A new
            // one takes the job rather than losing it.
            *worker = spawn(self.fonts.clone(), self.generations.clone());
            let _ = worker.jobs.send(job);
        }
        result
    }

    /// Give up on `generation`: its result is never delivered, and if it is
    /// still running, the thread running it is left to finish on its own
    /// while the next request goes to a fresh one.
    pub fn abandon(&self, generation: u64) {
        self.generations
            .abandoned
            .fetch_max(generation, Ordering::AcqRel);
        let mut worker = self.worker.lock().expect("the worker is never poisoned");
        if worker.running.load(Ordering::Acquire) == generation {
            *worker = spawn(self.fonts.clone(), self.generations.clone());
        }
    }

    /// The plain-text fallback for `request`, drawn synchronously: its
    /// composed [`Fallback`](crate::Fallback) when it carries one, else
    /// its plain text by the renderer's own minimal path. Drawn as
    /// `request`'s generation.
    pub fn fallback(&self, request: &RenderRequest, reason: FallbackReason) -> RenderedDocument {
        fallback(&self.fonts, request, reason)
    }
}

fn spawn(fonts: FontSet, generations: Arc<Generations>) -> Worker {
    let (jobs, queue) = mpsc::channel::<Job>();
    let running = Arc::new(AtomicU64::new(0));
    let busy = running.clone();
    std::thread::Builder::new()
        .name(THREAD_NAME.to_owned())
        .stack_size(STACK)
        .spawn(move || {
            for Job { request, reply } in queue {
                let generation = request.generation;
                if !generations.wanted(generation) {
                    continue;
                }
                if let Some(cap) = request.over_cap {
                    let document = fallback(&fonts, &request, FallbackReason::OverCap(cap));
                    if generations.wanted(generation) {
                        let _ = reply.send(document);
                    }
                    continue;
                }
                busy.store(generation, Ordering::Release);
                let stale = || !generations.wanted(generation);
                let rendered = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    crate::render::render_unless(&request, &fonts, &stale)
                }));
                busy.store(0, Ordering::Release);
                // After a panic the document went with the unwind; it is
                // never reused.
                let document = match rendered {
                    Ok(Some(document)) => document,
                    // Superseded part way: nothing to deliver.
                    Ok(None) => continue,
                    Err(_) => fallback(&fonts, &request, FallbackReason::Panicked),
                };
                if generations.wanted(generation) {
                    let _ = reply.send(document);
                }
            }
        })
        .expect("a render thread can be spawned");
    Worker { jobs, running }
}

/// Draw `request`'s plain text in place of its document: the reader's own
/// composition of it when the request carries one, so the fallback takes
/// the column, face and rhythm of any plain-text body (T218); the
/// renderer's minimal page otherwise.
fn fallback(fonts: &FontSet, request: &RenderRequest, reason: FallbackReason) -> RenderedDocument {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let notice = format!(
        "<p class=\"{NOTICE_CLASS}\">{}</p>",
        escape(&reason.notice())
    );
    let document = match &request.fallback {
        Some(composed) if composed.notice => with_notice(&composed.document, &notice),
        Some(composed) => composed.document.clone(),
        None => {
            let (ground, ink) = if request.theme.dark {
                ("#1e1e1e", "#e8e8e8")
            } else {
                ("#ffffff", "#1a1a1a")
            };
            format!(
                "<!DOCTYPE html><html><body style=\"margin:0;padding:16px;\
                 background:{ground};color:{ink};font-family:sans-serif\">\
                 <p style=\"margin:0 0 12px;font-size:12.5px;opacity:0.75\">{}</p>\
                 <pre style=\"white-space:pre-wrap;font-family:sans-serif;margin:0\">{}</pre>\
                 </body></html>",
                escape(&reason.notice()),
                escape(&request.plain_text)
            )
        }
    };
    let request = RenderRequest {
        document,
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        // Nothing the message names is drawn, so nothing is looked up: the
        // faces are the font set's own. And never the message's table,
        // which is what failed or is still in use by the render given up on.
        resources: Arc::new(crate::Resources::new()),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
        ..request.clone()
    };
    let mut document = crate::render(&request, fonts);
    document.outcome = crate::Outcome::FellBack(reason);
    document
}

/// The class of the quiet line a fallback document says why in: the
/// reader's stylesheet draws it as its other notices are drawn.
pub const NOTICE_CLASS: &str = "postio-fallback-notice";

/// `document` with `notice` as the first thing in its `<body>`, above the
/// body's own container. The `<body>` after the head: the head's sheets
/// and comments can spell the word too.
fn with_notice(document: &str, notice: &str) -> String {
    let head = document.find("</head>").unwrap_or(0);
    let at = document[head..]
        .find("<body")
        .map(|start| head + start)
        .and_then(|start| document[start..].find('>').map(|end| start + end + 1));
    match at {
        Some(at) => format!("{}{notice}{}", &document[..at], &document[at..]),
        None => format!("{notice}{document}"),
    }
}

/// Log a render thread's panic by where it happened, never by what it
/// said: a payload can quote the message (logs carry no content). Every
/// other thread's panic goes to the hook that was there before.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().name() == Some(THREAD_NAME) {
                tracing::error!(
                    location = %info.location().map_or_else(String::new, ToString::to_string),
                    "render panicked; showing the plain-text fallback"
                );
            } else {
                previous(info);
            }
        }));
    });
}
