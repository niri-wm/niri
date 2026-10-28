use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use anyhow::{anyhow, ensure, Context};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryBuffer, MemoryRenderBuffer};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Size, Transform};
use smithay::wayland::compositor::with_states;
use xcursor::parser::{parse_xcursor, Image};
use xcursor::CursorTheme;

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

// Limits apply before decoding/allocation, including to the union animation canvas.
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CURSOR_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 2048;
const MAX_FRAMES: usize = 512;

type Key = (CursorIcon, u64);

pub struct CursorManager {
    theme: String,
    size: u8,
    generation: u64,
    current_cursor: CursorImageStatus,
    cache: HashMap<Key, Option<Rc<PreparedCursor>>>,
    fallback: Rc<PreparedCursor>,
    scaled_fallbacks: HashMap<u64, Rc<PreparedCursor>>,
    active_scales: Vec<u64>,
    scale_scratch: Vec<u64>,
    requested_icon: Option<CursorIcon>,
    worker: Option<Worker>,
}

impl CursorManager {
    pub fn new(theme: &str, size: u8) -> Self {
        Self::ensure_env(theme, size);
        Self {
            theme: theme.to_owned(),
            size,
            generation: 0,
            current_cursor: CursorImageStatus::default_named(),
            cache: HashMap::new(),
            fallback: Rc::new(builtin_cursor(1.0)),
            scaled_fallbacks: HashMap::new(),
            active_scales: Vec::new(),
            scale_scratch: Vec::new(),
            requested_icon: None,
            worker: None,
        }
    }

    /// Start the reusable loader; completions only publish already-prepared CPU buffers.
    pub fn start(&mut self, event_loop: calloop::LoopHandle<'static, crate::niri::State>) {
        if self.worker.is_some() {
            return;
        }
        let (tx, rx) = calloop::channel::channel();
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(self.generation),
            ..Shared::default()
        });
        let worker_shared = shared.clone();
        let handle = thread::Builder::new()
            .name("cursor-loader".into())
            .spawn(move || worker_loop(worker_shared, tx))
            .expect("error spawning cursor loader");
        self.worker = Some(Worker {
            shared,
            _handle: handle,
        });
        event_loop
            .insert_source(rx, |event, _, state| {
                if let calloop::channel::Event::Msg(()) = event {
                    if state.niri.cursor_manager.complete() {
                        state.niri.queue_redraw_all();
                    }
                }
            })
            .unwrap();
        self.requested_icon = None;
    }

    pub fn reload(&mut self, theme: &str, size: u8) {
        Self::ensure_env(theme, size);
        self.theme = theme.to_owned();
        self.size = size;
        self.generation = self.generation.wrapping_add(1);
        self.cache.clear();
        self.scaled_fallbacks.clear();
        self.requested_icon = None;
        if let Some(worker) = &self.worker {
            worker
                .shared
                .generation
                .store(self.generation, Ordering::Release);
            let mut mailbox = worker.shared.mailbox.lock().unwrap();
            mailbox.invalidate(self.generation);
            worker.shared.wake.notify_one();
        }
    }

    /// Schedule the active named cursor and any needed default, at exact output scales.
    /// Getters never load, rasterize, enqueue, or create renderer resources.
    pub fn prepare(&mut self, scales: impl IntoIterator<Item = f64>) {
        let mut changed = false;
        self.scale_scratch.clear();
        self.scale_scratch
            .extend(scales.into_iter().filter_map(scale_key));
        self.scale_scratch.sort_unstable();
        self.scale_scratch.dedup();
        if self.scale_scratch != self.active_scales {
            std::mem::swap(&mut self.active_scales, &mut self.scale_scratch);
            // Keep both buffers warm, including the first unchanged refresh after a swap.
            self.scale_scratch.clear();
            self.scale_scratch.reserve(self.active_scales.capacity());
            self.cache
                .retain(|(_, scale), _| self.active_scales.contains(scale));
            self.scaled_fallbacks
                .retain(|scale, _| self.active_scales.contains(scale));
            changed = true;
        }
        let icon = self.named_icon();
        if self.requested_icon != icon {
            self.requested_icon = icon;
            changed = true;
        }
        if icon.is_some() {
            for &scale in &self.active_scales {
                self.scaled_fallbacks.entry(scale).or_insert_with(|| {
                    let factor = f64::from(self.size.max(1)) * f64::from_bits(scale) / 32.0;
                    Rc::new(builtin_cursor(factor))
                });
            }
        }
        // Unchanged refreshes do not allocate requests or touch the worker mutex.
        if changed {
            self.refresh_demand();
        }
    }

    fn named_icon(&self) -> Option<CursorIcon> {
        match self.current_cursor {
            CursorImageStatus::Named(icon) => Some(icon),
            _ => None,
        }
    }

    fn update_demand(&self, mailbox: &mut Mailbox) {
        mailbox.generation = self.generation;
        mailbox.theme.clone_from(&self.theme);
        mailbox.size = self.size;
        mailbox.desired.clear();
        let Some(icon) = self.named_icon() else {
            return;
        };
        for &scale in &self.active_scales {
            let key = (icon, scale);
            if !self.cache.contains_key(&key) {
                mailbox.desired.push(key);
            }
            // A prepared named cursor no longer needs its theme's default as fallback.
            if icon != CursorIcon::Default && !self.cache.get(&key).is_some_and(Option::is_some) {
                let default = (CursorIcon::Default, scale);
                if !self.cache.contains_key(&default) {
                    mailbox.desired.push(default);
                }
            }
        }
    }

    fn refresh_demand(&self) {
        if let Some(worker) = &self.worker {
            let mut mailbox = worker.shared.mailbox.lock().unwrap();
            self.update_demand(&mut mailbox);
            worker.shared.wake.notify_one();
        }
    }

    fn effective_named_cursor(&self, scale: f64) -> Option<Rc<PreparedCursor>> {
        self.named_icon().map(|icon| {
            self.get_cursor_with_name(icon, scale)
                .unwrap_or_else(|| self.get_default_cursor(scale))
        })
    }

    fn complete(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut mailbox = worker.shared.mailbox.lock().unwrap();
        let Some(completion) = mailbox.completed.take() else {
            return false;
        };
        let mut changed = false;
        if completion.generation == self.generation
            && self.active_scales.contains(&completion.key.1)
        {
            let scale = f64::from_bits(completion.key.1);
            let before = self.effective_named_cursor(scale);
            self.cache
                .insert(completion.key, completion.cursor.map(Rc::new));
            if let (Some(before), Some(after)) = (before, self.effective_named_cursor(scale)) {
                changed = !Rc::ptr_eq(&before, &after);
            }
        }
        // Accept the cache entry and retire its demand under the same lock, so the worker
        // cannot recapture it between completion consumption and the desired-set update.
        self.update_demand(&mut mailbox);
        drop(mailbox);
        worker.shared.wake.notify_one();
        changed
    }

    pub fn check_cursor_image_surface_alive(&mut self) {
        if let CursorImageStatus::Surface(surface) = &self.current_cursor {
            if !surface.alive() {
                self.set_cursor_image(CursorImageStatus::default_named());
            }
        }
    }

    pub fn get_render_cursor(&self, scale: f64) -> RenderCursor {
        match self.current_cursor.clone() {
            CursorImageStatus::Hidden => RenderCursor::Hidden,
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .hotspot
                });
                RenderCursor::Surface { hotspot, surface }
            }
            CursorImageStatus::Named(icon) => RenderCursor::Named {
                cursor: self
                    .get_cursor_with_name(icon, scale)
                    .unwrap_or_else(|| self.get_default_cursor(scale)),
            },
        }
    }

    pub fn is_current_cursor_animated(&self, scale: f64) -> bool {
        match self.current_cursor {
            CursorImageStatus::Named(icon) => self
                .get_cursor_with_name(icon, scale)
                .unwrap_or_else(|| self.get_default_cursor(scale))
                .is_animated_cursor(),
            _ => false,
        }
    }

    pub fn get_cursor_with_name(&self, icon: CursorIcon, scale: f64) -> Option<Rc<PreparedCursor>> {
        self.cache
            .get(&(icon, scale_key(scale)?))
            .and_then(Clone::clone)
    }

    pub fn get_default_cursor(&self, scale: f64) -> Rc<PreparedCursor> {
        self.get_cursor_with_name(CursorIcon::Default, scale)
            .or_else(|| {
                scale_key(scale).and_then(|scale| self.scaled_fallbacks.get(&scale).cloned())
            })
            .unwrap_or_else(|| self.fallback.clone())
    }

    pub fn cursor_image(&self) -> &CursorImageStatus {
        &self.current_cursor
    }

    pub fn set_cursor_image(&mut self, cursor: CursorImageStatus) {
        let previous = self.named_icon();
        self.current_cursor = cursor;
        if self.named_icon() != previous {
            self.refresh_demand();
        }
    }

    fn ensure_env(theme: &str, size: u8) {
        env::set_var("XCURSOR_THEME", theme);
        env::set_var("XCURSOR_SIZE", size.to_string());
    }
}

