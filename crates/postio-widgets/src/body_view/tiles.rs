//! Tiles of the snapshot: 512 logical pixels tall, rasterised off the UI thread and kept as
//! textures.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_render::RenderedDocument;
use postio_render::tile::{TileSpec, rasterize_tile, tile_width};

pub(super) const TILE: f64 = 512.0;

/// Texture byte cap per view; the low-resolution copy comes on top.
pub(super) const BUDGET: usize = 64 * 1024 * 1024;

struct Job {
    document: Arc<RenderedDocument>,
    index: u32,
    reply: Sender<(u64, u32, gdk::MemoryTexture)>,
}

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

#[derive(Default)]
pub(super) struct Tiles {
    generation: u64,
    textures: HashMap<u32, gdk::MemoryTexture>,
    pending: HashMap<u32, ()>,
    results: Option<Receiver<(u64, u32, gdk::MemoryTexture)>>,
    reply: Option<Sender<(u64, u32, gdk::MemoryTexture)>>,
    low_res: Option<gdk::MemoryTexture>,
    short: bool,
    drawn_at: Option<f64>,
    used: HashMap<u32, u64>,
    draws: u64,
    visible: (u32, u32),
}

#[derive(Clone, Copy)]
pub(super) struct Frame {
    pub left: f64,
    pub top: f64,
    pub height: f64,
    pub origin: f64,
    pub scale: f64,
}

/// Where tiles `first..=last` go, as `(index, y, height)` in logical pixels. Every edge is on a
/// device pixel and a tile ends where the next begins; fractional edges let the ground show as a
/// line between tiles.
pub(super) fn placements(
    first: u32,
    last: u32,
    top: f64,
    origin: f64,
    scale: f64,
) -> Vec<(u32, f64, f64)> {
    let edge = |index: u32| {
        let logical = f64::from(index) * TILE - top;
        ((origin + logical) * scale).round() / scale - origin
    };
    (first..=last)
        .map(|index| (index, edge(index), edge(index + 1) - edge(index)))
        .collect()
}

impl Tiles {
    pub(super) fn reset(&mut self, document: Arc<RenderedDocument>) {
        let (reply, results) = mpsc::channel();
        let low = &document.low_res;
        let low_res = (low.width > 0 && low.height > 0).then(|| {
            gdk::MemoryTexture::new(
                low.width as i32,
                low.height as i32,
                gdk::MemoryFormat::R8g8b8a8Premultiplied,
                &glib::Bytes::from(&low.rgba[..]),
                low.width as usize * 4,
            )
        });
        *self = Tiles {
            generation: document.generation,
            textures: HashMap::new(),
            pending: HashMap::new(),
            results: Some(results),
            reply: Some(reply),
            low_res,
            short: false,
            drawn_at: None,
            used: HashMap::new(),
            draws: 0,
            visible: (0, 0),
        };
    }

    pub(super) fn draw(
        &mut self,
        snapshot: &gtk::Snapshot,
        document: &Arc<RenderedDocument>,
        frame: Frame,
        redraw: impl Fn() + 'static,
    ) {
        let Frame {
            left,
            top,
            height,
            origin,
            scale,
        } = frame;
        self.draws += 1;
        let first = (top / TILE).floor().max(0.0) as u32;
        let last = ((top + height) / TILE).floor().max(0.0) as u32;
        let end = (document.size.height / TILE).floor() as u32;
        let last = last.min(end);
        self.visible = (first, last);
        self.collect();
        let mut missing = false;
        let places = placements(first, last, top, origin, scale);
        for (index, y, height) in places {
            self.used.insert(index, self.draws);
            match self.textures.get(&index) {
                Some(texture) => {
                    let bounds = gtk::graphene::Rect::new(
                        -left as f32,
                        y as f32,
                        document.size.width as f32,
                        height as f32,
                    );
                    snapshot.append_texture(texture, &bounds);
                }
                None => {
                    missing = true;
                    self.ask(document, index);
                    if let Some(low_res) = &self.low_res {
                        // Bands share device-pixel edges like the tiles, so none leaves a row
                        // between them.
                        let band = gtk::graphene::Rect::new(
                            -left as f32,
                            y as f32,
                            document.size.width as f32,
                            height as f32,
                        );
                        snapshot.push_clip(&band);
                        snapshot.append_texture(
                            low_res,
                            &gtk::graphene::Rect::new(
                                -left as f32,
                                -top as f32,
                                document.size.width as f32,
                                document.size.height as f32,
                            ),
                        );
                        snapshot.pop();
                    }
                }
            }
        }
        for index in [first.checked_sub(1), (last < end).then_some(last + 1)]
            .into_iter()
            .flatten()
        {
            if !self.textures.contains_key(&index) {
                self.used.insert(index, self.draws);
                self.ask(document, index);
            }
        }
        self.short = missing;
        self.drawn_at = Some(top);
        // Results are taken on a draw, so draw again while any are pending.
        if missing || !self.pending.is_empty() {
            self.wake(redraw);
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.textures
            .values()
            .map(|t| t.width() as usize * t.height() as usize * 4)
            .sum()
    }

    pub(super) fn settled(&self, top: f64) -> bool {
        self.pending.is_empty() && !self.short && self.drawn_at == Some(top)
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
        self.evict();
    }

    fn evict(&mut self) {
        while self.bytes() > BUDGET {
            let oldest = self
                .textures
                .keys()
                .copied()
                .filter(|index| !(self.visible.0..=self.visible.1).contains(index))
                .min_by_key(|index| self.used.get(index).copied().unwrap_or(0));
            let Some(oldest) = oldest else { return };
            self.textures.remove(&oldest);
        }
    }

    fn wake(&self, redraw: impl Fn() + 'static) {
        glib::timeout_add_local_once(Duration::from_millis(8), redraw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_tiles_meet_on_device_pixels() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5] {
            for origin in [0.0, 37.0, 50.5, 123.25] {
                for top in [0.0, 1.0, 333.0, 511.5, 700.37, 1023.6, 1500.75] {
                    let first = (top / TILE).floor() as u32;
                    let rects = placements(first, first + 4, top, origin, scale);
                    let device = |logical: f64| (origin + logical) * scale;
                    for (index, y, height) in &rects {
                        let (near, far) = (device(*y), device(*y + *height));
                        assert!(
                            (near - near.round()).abs() < 1e-6 && (far - far.round()).abs() < 1e-6,
                            "tile {index} at scale {scale}, origin {origin}, top {top}: \
                             edges {near} and {far} are between device pixels"
                        );
                        assert!(
                            (far - near - TILE * scale).abs() <= 1.0 + 1e-6,
                            "tile {index} is {} device px tall, not {}",
                            far - near,
                            TILE * scale
                        );
                    }
                    for pair in rects.windows(2) {
                        let (_, y, height) = pair[0];
                        assert!(
                            (y + height - pair[1].1).abs() < 1e-9,
                            "a gap or overlap after tile {} at scale {scale}, origin {origin}, \
                             top {top}: {} against {}",
                            pair[0].0,
                            y + height,
                            pair[1].1
                        );
                    }
                }
            }
        }
    }
}
