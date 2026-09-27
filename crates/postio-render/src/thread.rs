//! The render thread (research R6). Each reader owns one; it owns every
//! Blitz document, and only the immutable snapshot leaves it.
//!
//! - A request carries a generation; a result for a generation that has
//!   been superseded or abandoned is never delivered -- its receiver just
//!   disconnects.
//! - A render runs inside `catch_unwind`. After a panic the document is
//!   dropped, never reused, and the plain-text fallback is delivered.
//! - There is no way to stop a running render. `abandon` detaches the
//!   thread running it: the next request goes to a fresh thread, and the
//!   old one finishes, delivers nothing, and exits.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use crate::fonts::FontSet;
use crate::{FallbackReason, RenderRequest, RenderedDocument, Theme, Viewport};

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
        let (reply, result) = mpsc::channel();
        let job = Job { request, reply };
        let mut worker = self.worker.lock().expect("the worker is never poisoned");
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

    /// The plain-text fallback, drawn synchronously by the renderer's own
    /// minimal path: no sender markup, the reader's colours.
    pub fn fallback(
        &self,
        text: &str,
        theme: &Theme,
        viewport: Viewport,
        reason: FallbackReason,
        generation: u64,
    ) -> RenderedDocument {
        fallback(&self.fonts, text, theme, viewport, reason, generation)
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
                busy.store(generation, Ordering::Release);
                let rendered =
                    std::panic::catch_unwind(AssertUnwindSafe(|| crate::render(&request, &fonts)));
                busy.store(0, Ordering::Release);
                // After a panic the document went with the unwind; it is
                // never reused.
                let document = rendered.unwrap_or_else(|_| {
                    fallback(
                        &fonts,
                        &request.plain_text,
                        &request.theme,
                        request.viewport,
                        FallbackReason::Panicked,
                        generation,
                    )
                });
                if generations.wanted(generation) {
                    let _ = reply.send(document);
                }
            }
        })
        .expect("a render thread can be spawned");
    Worker { jobs, running }
}

/// Draw `text` as the reader draws plain mail.
fn fallback(
    fonts: &FontSet,
    text: &str,
    theme: &Theme,
    viewport: Viewport,
    reason: FallbackReason,
    generation: u64,
) -> RenderedDocument {
    let (ground, ink) = if theme.dark {
        ("#1e1e1e", "#e8e8e8")
    } else {
        ("#ffffff", "#1a1a1a")
    };
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let request = RenderRequest {
        generation,
        document: format!(
            "<!DOCTYPE html><html><body style=\"margin:0;background:{ground};color:{ink}\">\
             <pre style=\"white-space:pre-wrap;font-family:sans-serif;margin:16px\">{escaped}</pre>\
             </body></html>"
        ),
        plain_text: String::new(),
        resources: Arc::new(crate::Resources::new()),
        viewport,
        theme: *theme,
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    let mut document = crate::render(&request, fonts);
    document.outcome = crate::Outcome::FellBack(reason);
    document
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