fn scale_key(scale: f64) -> Option<u64> {
    (scale.is_finite() && scale > 0.0 && scale <= 16.0).then(|| scale.to_bits())
}

pub enum RenderCursor {
    Hidden,
    Surface {
        hotspot: Point<i32, Logical>,
        surface: WlSurface,
    },
    Named {
        cursor: Rc<PreparedCursor>,
    },
}

pub struct PreparedCursor {
    frames: Vec<CursorFrame>,
    duration: u32,
    size: Size<i32, Physical>,
}

impl PreparedCursor {
    pub fn frame(&self, millis: u32) -> (usize, &CursorFrame) {
        let index = frame_index(
            self.frames.iter().map(|frame| frame.delay),
            self.duration,
            millis,
        );
        (index, &self.frames[index])
    }

    pub fn frames(&self) -> &[CursorFrame] {
        &self.frames
    }

    pub fn is_animated_cursor(&self) -> bool {
        self.frames.len() > 1 && self.duration > 0
    }

    pub fn size(&self) -> Size<i32, Physical> {
        self.size
    }
}

pub struct CursorFrame {
    hotspot: Point<i32, Physical>,
    buffer: MemoryRenderBuffer,
    delay: u32,
}

impl CursorFrame {
    pub fn hotspot(&self) -> Point<i32, Physical> {
        self.hotspot
    }
    pub fn buffer(&self) -> &MemoryRenderBuffer {
        &self.buffer
    }
}

fn frame_index(delays: impl IntoIterator<Item = u32>, duration: u32, millis: u32) -> usize {
    if duration == 0 {
        return 0;
    }
    let mut remaining = millis % duration;
    for (index, delay) in delays.into_iter().enumerate() {
        if remaining < delay {
            return index;
        }
        remaining -= delay;
    }
    unreachable!("animation duration is the sum of frame delays")
}

#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    stopped: AtomicBool,
    generation: AtomicU64,
}

#[derive(Default)]
struct Mailbox {
    generation: u64,
    theme: String,
    size: u8,
    // Authoritative uncached demand: at most two icons per active scale, not a FIFO.
    desired: Vec<Key>,
    // Captured scales stay represented until checked against demand or published.
    in_flight: Vec<Key>,
    // Backpressure: at most one fully prepared cursor awaits main-thread publication.
    completed: Option<Completion>,
}

impl Mailbox {
    fn outstanding(&self, key: Key) -> bool {
        self.in_flight.contains(&key)
            || self
                .completed
                .as_ref()
                .is_some_and(|completion| completion.key == key)
    }

    fn capture(&mut self, scales: &mut Vec<u64>) -> Option<CursorIcon> {
        if self.completed.is_some() {
            return None;
        }
        let icon = self.desired.iter().find(|&&key| !self.outstanding(key))?.0;
        scales.clear();
        for &key in &self.desired {
            if key.0 == icon && !self.outstanding(key) {
                scales.push(key.1);
                self.in_flight.push(key);
            }
        }
        Some(icon)
    }

    // A captured job is only permission to check this scale, not stale demand to render it.
    fn prepare_key(&mut self, generation: u64, key: Key) -> bool {
        if self.generation != generation {
            return false;
        }
        if self.desired.contains(&key) {
            return true;
        }
        self.in_flight.retain(|request| *request != key);
        false
    }

    fn invalidate(&mut self, generation: u64) {
        self.generation = generation;
        self.desired.clear();
        self.in_flight.clear();
        self.completed = None;
    }

