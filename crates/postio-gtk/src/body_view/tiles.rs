//! Tiles of the snapshot on screen (research R8): 512 logical pixels tall,
//! rasterised off the UI thread and kept as textures.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_render::RenderedDocument;
use postio_render::tile::{TileSpec, rasterize_tile, tile_width};

/// A tile's height in logical pixels.
pub(super) const TILE: f64 = 512.0;

/// One tile to rasterise, and where to send it.
struct Job {
    document: Arc<RenderedDocument>,
    index: u32,
    reply: Sender<(u64, u32, gdk::MemoryTexture)>,
}

/// The rasterising pool: two threads for the whole process. A tile is a
/// few milliseconds of CPU; two keep one scroll ahead without contending
/// with the render thread.
fn pool() -> &'static Mutex<Sender<Job>> {
    static POOL: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (jobs, queue) = mpsc::channel::<Job>();
        let queue = Arc::new(Mutex::new(queue));
        for n in 0..2 {
            let queue = queue.clone();
            std::thread::Builder::new()
                .name(format!("postio-tiles-{n}"))
                .spawn(move || {
                    loop {
                        let job = match queue.lock() {
                            Ok(queue) => queue.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        let texture = raster(&job.document, job.index);
                        let _ = job
                            .reply
                            .send((job.document.generation, job.index, texture));
                    }
                })
                .expect("a tile thread can be spawned");
        }
        Mutex::new(jobs)
    })
}

/// The device-pixel band tile `index` covers.
fn spec(document: &RenderedDocument, index: u32) -> TileSpec {
    let height = (TILE * document.scale).ceil() as u32;
    TileSpec::nth(index, height)
}

fn raster(document: &RenderedDocument, index: u32) -> gdk::MemoryTexture {
    let spec = spec(document, index);
    let width = tile_width(document);
    let mut pixels = vec![0u8; width as usize * spec.height as usize * 4];
    rasterize_tile(document, spec, &mut pixels);
    gdk::MemoryTexture::new(
        width as i32,
        spec.height as i32,
        gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &glib::Bytes::from_owned(pixels),
        width as usize * 4,
    )
}

/// The textures of one snapshot, and the ones on their way.
#[derive(Default)]
pub(super) struct Tiles {
    generation: u64,
    textures: HashMap<u32, gdk::MemoryTexture>,
    pending: HashMap<u32, ()>,
    results: Option<Receiver<(u64, u32, gdk::MemoryTexture)>>,
    reply: Option<Sender<(u64, u32, gdk::MemoryTexture)>>,
}

impl Tiles {
    /// Start over for a new snapshot.
    pub(super) fn reset(&mut self, document: Arc<RenderedDocument>) {
        let (reply, results) = mpsc::channel();
        *self = Tiles {
            generation: document.generation,
            textures: HashMap::new(),
            pending: HashMap::new(),
            results: Some(results),
            reply: Some(reply),
        };
    }

    /// Draw the tiles in view; ask for the missing ones, and call `redraw`
    /// when they arrive.
    pub(super) fn draw(
        &mut self,
        snapshot: &gtk::Snapshot,
        document: &Arc<RenderedDocument>,
        left: f64,
        top: f64,
        height: f64,
        redraw: impl Fn() + 'static,
    ) {
        self.collect();
        let first = (top / TILE).floor().max(0.0) as u32;
        let last = ((top + height) / TILE).floor().max(0.0) as u32;
        let last = last.min((document.size.height / TILE).floor() as u32);
        let mut missing = false;
        for index in first..=last {
            let y = f64::from(index) * TILE - top;
            match self.textures.get(&index) {
                Some(texture) => {
                    let bounds = gtk::graphene::Rect::new(
                        -left as f32,
                        y as f32,
                        document.size.width as f32,
                        (f64::from(texture.height()) / document.scale) as f32,
                    );
                    snapshot.append_texture(texture, &bounds);
                }
                None => {
                    missing = true;
                    self.ask(document, index);
                }
            }
        }
        if missing {
            self.wake(redraw);
        }
    }

    fn ask(&mut self, document: &Arc<RenderedDocument>, index: u32) {
        if self.pending.contains_key(&index) {
            return;
        }
        let Some(reply) = self.reply.clone() else {
            return;
        };
        self.pending.insert(index, ());
        let job = Job {
            document: document.clone(),
            index,
            reply,
        };
        if let Ok(pool) = pool().lock() {
            let _ = pool.send(job);
        }
    }

    /// Take the tiles that have arrived.
    fn collect(&mut self) {
        let Some(results) = &self.results else {
            return;
        };
        while let Ok((generation, index, texture)) = results.try_recv() {
            if generation == self.generation {
                self.pending.remove(&index);
                self.textures.insert(index, texture);
            }
        }
    }

    /// Redraw once the pool has sent something.
    fn wake(&self, redraw: impl Fn() + 'static) {
        glib::timeout_add_local_once(Duration::from_millis(8), redraw);
    }
}