    fn publish(&mut self, generation: u64, key: Key, cursor: Option<PreparedCursor>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.in_flight.retain(|request| *request != key);
        if !self.desired.contains(&key) {
            return false;
        }
        assert!(
            self.completed.is_none(),
            "cursor completion slot is occupied"
        );
        self.completed = Some(Completion {
            generation,
            key,
            cursor,
        });
        true
    }
}

struct Completion {
    generation: u64,
    key: Key,
    cursor: Option<PreparedCursor>,
}

struct Worker {
    shared: Arc<Shared>,
    // Dropping a JoinHandle detaches, rather than blocking the compositor on SVG decoding.
    // The stop flag wakes the worker and prevents publication; its owned sources drop there.
    _handle: thread::JoinHandle<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Pair the predicate change with the condition-variable mutex to avoid a lost wake.
        let _mailbox = self.shared.mailbox.lock().unwrap();
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }
}

fn worker_loop(shared: Arc<Shared>, tx: calloop::channel::Sender<()>) {
    let mut generation = None;
    let mut theme = None;
    let mut scales = Vec::new();
    loop {
        let (request_generation, mut theme_name, size, icon) = {
            let mut mailbox = shared.mailbox.lock().unwrap();
            let icon = loop {
                if shared.stopped.load(Ordering::Acquire) {
                    return;
                }
                if let Some(icon) = mailbox.capture(&mut scales) {
                    break icon;
                }
                mailbox = shared.wake.wait(mailbox).unwrap();
            };
            let theme_name =
                (generation != Some(mailbox.generation)).then(|| mailbox.theme.clone());
            (mailbox.generation, theme_name, mailbox.size, icon)
        };
        let cancelled = || {
            shared.stopped.load(Ordering::Acquire)
                || shared.generation.load(Ordering::Acquire) != request_generation
        };
        if cancelled() {
            continue;
        }
        // Parsed sources belong to this job, not the warm prepared-cursor cache.
        let mut candidates = None;
        for &scale in &scales {
            // Do not prepare another cursor until the previous scale has been consumed.
            let mut mailbox = shared.mailbox.lock().unwrap();
            while !cancelled()
                && mailbox.generation == request_generation
                && mailbox.completed.is_some()
            {
                mailbox = shared.wake.wait(mailbox).unwrap();
            }
            if cancelled() || mailbox.generation != request_generation {
                break;
            }
            if !mailbox.prepare_key(request_generation, (icon, scale)) {
                continue;
            }
            drop(mailbox);
            if let Some(theme_name) = theme_name.take() {
                generation = Some(request_generation);
                theme = Some(CursorTheme::load(&theme_name));
            }
            if cancelled() {
                break;
            }
            let candidates =
                candidates.get_or_insert_with(|| source_candidates(theme.as_ref().unwrap(), icon));

            let cursor =
                match prepare_candidates(candidates, size, f64::from_bits(scale), &cancelled) {
                    Ok(cursor) => Some(cursor),
                    Err(err) => {
                        if !cancelled() {
                            warn!("error loading cursor {}: {err:#}", icon.name());
                        }
                        None
                    }
                };
            let mut mailbox = shared.mailbox.lock().unwrap();
            if cancelled() {
                break;
            }
            let published = mailbox.publish(request_generation, (icon, scale), cursor);
            drop(mailbox);
            if published && tx.send(()).is_err() {
                return;
            }
        }
        // Release decoded sources before waiting on the final completion or next job.
        drop(candidates);
    }
}

enum CursorSource {
    XCursor(Vec<Image>),
}

struct SourceCandidate {
    path: PathBuf,
    // None is untried; Some(None) is a permanently invalid source for this job.
    parsed: Option<Option<CursorSource>>,
}

fn source_candidates(theme: &CursorTheme, icon: CursorIcon) -> Vec<SourceCandidate> {
    let mut candidates = Vec::new();
    for (alias, name) in std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .enumerate()
    {
        if let Some((path, depth)) = theme.load_icon_with_depth(name) {
            candidates.push((depth, alias, path));
        }
    }
    candidates.sort_by_key(|(depth, alias, _)| (*depth, *alias));
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|(_, _, path)| {
            if !seen.insert(path.clone()) {
                return None;
            }
            Some(SourceCandidate { path, parsed: None })
        })
        .collect()
}

fn prepare_candidates(
    candidates: &mut [SourceCandidate],
    size: u8,
    scale: f64,
    cancelled: &impl Fn() -> bool,
) -> anyhow::Result<PreparedCursor> {
    let mut last_error = anyhow!("no usable cursor in theme or inherited themes");
    for candidate in candidates {
        ensure!(!cancelled(), "cursor loading cancelled");
        if candidate.parsed.is_none() {
            let parsed = load_xcursor(&candidate.path);
            ensure!(!cancelled(), "cursor loading cancelled");
            candidate.parsed = Some(match parsed {
                Ok(source) => Some(source),
                Err(err) => {
                    last_error =
                        err.context(format!("invalid cursor {}", candidate.path.display()));
                    None
                }
            });
        }
        let Some(source) = candidate.parsed.as_ref().unwrap() else {
            continue;
        };
        match prepare_source(source, size, scale, cancelled) {
            Ok(cursor) => return Ok(cursor),
            Err(err) => {
                last_error = err.context(format!("preparing cursor {}", candidate.path.display()))
            }
        }
    }
    Err(last_error)
}

fn read_bounded(path: &Path, limit: usize) -> anyhow::Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "cursor file too large"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "cursor file too large");
    Ok(bytes)
}

fn load_xcursor(path: &Path) -> anyhow::Result<CursorSource> {
    let bytes = read_bounded(path, MAX_FILE_BYTES)?;
    // xcursor allocates from untrusted header sizes, so preflight before invoking its parser.
    let word = |offset: usize| -> anyhow::Result<u32> {
        let bytes = bytes.get(offset..offset + 4).context("truncated XCursor")?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    };
    ensure!(bytes.get(..4) == Some(b"Xcur"), "invalid XCursor magic");
    let header = word(4)? as usize;
    let count = word(12)? as usize;
    ensure!(
        header >= 16 && count <= MAX_FRAMES * 16,
        "invalid XCursor header"
    );
    ensure!(
        header
            .checked_add(count * 12)
            .is_some_and(|end| end <= bytes.len()),
        "truncated XCursor table"
    );
    let mut frame_count = 0;
    let mut total_bytes = 0usize;
    for index in 0..count {
        let toc = header + index * 12;
        if word(toc)? != 0xfffd_0002 {
            continue;
        }
        frame_count += 1;
        ensure!(frame_count <= MAX_FRAMES, "too many XCursor frames");
        let offset = word(toc + 8)? as usize;
        let width = word(offset + 16)?;
        let height = word(offset + 20)?;
        ensure!(
            width > 0 && height > 0 && width <= MAX_DIMENSION && height <= MAX_DIMENSION,
            "invalid XCursor dimensions"
        );
        ensure!(
            word(offset + 8)? > 0 && word(offset + 24)? <= width && word(offset + 28)? <= height,
            "invalid XCursor nominal size or hotspot"
        );
        let length = width as usize * height as usize * 4;
        ensure!(
            offset
                .checked_add(36 + length)
                .is_some_and(|end| end <= bytes.len()),
            "truncated XCursor image"
        );
        total_bytes += length * 2; // The parser keeps two channel orderings.
        ensure!(total_bytes <= MAX_CURSOR_BYTES, "XCursor images too large");
    }
    ensure!(frame_count > 0, "empty XCursor");
    let mut images = parse_xcursor(&bytes).context("invalid XCursor")?;
    for image in &mut images {
        image.pixels_argb = Vec::new();
    }
    Ok(CursorSource::XCursor(images))
}

struct FrameGeometry {
    source_index: usize,
    factor: f64,
    size: Size<i32, Physical>,
    hotspot: Point<i32, Physical>,
    delay: u32,
}

fn scaled_hotspot(value: f64, factor: f64) -> anyhow::Result<i32> {
    let value = (value * factor).round();
    ensure!(
        value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX),
        "scaled cursor hotspot out of bounds"
    );
    Ok(value as i32)
}

fn frame_geometry(
    source_index: usize,
    dimensions: (f64, f64),
    hotspot: (f64, f64),
    delay: u32,
    factor: f64,
) -> anyhow::Result<FrameGeometry> {
    let width = (dimensions.0 * factor).ceil();
    let height = (dimensions.1 * factor).ceil();
    ensure!(
        width >= 1.0
            && height >= 1.0
            && width <= f64::from(MAX_DIMENSION)
            && height <= f64::from(MAX_DIMENSION),
        "rasterized cursor dimensions out of bounds"
    );
    Ok(FrameGeometry {
        source_index,
        factor,
        size: (width as i32, height as i32).into(),
        hotspot: (
            scaled_hotspot(hotspot.0, factor)?,
            scaled_hotspot(hotspot.1, factor)?,
        )
            .into(),
        delay,
    })
}

fn source_geometry(source: &CursorSource, target: f64) -> anyhow::Result<Vec<FrameGeometry>> {
    let count = match source {
        CursorSource::XCursor(images) => images.len(),
    };
    ensure!(
        count > 0 && count <= MAX_FRAMES,
        "invalid cursor frame count"
    );
    let mut frames = Vec::with_capacity(count);
    match source {
        CursorSource::XCursor(images) => {
            let nominal = images
                .iter()
                .min_by(|a, b| {
                    (f64::from(a.size) - target)
                        .abs()
                        .total_cmp(&(f64::from(b.size) - target).abs())
                })
                .context("empty XCursor")?
                .size;
            ensure!(nominal > 0, "invalid XCursor nominal size");
            // Nominal size, not dimensions: animation frames may have different canvases.
            for (index, image) in images
                .iter()
                .enumerate()
                .filter(|(_, image)| image.size == nominal)
            {
                ensure!(
                    image.width > 0
                        && image.height > 0
                        && image.width <= MAX_DIMENSION
                        && image.height <= MAX_DIMENSION,
                    "invalid XCursor dimensions"
                );
                ensure!(
                    image.pixels_rgba.len() == image.width as usize * image.height as usize * 4,
                    "invalid XCursor pixels"
                );
                frames.push(frame_geometry(
                    index,
                    (f64::from(image.width), f64::from(image.height)),
                    (f64::from(image.xhot), f64::from(image.yhot)),
                    image.delay,
                    target / f64::from(nominal),
                )?);
            }
        }
    }
    Ok(frames)
}

fn render_rgba(
    source: &CursorSource,
    geometry: &FrameGeometry,
    destination: &mut tiny_skia::PixmapMut<'_>,
) -> anyhow::Result<()> {
    let transform =
        tiny_skia::Transform::from_scale(geometry.factor as f32, geometry.factor as f32);
    match source {
        CursorSource::XCursor(images) => {
            let image = &images[geometry.source_index];
            // Bilinear sampling treats RGB channels symmetrically. Keep file-order BGRA
            // throughout XCursor preparation and publish it as Argb8888.
            let pixmap =
                tiny_skia::PixmapRef::from_bytes(&image.pixels_rgba, image.width, image.height)
                    .context("invalid XCursor pixels")?;
            let paint = tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Bilinear,
                ..Default::default()
            };
            destination.draw_pixmap(0, 0, pixmap, &paint, transform, None);
        }
    }
    Ok(())
}

fn frame_buffer(
    pixels: &[u8],
    format: Fourcc,
    geometry: &FrameGeometry,
    size: Size<i32, Physical>,
    anchor: Point<i32, Physical>,
) -> MemoryBuffer {
    if geometry.size == size && geometry.hotspot == anchor {
        return MemoryBuffer::from_slice(pixels, format, (size.w, size.h));
    }
    let mut memory = MemoryBuffer::new(format, (size.w, size.h));
    let row_bytes = geometry.size.w as usize * 4;
    let x = (anchor.x - geometry.hotspot.x) as usize;
    let y = (anchor.y - geometry.hotspot.y) as usize;
    for row in 0..geometry.size.h as usize {
        let destination = ((row + y) * size.w as usize + x) * 4;
        memory[destination..destination + row_bytes]
            .copy_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
    }
    memory
}

fn prepare_source(
    source: &CursorSource,
    size: u8,
    scale: f64,
    cancelled: &impl Fn() -> bool,
) -> anyhow::Result<PreparedCursor> {
    ensure!(scale_key(scale).is_some(), "invalid output scale");
    ensure!(!cancelled(), "cursor preparation cancelled");
    let geometry = source_geometry(source, f64::from(size.max(1)) * scale)?;
    // Validate all geometry, timing and final allocation bounds before rendering any frame.
    let (size, anchor) = canvas(&geometry)?;
    let animated = geometry.len() > 1;
    let duration = geometry.iter().try_fold(0u32, |duration, frame| {
        duration
            .checked_add(if animated { frame.delay } else { 0 })
            .context("animation duration overflow")
    })?;
    let mut frames = Vec::with_capacity(geometry.len());
    for geometry in geometry {
        ensure!(!cancelled(), "cursor preparation cancelled");
        let native = geometry.factor == 1.0;
        let format = Fourcc::Argb8888;
        let memory = if native {
            let CursorSource::XCursor(images) = source;
            frame_buffer(
                &images[geometry.source_index].pixels_rgba,
                format,
                &geometry,
                size,
                anchor,
            )
        } else if geometry.size == size && geometry.hotspot == anchor {
            let mut memory = MemoryBuffer::new(format, (size.w, size.h));
            let mut destination =
                tiny_skia::PixmapMut::from_bytes(&mut memory, size.w as u32, size.h as u32)
                    .context("invalid cursor canvas")?;
            render_rgba(source, &geometry, &mut destination)?;
            memory
        } else {
            // Keep the intrinsic bilinear sampling boundary when padding into the union canvas.
            let mut scratch =
                tiny_skia::Pixmap::new(geometry.size.w as u32, geometry.size.h as u32)
                    .context("allocating cursor frame")?;
            render_rgba(source, &geometry, &mut scratch.as_mut())?;
            frame_buffer(scratch.data(), format, &geometry, size, anchor)
        };
        frames.push(CursorFrame {
            hotspot: anchor,
            buffer: MemoryRenderBuffer::from_memory(memory, 1, Transform::Normal, None),
            delay: if animated { geometry.delay } else { 0 },
        });
    }
    Ok(PreparedCursor {
        frames,
        duration,
        size,
    })
}

/// The union is measured relative to the pointer, not each image's top-left corner.
fn canvas(frames: &[FrameGeometry]) -> anyhow::Result<(Size<i32, Physical>, Point<i32, Physical>)> {
    ensure!(!frames.is_empty(), "empty cursor");
    // Include the pointer origin itself. KDE permits hotspots outside the SVG image.
    // Widen before subtraction/addition so even extreme malformed coordinates cannot overflow.
    let (mut left, mut top, mut right, mut bottom) = (0i64, 0i64, 0i64, 0i64);
    for frame in frames {
        ensure!(
            frame.size.w > 0 && frame.size.h > 0,
            "invalid cursor dimensions"
        );
        left = left.max(i64::from(frame.hotspot.x));
        top = top.max(i64::from(frame.hotspot.y));
        right = right.max(i64::from(frame.size.w) - i64::from(frame.hotspot.x));
        bottom = bottom.max(i64::from(frame.size.h) - i64::from(frame.hotspot.y));
    }
    let (width, height) = (left + right, top + bottom);
    ensure!(
        width > 0
            && height > 0
            && width <= i64::from(MAX_DIMENSION)
            && height <= i64::from(MAX_DIMENSION),
        "cursor animation canvas too large"
    );
    let size = Size::from((width as i32, height as i32));
    let anchor = Point::from((left as i32, top as i32));
    ensure!(
        size.w as usize * size.h as usize * 4 * frames.len() <= MAX_CURSOR_BYTES,
        "cursor animation too large"
    );
    Ok((size, anchor))
}

fn builtin_cursor(factor: f64) -> PreparedCursor {
    // A temporary no-I/O fallback must stay cheap even for huge configured cursor sizes.
    let factor = factor.min(2.0);
    let side = (64.0 * factor).round().clamp(1.0, 128.0) as usize;
    let mut memory = MemoryBuffer::new(Fourcc::Abgr8888, (side as i32, side as i32));
    for y in 0..side {
        for x in 0..side {
            let source = ((y * 64 / side) * 64 + x * 64 / side) * 4;
            let target = (y * side + x) * 4;
            let pixel = &FALLBACK_CURSOR_DATA[source..source + 4];
            // The builtin resource is straight RGBA, unlike SVG/XCursor raster output.
            for channel in 0..3 {
                memory[target + channel] =
                    ((u16::from(pixel[channel]) * u16::from(pixel[3]) + 127) / 255) as u8;
            }
            memory[target + 3] = pixel[3];
        }
    }
    PreparedCursor {
        frames: vec![CursorFrame {
            hotspot: (
                (factor.round() as i32).clamp(0, side as i32),
                (factor.round() as i32).clamp(0, side as i32),
            )
                .into(),
            delay: 0,
            buffer: MemoryRenderBuffer::from_memory(memory, 1, Transform::Normal, None),
        }],
        duration: 0,
        size: (side as i32, side as i32).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, xhot: u32, yhot: u32, delay: u32) -> Image {
        Image {
            size: 24,
            width,
            height,
            xhot,
            yhot,
            delay,
            pixels_rgba: [3, 2, 1, 255].repeat(width as usize * height as usize),
            pixels_argb: vec![],
        }
    }

    fn prepare_images(images: Vec<Image>) -> PreparedCursor {
        prepare_source(&CursorSource::XCursor(images), 24, 1.0, &|| false).unwrap()
    }

    fn frame_pixels(cursor: &mut PreparedCursor, index: usize) -> Vec<u8> {
        let mut pixels = Vec::new();
        cursor.frames[index]
            .buffer
            .render()
            .draw(|bytes| {
                pixels = bytes.to_vec();
                Ok::<_, std::convert::Infallible>(Vec::new())
            })
            .unwrap();
        pixels
    }

    #[test]
    fn animation_canvas_keeps_every_frame_and_common_anchor() {
        let mut prepared = prepare_images(vec![
            image(10, 20, 1, 15, 10),
            image(30, 8, 25, 2, 20),
            image(8, 9, 3, 7, 30),
        ]);
        assert_eq!(prepared.size(), Size::from((34, 21)));
        assert_eq!(prepared.frames().len(), 3);
        for frame in prepared.frames() {
            assert_eq!(frame.hotspot(), Point::from((25, 15)));
        }
        assert!(prepared.is_animated_cursor());
        let pixels = frame_pixels(&mut prepared, 0);
        assert_eq!(&pixels[..4], &[0; 4]);
        assert_eq!(&pixels[24 * 4..25 * 4], &[3, 2, 1, 255]);
        assert_eq!(&pixels[34 * 20 * 4..34 * 21 * 4], &[0; 34 * 4]);
    }

    #[test]
    fn legacy_animation_boundaries_keep_zero_delay_frames_without_inventing_time() {
        let prepared = prepare_images(vec![
            image(2, 2, 1, 1, 10),
            image(2, 2, 1, 1, 0),
            image(2, 2, 1, 1, 20),
        ]);
        assert_eq!(prepared.frames().len(), 3);
        for (time, expected) in [(0, 0), (9, 0), (10, 2), (29, 2), (30, 0), (40, 2)] {
            assert_eq!(prepared.frame(time).0, expected, "at {time}ms");
        }
        let still = prepare_images(vec![image(1, 1, 0, 0, u32::MAX)]);
        assert_eq!(still.frame(u32::MAX).0, 0);
        assert!(!still.is_animated_cursor());
        let zero_duration = prepare_images(vec![image(1, 1, 0, 0, 0), image(1, 1, 0, 0, 0)]);
        assert_eq!(zero_duration.frames().len(), 2);
        assert_eq!(zero_duration.frame(u32::MAX).0, 0);
        assert!(!zero_duration.is_animated_cursor());
    }

    #[test]
    fn malformed_geometry_and_animation_are_rejected() {
        assert!(canvas(&[]).is_err());
        for hotspot in [i32::MIN, i32::MAX] {
            let geometry =
                frame_geometry(0, (4.0, 4.0), (f64::from(hotspot), 1.0), 1, 1.0).unwrap();
            assert!(canvas(&[geometry]).is_err());
        }
        assert!(scaled_hotspot(f64::MAX, 16.0).is_err());
        assert!(scaled_hotspot(f64::NAN, 1.0).is_err());
        let overflowing =
            CursorSource::XCursor(vec![image(1, 1, 0, 0, u32::MAX), image(1, 1, 0, 0, 1)]);
        assert!(prepare_source(&overflowing, 24, 1.0, &|| false).is_err());
        assert!(scale_key(f64::NAN).is_none());
        assert!(scale_key(0.0).is_none());
    }

    #[test]
    fn padded_xcursor_resampling_keeps_intrinsic_filter_boundary() {
        let first = || Image {
            pixels_rgba: [0, 0, 128, 128].repeat(8 * 8),
            ..image(8, 8, 0, 0, 20)
        };
        let mut direct =
            prepare_source(&CursorSource::XCursor(vec![first()]), 24, 1.25, &|| false).unwrap();
        let mut padded = prepare_source(
            &CursorSource::XCursor(vec![first(), image(16, 16, 4, 4, 20)]),
            24,
            1.25,
            &|| false,
        )
        .unwrap();
        let pixels = frame_pixels(&mut direct, 0);
        let mut expected = vec![0; 20 * 20 * 4];
        for row in 0..10 {
            let start = ((row + 5) * 20 + 5) * 4;
            expected[start..start + 40].copy_from_slice(&pixels[row * 40..(row + 1) * 40]);
        }
        assert_eq!(padded.size(), Size::from((20, 20)));
        assert_eq!(frame_pixels(&mut padded, 0), expected);
    }

    #[test]
    fn xcursor_animation_groups_by_nominal_size_not_dimensions() {
        let image = |size, width, delay| Image {
            size,
            width,
            height: 2,
            xhot: 1,
            yhot: 1,
            delay,
            pixels_rgba: [3, 2, 1, 255].repeat((width * 2) as usize),
            pixels_argb: vec![],
        };
        let source =
            CursorSource::XCursor(vec![image(24, 2, 10), image(24, 4, 20), image(48, 8, 10)]);
        let mut prepared = prepare_source(&source, 24, 1.0, &|| false).unwrap();
        assert_eq!(prepared.frames().len(), 2);
        assert_eq!(prepared.size().w, 4);
        assert_eq!(&frame_pixels(&mut prepared, 0)[..4], &[3, 2, 1, 255]);
        assert_eq!(&frame_pixels(&mut prepared, 0)[8..16], &[0; 8]);
    }

    #[test]
    fn xcursor_parser_file_bytes_stay_premultiplied_bgra_at_native_size() {
        let mut bytes = b"Xcur".to_vec();
        for word in [
            16u32,
            1,
            1, // File header and one TOC entry.
            0xfffd_0002,
            24,
            28,
            36,
            0xfffd_0002,
            24,
            1,
            1,
            1,
            0,
            0,
            0,           // Image header.
            0x8040_2010, // Premultiplied ARGB, little-endian BGRA on disk.
        ] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let images = parse_xcursor(&bytes).unwrap();
        assert_eq!(images[0].pixels_rgba, [0x10, 0x20, 0x40, 0x80]);
        let mut prepared =
            prepare_source(&CursorSource::XCursor(images), 24, 1.0, &|| false).unwrap();
        assert_eq!(frame_pixels(&mut prepared, 0), [0x10, 0x20, 0x40, 0x80]);
    }

    #[test]
    fn xcursor_fractional_scale_resamples_pixels_and_hotspot() {
        let source = CursorSource::XCursor(vec![Image {
            size: 24,
            width: 24,
            height: 16,
            xhot: 4,
            yhot: 8,
            delay: 0,
            // Premultiplied translucent red, stored as BGRA by the real parser.
            pixels_rgba: [0, 0, 128, 128].repeat(24 * 16),
            pixels_argb: vec![],
        }]);
        for (scale, dimensions, hotspot) in [
            (1.25, (30, 20), (5, 10)),
            (1.5, (36, 24), (6, 12)),
            (1.6, (39, 26), (6, 13)),
        ] {
            let mut prepared = prepare_source(&source, 24, scale, &|| false).unwrap();
            assert_eq!(prepared.size(), Size::from(dimensions));
            assert_eq!(prepared.frame(0).1.hotspot(), Point::from(hotspot));
            let pixels = frame_pixels(&mut prepared, 0);
            let center = ((dimensions.1 / 2 * dimensions.0 + dimensions.0 / 2) * 4) as usize;
            assert_eq!(&pixels[center..center + 4], &[0, 0, 128, 128]);
        }
    }

    fn manager_with_mailbox() -> (CursorManager, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let mut manager = CursorManager::new("", 24);
        manager.worker = Some(Worker {
            shared: shared.clone(),
            _handle: thread::spawn(|| {}),
        });
        (manager, shared)
    }

    fn named_render(manager: &CursorManager, scale: f64) -> Rc<PreparedCursor> {
        let RenderCursor::Named { cursor } = manager.get_render_cursor(scale) else {
            panic!("expected a named cursor");
        };
        cursor
    }

    #[test]
    fn changing_icons_does_not_replace_an_already_prepared_visible_frame() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.prepare([1.0]);
        let default = (CursorIcon::Default, 1.0f64.to_bits());
        let mut scales = Vec::new();
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert_eq!(mailbox.in_flight, vec![default]);
        }
        // Changing demand during rasterization must not duplicate a captured fallback.
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0]);
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .publish(0, default, Some(builtin_cursor(1.0)),));
        manager.set_cursor_image(CursorImageStatus::default_named());
        manager.prepare([1.0]);
        assert!(manager.complete());
        let visible = named_render(&manager, 1.0);
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0]);
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Text));
            assert!(mailbox.publish(0, (CursorIcon::Text, default.1), None));
        }
        assert!(
            !manager.complete(),
            "failed named cursor keeps its fallback"
        );
        manager.set_cursor_image(CursorImageStatus::default_named());
        manager.prepare([1.0]);
        let current = named_render(&manager, 1.0);
        assert!(Rc::ptr_eq(&visible, &current));
        assert!(std::ptr::eq(
            visible.frame(0).1.buffer(),
            current.frame(0).1.buffer()
        ));
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .capture(&mut scales)
            .is_none());
        assert!(!manager.complete(), "no duplicate publication/redraw");
    }

    #[test]
    fn scale_pruning_skips_only_obsolete_captured_scales() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.prepare([1.0, 1.25, 1.5]);
        let first = (CursorIcon::Default, 1.0f64.to_bits());
        let obsolete = (CursorIcon::Default, 1.25f64.to_bits());
        let last = (CursorIcon::Default, 1.5f64.to_bits());
        let mut scales = Vec::new();
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert!(mailbox.publish(0, first, Some(builtin_cursor(1.0))));
        }
        assert!(manager.complete());
        let visible = named_render(&manager, 1.0);
        manager.prepare([1.0, 1.5]);
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert!(!mailbox.prepare_key(0, obsolete));
            assert_eq!(mailbox.in_flight, vec![last]);
            assert!(mailbox.prepare_key(0, last));
            assert!(mailbox.publish(0, last, Some(builtin_cursor(1.5))));
        }
        assert!(manager.complete());
        assert!(manager
            .get_cursor_with_name(CursorIcon::Default, 1.25)
            .is_none());
        assert!(Rc::ptr_eq(&visible, &named_render(&manager, 1.0)));
        assert!(Rc::ptr_eq(
            &named_render(&manager, 1.5),
            &manager
                .get_cursor_with_name(CursorIcon::Default, 1.5)
                .unwrap(),
        ));
    }

    #[test]
    fn removing_and_readding_scales_keeps_completed_and_captured_work_deduplicated() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.prepare([1.0, 1.25]);
        let first = (CursorIcon::Default, 1.0f64.to_bits());
        let second = (CursorIcon::Default, 1.25f64.to_bits());
        let mut scales = Vec::new();
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert!(mailbox.publish(0, first, Some(builtin_cursor(1.0))));
        }
        manager.prepare([]);
        manager.prepare([1.0, 1.25]);
        {
            let mailbox = shared.mailbox.lock().unwrap();
            assert!(mailbox.outstanding(first));
            assert!(mailbox.outstanding(second));
            assert_eq!(mailbox.in_flight, vec![second]);
        }
        assert!(manager.complete());
        let visible = named_render(&manager, 1.0);
        manager.prepare([1.0]);
        assert!(!shared.mailbox.lock().unwrap().prepare_key(0, second));
        manager.prepare([1.0, 1.25]);
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert_eq!(scales, vec![second.1]);
            assert!(mailbox.publish(0, second, Some(builtin_cursor(1.25))));
        }
        assert!(manager.complete());
        assert!(Rc::ptr_eq(&visible, &named_render(&manager, 1.0)));
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .capture(&mut scales)
            .is_none());
    }

    #[test]
    fn current_icon_changes_skip_old_scales_and_do_not_redraw_for_old_completions() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0, 1.25]);
        let first = (CursorIcon::Text, 1.0f64.to_bits());
        let second = (CursorIcon::Text, 1.25f64.to_bits());
        let mut scales = Vec::new();
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Text));
            assert!(mailbox.publish(0, first, Some(builtin_cursor(1.0))));
        }
        // The setter publishes current demand even before the next refresh.
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Pointer));
        let visible = named_render(&manager, 1.0);
        assert!(!manager.complete());
        assert!(manager
            .get_cursor_with_name(CursorIcon::Text, 1.0)
            .is_some());
        assert!(Rc::ptr_eq(&visible, &named_render(&manager, 1.0)));
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert!(!mailbox.prepare_key(0, second));
            assert!(mailbox.in_flight.is_empty());
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Pointer));
            assert!(mailbox.publish(0, (CursorIcon::Pointer, first.1), Some(builtin_cursor(1.0)),));
        }
        assert!(manager.complete());
        assert!(!Rc::ptr_eq(&visible, &named_render(&manager, 1.0)));
        let mailbox = shared.mailbox.lock().unwrap();
        assert!(!mailbox.desired.contains(&(CursorIcon::Default, first.1)));
        assert!(mailbox.desired.contains(&(CursorIcon::Default, second.1)));
        assert_eq!(mailbox.in_flight, vec![(CursorIcon::Pointer, second.1)]);
    }

    #[test]
    fn hidden_cursor_does_not_prepare_the_theme_or_redraw_for_completed_named_work() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.set_cursor_image(CursorImageStatus::Hidden);
        manager.prepare([1.0, 1.25]);
        let mut scales = Vec::new();
        assert!(manager.scaled_fallbacks.is_empty());
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .capture(&mut scales)
            .is_none());
        assert!(matches!(
            manager.get_render_cursor(1.0),
            RenderCursor::Hidden
        ));
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0, 1.25]);
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Text));
            assert!(mailbox.publish(
                0,
                (CursorIcon::Text, 1.0f64.to_bits()),
                Some(builtin_cursor(1.0)),
            ));
        }
        manager.set_cursor_image(CursorImageStatus::Hidden);
        assert!(!manager.complete());
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert!(!mailbox.prepare_key(0, (CursorIcon::Text, 1.25f64.to_bits())));
            assert!(mailbox.capture(&mut scales).is_none());
            assert!(mailbox.desired.is_empty());
        }
        assert!(matches!(
            manager.get_render_cursor(1.25),
            RenderCursor::Hidden
        ));
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0, 1.25]);
        assert!(Rc::ptr_eq(
            &named_render(&manager, 1.0),
            &manager.get_cursor_with_name(CursorIcon::Text, 1.0).unwrap(),
        ));
    }

    #[test]
    fn completion_redraw_tracks_effective_fallback_and_active_scale() {
        let (mut manager, shared) = manager_with_mailbox();
        manager.set_cursor_image(CursorImageStatus::Named(CursorIcon::Text));
        manager.prepare([1.0, 1.25]);
        let first = 1.0f64.to_bits();
        let second = 1.25f64.to_bits();
        let before = named_render(&manager, 1.0);
        let mut scales = Vec::new();
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Text));
            assert!(mailbox.publish(0, (CursorIcon::Text, first), None));
        }
        assert!(!manager.complete());
        assert!(Rc::ptr_eq(&before, &named_render(&manager, 1.0)));
        assert!(shared.mailbox.lock().unwrap().publish(
            0,
            (CursorIcon::Text, second),
            Some(builtin_cursor(1.25)),
        ));
        assert!(manager.complete());
        // Success updates fallback demand immediately, without another prepare call.
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.desired, vec![(CursorIcon::Default, first)]);
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert_eq!(scales, vec![first]);
            assert!(mailbox.publish(0, (CursorIcon::Default, first), None));
        }
        assert!(
            !manager.complete(),
            "missing default does not replace builtin fallback"
        );
        assert!(Rc::ptr_eq(&before, &named_render(&manager, 1.0)));
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .capture(&mut scales)
            .is_none());

        manager.set_cursor_image(CursorImageStatus::default_named());
        manager.prepare([1.0, 1.25]);
        {
            let mut mailbox = shared.mailbox.lock().unwrap();
            assert_eq!(mailbox.capture(&mut scales), Some(CursorIcon::Default));
            assert!(mailbox.publish(0, (CursorIcon::Default, second), Some(builtin_cursor(1.25)),));
        }
        manager.prepare([1.0]);
        assert!(
            !manager.complete(),
            "inactive scale cannot redraw the current cursor"
        );
        assert!(manager
            .get_cursor_with_name(CursorIcon::Default, 1.25)
            .is_none());
    }

    #[test]
    fn reload_discards_pending_completion_and_rejects_in_flight_old_generation() {
        fn assert_send<T: Send>() {}
        assert_send::<PreparedCursor>();
        let key = (CursorIcon::Default, 1.25f64.to_bits());
        let mut mailbox = Mailbox::default();
        mailbox.invalidate(7);
        mailbox.desired.push(key);
        assert!(mailbox.publish(7, key, None));
        mailbox.invalidate(8);
        assert!(mailbox.completed.is_none());
        assert!(mailbox.desired.is_empty());
        mailbox.desired.push(key);
        mailbox.in_flight.push(key);
        assert!(!mailbox.prepare_key(7, key));
        assert!(!mailbox.publish(7, key, None));
        assert_eq!(mailbox.desired, vec![key]);
        assert_eq!(mailbox.in_flight, vec![key]);
        assert!(mailbox.completed.is_none());
        assert!(mailbox.publish(8, key, None));
        assert!(mailbox.in_flight.is_empty());
        assert_eq!(mailbox.completed.unwrap().generation, 8);
    }

    #[test]
    fn stale_publication_cannot_clear_new_generation_job_or_completion() {
        let key = (CursorIcon::Default, 1.25f64.to_bits());
        let pending = (CursorIcon::Text, 1.5f64.to_bits());
        let mut mailbox = Mailbox::default();
        mailbox.invalidate(8);
        mailbox.in_flight.push(key);
        mailbox.desired.push(pending);
        assert!(mailbox.publish(8, pending, Some(builtin_cursor(1.5))));
        assert!(!mailbox.publish(7, key, None));
        assert_eq!(mailbox.in_flight, vec![key]);
        assert_eq!(mailbox.completed.as_ref().unwrap().key, pending);
        assert_eq!(mailbox.completed.as_ref().unwrap().generation, 8);
        assert!(mailbox.completed.as_ref().unwrap().cursor.is_some());
    }

    #[test]
    fn reload_and_stop_wake_full_completion_slot_waiters() {
        use std::sync::mpsc;
        use std::time::Duration;

        for stop in [false, true] {
            let shared = Arc::new(Shared::default());
            let mut manager = CursorManager::new("", 24);
            manager.worker = Some(Worker {
                shared: shared.clone(),
                _handle: thread::spawn(|| {}),
            });
            manager.prepare([1.0]);
            assert!(shared.mailbox.lock().unwrap().publish(
                0,
                (CursorIcon::Default, 1.0f64.to_bits()),
                Some(builtin_cursor(1.0)),
            ));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let waiting_shared = shared.clone();
            let waiter = thread::spawn(move || {
                let mut mailbox = waiting_shared.mailbox.lock().unwrap();
                assert!(mailbox.completed.is_some());
                // Signal under the predicate lock: lifecycle changes can only happen
                // after wait atomically releases it, so this exercises the actual wake.
                ready_tx.send(()).unwrap();
                while mailbox.completed.is_some() && !waiting_shared.stopped.load(Ordering::Acquire)
                {
                    mailbox = waiting_shared.wake.wait(mailbox).unwrap();
                }
                done_tx
                    .send((
                        mailbox.generation,
                        waiting_shared.stopped.load(Ordering::Acquire),
                    ))
                    .unwrap();
            });
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if stop {
                drop(manager.worker.take());
            } else {
                manager.reload("", 24);
            }
            assert_eq!(
                done_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                if stop { (0, true) } else { (1, false) }
            );
            waiter.join().unwrap();
        }
    }
}
