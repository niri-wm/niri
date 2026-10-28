use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use anyhow::{anyhow, ensure, Context};
use resvg::usvg;
use serde::Deserialize;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryBuffer, MemoryRenderBuffer};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Size, Transform};
use smithay::wayland::compositor::with_states;
use xcursor::parser::{parse_xcursor, Image};
mod resolver;
use resolver::ThemeResolver;

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
    xcursor_fallback: bool,
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
    pub fn new(theme: &str, size: u8, xcursor_fallback: bool) -> Self {
        Self::ensure_env(theme, size);
        Self {
            theme: theme.to_owned(),
            size,
            xcursor_fallback,
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

    pub fn reload(&mut self, theme: &str, size: u8, xcursor_fallback: bool) {
        Self::ensure_env(theme, size);
        self.theme = theme.to_owned();
        self.size = size;
        self.xcursor_fallback = xcursor_fallback;
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
        mailbox.xcursor_fallback = self.xcursor_fallback;
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

    #[cfg(test)]
    fn frames(&self) -> &[CursorFrame] {
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
    xcursor_fallback: bool,
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
        let (request_generation, mut theme_name, size, xcursor_fallback, icon) = {
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
            (
                mailbox.generation,
                theme_name,
                mailbox.size,
                mailbox.xcursor_fallback,
                icon,
            )
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
                theme = Some(ThemeResolver::load(&theme_name, &cancelled));
            }
            if cancelled() {
                break;
            }
            let candidates = candidates.get_or_insert_with(|| {
                source_candidates(theme.as_ref().unwrap(), icon, xcursor_fallback, &cancelled)
            });

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

#[derive(Deserialize)]
struct Metadata {
    filename: String,
    nominal_size: f64,
    hotspot_x: f64,
    hotspot_y: f64,
    #[serde(default)]
    delay: u32,
}

struct SvgFrame {
    tree: usvg::Tree,
    metadata: Metadata,
}

enum CursorSource {
    Svg(Vec<SvgFrame>),
    XCursor(Vec<Image>),
}

struct SourceCandidate {
    path: PathBuf,
    scalable: bool,
    // None is untried; Some(None) is a permanently invalid source for this job.
    parsed: Option<Option<CursorSource>>,
}

fn source_candidates(
    theme: &ThemeResolver,
    icon: CursorIcon,
    xcursor_fallback: bool,
    cancelled: &impl Fn() -> bool,
) -> Vec<SourceCandidate> {
    theme.candidates(icon, xcursor_fallback, cancelled)
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
            let parsed = if candidate.scalable {
                load_svg(&candidate.path, cancelled)
            } else {
                load_xcursor(&candidate.path, cancelled)
            };
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
    // Follow theme/artwork symlinks, but never wait for a FIFO writer while opening.
    // This bounds bytes, not filesystem latency or SVG parser execution time.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "cursor source is not an ordinary file");
    ensure!(metadata.len() <= limit as u64, "cursor file too large");
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "cursor file too large");
    Ok(bytes)
}

fn load_svg(path: &Path, cancelled: &impl Fn() -> bool) -> anyhow::Result<CursorSource> {
    ensure!(!cancelled(), "cursor loading cancelled");
    let bytes = read_bounded(&path.join("metadata.json"), 1024 * 1024)?;
    ensure!(!cancelled(), "cursor loading cancelled");
    let metadata: Vec<Metadata> = serde_json::from_slice(&bytes)?;
    ensure!(
        !metadata.is_empty() && metadata.len() <= MAX_FRAMES,
        "invalid frame count"
    );
    validate_svg_delays(metadata.iter().map(|frame| frame.delay), metadata.len())?;
    let mut frames = Vec::with_capacity(metadata.len());
    let mut total_bytes = 0;
    for metadata in metadata {
        ensure!(!cancelled(), "cursor loading cancelled");
        ensure!(
            metadata.nominal_size.is_finite() && metadata.nominal_size > 0.0,
            "invalid nominal_size"
        );
        // Filenames are local to the cursor directory; symlinked cursor directories remain valid.
        ensure!(
            !metadata.filename.is_empty()
                && Path::new(&metadata.filename)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "invalid SVG filename"
        );
        let bytes = read_bounded(&path.join(&metadata.filename), MAX_FILE_BYTES)?;
        ensure!(!cancelled(), "cursor loading cancelled");
        total_bytes += bytes.len();
        ensure!(total_bytes <= MAX_CURSOR_BYTES, "cursor source too large");
        let options = usvg::Options {
            // Cursor SVGs are self-contained vector artwork. Never read external images/fonts.
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        // from_str deliberately excludes compressed SVG expansion beyond our input bound.
        let tree = usvg::Tree::from_str(std::str::from_utf8(&bytes)?, &options)?;
        ensure!(
            metadata.hotspot_x.is_finite() && metadata.hotspot_y.is_finite(),
            "invalid SVG hotspot"
        );
        frames.push(SvgFrame { tree, metadata });
    }
    Ok(CursorSource::Svg(frames))
}

fn validate_svg_delays(delays: impl IntoIterator<Item = u32>, count: usize) -> anyhow::Result<()> {
    if count <= 1 {
        return Ok(());
    }
    let mut duration = 0u32;
    for delay in delays {
        ensure!(delay > 0, "animated SVG delay must be positive");
        duration = duration
            .checked_add(delay)
            .context("SVG animation duration overflow")?;
    }
    Ok(())
}

// Keep this preflight aligned with resvg 0.46's path/render/filter/clip/mask modules.
// It budgets cumulative intermediate surface bytes, not source coordinates or process RSS.
// The conservative filter allowance also covers retained results and pixel-sized scratch buffers.
// Parsing/tree storage, path tessellation and filter-specific CPU complexity are not bounded here.
fn validate_svg_resources(tree: &usvg::Tree, geometry: &FrameGeometry) -> anyhow::Result<()> {
    let size = tiny_skia::IntSize::from_wh(geometry.size.w as u32, geometry.size.h as u32)
        .context("invalid SVG output size")?;
    let transform =
        tiny_skia::Transform::from_scale(geometry.factor as f32, geometry.factor as f32);
    SvgResources {
        remaining: MAX_CURSOR_BYTES as u64,
    }
    .nodes(tree.root(), transform, size, svg_max_bbox(size))
}

fn svg_max_bbox(size: tiny_skia::IntSize) -> tiny_skia::IntRect {
    // resvg::render uses a five-canvas-wide region, despite its internal "4x" comments.
    tiny_skia::IntRect::from_xywh(
        -(size.width() as i32) * 2,
        -(size.height() as i32) * 2,
        size.width() * 5,
        size.height() * 5,
    )
    .unwrap()
}

fn svg_int_rect(rect: usvg::NonZeroRect) -> anyhow::Result<tiny_skia::IntRect> {
    // Match NonZeroRect::to_int_rect without its unwrap on unrepresentable rectangles.
    tiny_skia::IntRect::from_xywh(
        rect.x().floor() as i32,
        rect.y().floor() as i32,
        (rect.width().ceil() as u32).max(1),
        (rect.height().ceil() as u32).max(1),
    )
    .context("SVG intermediate rectangle out of bounds")
}

struct SvgResources {
    remaining: u64,
}

impl SvgResources {
    fn surface(&mut self, size: tiny_skia::IntSize, copies: u64) -> anyhow::Result<()> {
        let bytes = u64::from(size.width())
            .checked_mul(u64::from(size.height()))
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| bytes.checked_mul(copies))
            .context("SVG intermediate size overflow")?;
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .context("SVG intermediate resources exceed safety limit")?;
        Ok(())
    }

    fn nodes(
        &mut self,
        root: &usvg::Group,
        transform: tiny_skia::Transform,
        size: tiny_skia::IntSize,
        max_bbox: tiny_skia::IntRect,
    ) -> anyhow::Result<()> {
        // resvg::render_nodes ignores the subroot group's own transform/properties.
        for node in root.children() {
            match node {
                usvg::Node::Group(group) => self.group(group, transform, size, max_bbox)?,
                usvg::Node::Text(text) => {
                    self.group(text.flattened(), transform, size, max_bbox)?
                }
                usvg::Node::Path(path) if path.is_visible() => {
                    if path.data().bounds().width() != 0.0 && path.data().bounds().height() != 0.0 {
                        if let Some(fill) = path.fill() {
                            self.paint(fill.paint(), transform, max_bbox)?;
                        }
                    }
                    if let Some(stroke) = path.stroke() {
                        self.paint(stroke.paint(), transform, max_bbox)?;
                    }
                }
                usvg::Node::Image(image) if image.is_visible() => {
                    if let usvg::ImageKind::SVG(tree) = image.kind() {
                        self.surface(size, 1)?;
                        self.nodes(tree.root(), transform, size, svg_max_bbox(size))?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn paint(
        &mut self,
        paint: &usvg::Paint,
        transform: tiny_skia::Transform,
        max_bbox: tiny_skia::IntRect,
    ) -> anyhow::Result<()> {
        if let usvg::Paint::Pattern(pattern) = paint {
            let (sx, sy) = transform.pre_concat(pattern.transform()).get_scale();
            // Match resvg's f32 multiplication, rounding and saturating cast exactly.
            let Some(size) = tiny_skia::IntSize::from_wh(
                (pattern.rect().width() * sx).round() as u32,
                (pattern.rect().height() * sy).round() as u32,
            ) else {
                return Ok(()); // resvg skips zero-sized tiles.
            };
            self.surface(size, 1)?;
            // Nested patterns inherit this scale, not the original affine transform.
            self.nodes(
                pattern.root(),
                tiny_skia::Transform::from_scale(sx, sy),
                size,
                max_bbox,
            )?;
        }
        Ok(())
    }

    fn group(
        &mut self,
        group: &usvg::Group,
        transform: tiny_skia::Transform,
        size: tiny_skia::IntSize,
        max_bbox: tiny_skia::IntRect,
    ) -> anyhow::Result<()> {
        let mut transform = transform.pre_concat(group.transform());
        if !group.should_isolate() {
            return self.nodes(group, transform, size, max_bbox);
        }
        let Some(bbox) = group.layer_bounding_box().transform(transform) else {
            return Ok(());
        };
        let bounds = if group.filters().is_empty() {
            // resvg expands non-filtered layers by two pixels on each side.
            (|| {
                tiny_skia::IntRect::from_xywh(
                    (bbox.x().floor() as i32).checked_sub(2)?,
                    (bbox.y().floor() as i32).checked_sub(2)?,
                    (bbox.width().ceil() as u32).checked_add(4)?,
                    (bbox.height().ceil() as u32).checked_add(4)?,
                )
            })()
        } else {
            Some(svg_int_rect(bbox)?)
        };
        let Some(bounds) = bounds.and_then(|bounds| bounds.intersect(&max_bbox)) else {
            return Ok(()); // resvg skips layers outside its canvas bounds.
        };
        let size = bounds.size();
        self.surface(size, 1)?;
        let mut dx = bbox.x();
        let mut dy = bbox.y();
        dx -= bbox.x() - bounds.x() as f32;
        dy -= bbox.y() - bounds.y() as f32;
        transform = tiny_skia::Transform::from_translate(-dx, -dy).pre_concat(transform);
        self.nodes(group, transform, size, max_bbox)?;
        for filter in group.filters() {
            // Unlike group layers, filter::apply_inner does NOT clip this region to max_bbox.
            let Some(region) = filter.rect().transform(transform) else {
                continue;
            };
            let region = svg_int_rect(region)?;
            let source_pixels = u64::from(size.width()) * u64::from(size.height());
            let filter_pixels = u64::from(region.width()) * u64::from(region.height());
            let filter_size = if source_pixels >= filter_pixels {
                size
            } else {
                region.size()
            };
            for primitive in filter.primitives() {
                // Eight full surfaces cover inputs, color-space/copy-on-write copies, output,
                // clipping, retained results, and the blur's two-surface f64 scratch buffer.
                let Some(subregion) = primitive.rect().transform(transform) else {
                    break; // resvg aborts this filter on an invalid primitive region.
                };
                let subregion = svg_int_rect(subregion)?;
                self.surface(filter_size, 8)?;
                if let usvg::filter::Kind::Merge(merge) = primitive.kind() {
                    self.surface(filter_size, merge.inputs().len() as u64)?;
                }
                if let usvg::filter::Kind::Image(image) = primitive.kind() {
                    // feImage starts a new render context bounded by its filter region.
                    let (sx, sy) = transform.get_scale();
                    let image_transform = tiny_skia::Transform::from_row(
                        sx,
                        0.0,
                        0.0,
                        sy,
                        subregion.x() as f32,
                        subregion.y() as f32,
                    );
                    let image_bounds =
                        tiny_skia::IntRect::from_xywh(0, 0, region.width(), region.height())
                            .unwrap();
                    self.nodes(image.root(), image_transform, region.size(), image_bounds)?;
                }
            }
        }
        if let Some(clip) = group.clip_path() {
            self.clip(clip, size)?;
        }
        if let Some(mask) = group.mask() {
            self.mask(mask, transform, size, max_bbox)?;
        }
        Ok(())
    }

    fn clip(&mut self, clip: &usvg::ClipPath, size: tiny_skia::IntSize) -> anyhow::Result<()> {
        // One RGBA clip canvas plus an alpha mask (charged as a full surface).
        self.surface(size, 2)?;
        self.clip_children(clip.root(), size)?;
        if let Some(clip) = clip.clip_path() {
            self.clip(clip, size)?;
        }
        Ok(())
    }

    fn clip_children(
        &mut self,
        root: &usvg::Group,
        size: tiny_skia::IntSize,
    ) -> anyhow::Result<()> {
        // usvg normalizes clip-path paints to black with no strokes or filters.
        for node in root.children() {
            match node {
                usvg::Node::Group(group) => {
                    if let Some(clip) = group.clip_path() {
                        self.surface(size, 1)?;
                        self.clip(clip, size)?;
                    }
                    self.clip_children(group, size)?;
                }
                usvg::Node::Text(text) => self.clip_children(text.flattened(), size)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn mask(
        &mut self,
        mask: &usvg::Mask,
        transform: tiny_skia::Transform,
        size: tiny_skia::IntSize,
        max_bbox: tiny_skia::IntRect,
    ) -> anyhow::Result<()> {
        if mask.root().children().is_empty() {
            return Ok(());
        }
        // RGBA mask canvas and two sequential alpha masks.
        self.surface(size, 2)?;
        self.nodes(mask.root(), transform, size, max_bbox)?;
        if let Some(mask) = mask.mask() {
            self.mask(mask, transform, size, max_bbox)?;
        }
        Ok(())
    }
}

fn load_xcursor(path: &Path, cancelled: &impl Fn() -> bool) -> anyhow::Result<CursorSource> {
    ensure!(!cancelled(), "cursor loading cancelled");
    let bytes = read_bounded(path, MAX_FILE_BYTES)?;
    ensure!(!cancelled(), "cursor loading cancelled");
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
        CursorSource::Svg(frames) => frames.len(),
        CursorSource::XCursor(images) => images.len(),
    };
    ensure!(
        count > 0 && count <= MAX_FRAMES,
        "invalid cursor frame count"
    );
    let mut frames = Vec::with_capacity(count);
    match source {
        CursorSource::Svg(source_frames) => {
            validate_svg_delays(
                source_frames.iter().map(|frame| frame.metadata.delay),
                source_frames.len(),
            )?;
            for (index, frame) in source_frames.iter().enumerate() {
                ensure!(
                    frame.metadata.nominal_size.is_finite() && frame.metadata.nominal_size > 0.0,
                    "invalid nominal_size"
                );
                let geometry = frame_geometry(
                    index,
                    (
                        f64::from(frame.tree.size().width()),
                        f64::from(frame.tree.size().height()),
                    ),
                    (frame.metadata.hotspot_x, frame.metadata.hotspot_y),
                    frame.metadata.delay,
                    target / frame.metadata.nominal_size,
                )?;
                validate_svg_resources(&frame.tree, &geometry)?;
                frames.push(geometry);
            }
        }
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

fn render_bgra(
    source: &CursorSource,
    geometry: &FrameGeometry,
    destination: &mut tiny_skia::PixmapMut<'_>,
) -> anyhow::Result<()> {
    let transform =
        tiny_skia::Transform::from_scale(geometry.factor as f32, geometry.factor as f32);
    match source {
        CursorSource::Svg(frames) => {
            resvg::render(&frames[geometry.source_index].tree, transform, destination);
            // Normalize the intrinsic raster before padding it into the animation canvas.
            for pixel in destination.data_mut().chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
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
        let native = matches!(source, CursorSource::XCursor(_)) && geometry.factor == 1.0;
        let format = Fourcc::Argb8888;
        let memory = if native {
            let CursorSource::XCursor(images) = source else {
                unreachable!()
            };
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
            render_bgra(source, &geometry, &mut destination)?;
            memory
        } else {
            // Keep the intrinsic raster boundary: translating SVG/filter rendering or a
            // bilinear image into the union canvas would expose pixels formerly clipped.
            let mut scratch =
                tiny_skia::Pixmap::new(geometry.size.w as u32, geometry.size.h as u32)
                    .context("allocating cursor frame")?;
            render_bgra(source, &geometry, &mut scratch.as_mut())?;
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
    let mut memory = MemoryBuffer::new(Fourcc::Argb8888, (side as i32, side as i32));
    for y in 0..side {
        for x in 0..side {
            let source = ((y * 64 / side) * 64 + x * 64 / side) * 4;
            let target = (y * side + x) * 4;
            let pixel = &FALLBACK_CURSOR_DATA[source..source + 4];
            // Premultiply the straight RGBA resource directly into the BGRA output.
            for (channel, source_channel) in [2, 1, 0].into_iter().enumerate() {
                memory[target + channel] =
                    ((u16::from(pixel[source_channel]) * u16::from(pixel[3]) + 127) / 255) as u8;
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

    // Absolute theme and inheritance paths avoid global environment changes and host themes.
    pub(super) fn policy_theme() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = env::temp_dir().join(format!(
            "niri-cursor-policy-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("index.theme"),
            format!("[Icon Theme]\nInherits={}\n", path.display()),
        )
        .unwrap();
        path
    }

    fn write_policy_bitmap(theme: &Path, name: &str) {
        std::fs::create_dir_all(theme.join("cursors")).unwrap();
        let mut bytes = b"Xcur".to_vec();
        for word in [
            16u32,
            1,
            1,
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
            0,
            0xff12_3456,
        ] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        std::fs::write(theme.join("cursors").join(name), bytes).unwrap();
    }

    fn write_policy_svg(theme: &Path, name: &str, broken: bool) {
        let path = theme.join("cursors_scalable").join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("metadata.json"),
            r#"[{"filename":"frame.svg","nominal_size":24,"hotspot_x":0,"hotspot_y":0}]"#,
        )
        .unwrap();
        std::fs::write(
            path.join("frame.svg"),
            if broken {
                "not SVG"
            } else {
                "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><path fill='red' d='M0 0H2V2H0Z'/></svg>"
            },
        )
        .unwrap();
    }

    fn write_policy_inheritance(theme: &Path, parents: &[&str]) {
        std::fs::create_dir_all(theme).unwrap();
        std::fs::write(
            theme.join("index.theme"),
            format!("[Icon Theme]\nInherits={}\n", parents.join(",")),
        )
        .unwrap();
    }

    fn policy_fifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;

        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the path is a valid NUL-terminated string and the mode is valid.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn whitespace_separated_inherited_themes_supply_prepared_shapes() {
        let fixture = policy_theme();
        let roots = [fixture.clone()];
        let child = fixture.join("child");
        std::fs::create_dir_all(&child).unwrap();
        write_policy_svg(&fixture.join("first"), CursorIcon::Default.name(), false);
        write_policy_svg(&fixture.join("second"), CursorIcon::Text.name(), false);
        for separator in [" ", "\t", ",", ";"] {
            std::fs::write(
                child.join("index.theme"),
                format!("[Icon Theme]\nInherits=first{separator}second\n"),
            )
            .unwrap();
            let theme = ThemeResolver::with_roots("child", &roots, &|| false);
            for icon in [CursorIcon::Default, CursorIcon::Text] {
                let mut candidates = source_candidates(&theme, icon, false, &|| false);
                let mut cursor = prepare_candidates(&mut candidates, 24, 1.0, &|| false)
                    .expect("each inherited theme must supply its shape");
                assert_eq!(frame_pixels(&mut cursor, 0), [0, 0, 255, 255].repeat(4));
            }
        }
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn resolver_orders_depth_format_alias_roots_and_all_parents_with_cycles() {
        let fixture = policy_theme();
        let roots = [fixture.join("first/icons"), fixture.join("second/icons")];
        let child = roots[0].join("child");
        let other_child = roots[1].join("child");
        let first_parent = roots[0].join("parent-one");
        let other_parent = roots[1].join("parent-one");
        let second_parent = roots[0].join("parent-two");
        let grandparent = roots[0].join("grandparent");
        write_policy_inheritance(&child, &["parent-one", "parent-two"]);
        write_policy_inheritance(&other_child, &["parent-two"]);
        write_policy_inheritance(&first_parent, &["child", "grandparent"]);
        write_policy_inheritance(&other_parent, &["parent-two"]);
        write_policy_inheritance(&second_parent, &["child"]);
        write_policy_inheritance(&grandparent, &["parent-one"]);
        let icon = CursorIcon::Text;
        let name = icon.name();
        let alias = icon.alt_names()[0];
        write_policy_svg(&child, alias, false);
        write_policy_svg(&other_child, name, false);
        write_policy_bitmap(&child, name);
        write_policy_bitmap(&other_child, name);
        write_policy_svg(&first_parent, name, false);
        write_policy_svg(&other_parent, name, false);
        write_policy_svg(&second_parent, name, false);
        write_policy_svg(&grandparent, name, false);
        let theme = ThemeResolver::with_roots("child", &roots, &|| false);
        let candidates = source_candidates(&theme, icon, true, &|| false);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.path.clone())
                .collect::<Vec<_>>(),
            [
                other_child.join("cursors_scalable").join(name),
                child.join("cursors_scalable").join(alias),
                child.join("cursors").join(name),
                other_child.join("cursors").join(name),
                first_parent.join("cursors_scalable").join(name),
                other_parent.join("cursors_scalable").join(name),
                second_parent.join("cursors_scalable").join(name),
                grandparent.join("cursors_scalable").join(name),
            ],
        );
        let scalable = source_candidates(&theme, icon, false, &|| false);
        assert_eq!(scalable.len(), 6);
        assert!(scalable.iter().all(|candidate| candidate.scalable));
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn corrupt_or_incomplete_child_keeps_same_name_inherited_and_other_root_candidates() {
        let fixture = policy_theme();
        let roots = [fixture.join("first/icons"), fixture.join("second/icons")];
        let child = roots[0].join("child");
        let other_child = roots[1].join("child");
        let parent = roots[0].join("parent");
        write_policy_inheritance(&child, &["parent"]);
        write_policy_inheritance(&other_child, &["parent"]);
        write_policy_inheritance(&parent, &["child"]);
        let name = CursorIcon::Default.name();
        write_policy_svg(&child, name, true);
        // An existing scalable directory with absent metadata must not mask inheritance.
        std::fs::create_dir_all(other_child.join("cursors_scalable").join(name)).unwrap();
        write_policy_svg(&parent, name, false);
        let theme = ThemeResolver::with_roots("child", &roots, &|| false);
        let mut candidates = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert_eq!(candidates.len(), 3);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
        assert!(matches!(candidates[0].parsed, Some(None)));
        assert!(matches!(candidates[1].parsed, Some(None)));
        assert!(matches!(candidates[2].parsed, Some(Some(_))));
        write_policy_svg(&other_child, name, false);
        let mut candidates = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
        assert!(matches!(candidates[1].parsed, Some(Some(_))));
        assert!(candidates[2].parsed.is_none());
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn ordinary_file_reader_rejects_fifo_symlinks_devices_and_byte_overflow() {
        use std::os::unix::fs::symlink;

        let fixture = policy_theme();
        let fifo = fixture.join("fifo");
        policy_fifo(&fifo);
        let link = fixture.join("fifo-link");
        symlink(&fifo, &link).unwrap();
        assert!(read_bounded(&fifo, 100).is_err());
        assert!(read_bounded(&link, 100).is_err());
        assert!(read_bounded(Path::new("/dev/null"), 100).is_err());
        assert!(read_bounded(&fixture, 100).is_err());
        let regular = fixture.join("regular");
        std::fs::write(&regular, b"1234").unwrap();
        assert!(read_bounded(&regular, 3).is_err());
        assert_eq!(read_bounded(&regular, 4).unwrap(), b"1234");
        let regular_link = fixture.join("regular-link");
        symlink(regular, &regular_link).unwrap();
        assert_eq!(read_bounded(&regular_link, 4).unwrap(), b"1234");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn fifo_metadata_artwork_and_index_allow_usable_inherited_svg() {
        use std::os::unix::fs::symlink;

        let fixture = policy_theme();
        let roots = [fixture.clone()];
        let name = CursorIcon::Default.name();
        write_policy_svg(&fixture.join("default"), name, false);
        for linked in [false, true] {
            let fifo = fixture.join(format!("pipe-{linked}"));
            policy_fifo(&fifo);
            for part in ["metadata.json", "frame.svg", "index.theme"] {
                let theme_name = format!("theme-{linked}-{part}");
                let child = fixture.join(&theme_name);
                write_policy_svg(&child, name, false);
                let target = if part == "index.theme" {
                    // A missing/invalid index uses the usual default-theme inheritance.
                    child.join(part)
                } else {
                    write_policy_inheritance(&child, &["default"]);
                    child.join("cursors_scalable").join(name).join(part)
                };
                if target.exists() {
                    std::fs::remove_file(&target).unwrap();
                }
                if linked {
                    symlink(&fifo, &target).unwrap();
                } else {
                    policy_fifo(&target);
                }
                let theme = ThemeResolver::with_roots(&theme_name, &roots, &|| false);
                let mut candidates =
                    source_candidates(&theme, CursorIcon::Default, false, &|| false);
                assert_eq!(candidates.len(), 2);
                assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
                if part != "index.theme" {
                    assert!(matches!(candidates[0].parsed, Some(None)));
                    assert!(matches!(candidates[1].parsed, Some(Some(_))));
                }
            }
        }
        let child = fixture.join("oversized-index");
        write_policy_svg(&child, name, true);
        std::fs::write(child.join("index.theme"), vec![b' '; 64 * 1024 + 1]).unwrap();
        let theme = ThemeResolver::with_roots("oversized-index", &roots, &|| false);
        let mut candidates = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert_eq!(candidates.len(), 2);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn symlinked_themes_and_artwork_remain_valid_and_deduplicate_cycles() {
        use std::os::unix::fs::symlink;

        let fixture = policy_theme();
        let child = fixture.join("theme");
        let alias_theme = fixture.join("linked-theme");
        let name = CursorIcon::Default.name();
        write_policy_inheritance(&child, &["linked-theme"]);
        write_policy_svg(&child, name, false);
        symlink(&child, &alias_theme).unwrap();
        let cursor = child.join("cursors_scalable").join(name);
        std::fs::rename(cursor.join("frame.svg"), child.join("art.svg")).unwrap();
        symlink(child.join("art.svg"), cursor.join("frame.svg")).unwrap();
        std::fs::rename(cursor.join("metadata.json"), child.join("metadata.json")).unwrap();
        symlink(child.join("metadata.json"), cursor.join("metadata.json")).unwrap();
        symlink(
            &cursor,
            child
                .join("cursors_scalable")
                .join(CursorIcon::Default.alt_names()[0]),
        )
        .unwrap();
        let theme = ThemeResolver::with_roots("linked-theme", &[fixture.clone()], &|| false);
        let mut candidates = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert_eq!(candidates.len(), 1);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
        assert!(load_svg(&cursor, &|| true).is_err());
        assert!(load_xcursor(&fixture.join("missing"), &|| true).is_err());
        let checks = std::cell::Cell::new(0);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() >= 2
        };
        let error = load_svg(&cursor, &cancelled).err().unwrap();
        assert!(error.to_string().contains("cancelled"));
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn bitmap_only_theme_requires_explicit_fallback() {
        let path = policy_theme();
        write_policy_bitmap(&path, CursorIcon::Default.name());
        let theme = ThemeResolver::with_roots(path.to_str().unwrap(), &[], &|| false);
        assert!(source_candidates(&theme, CursorIcon::Default, false, &|| false).is_empty());
        let mut candidates = source_candidates(&theme, CursorIcon::Default, true, &|| false);
        assert_eq!(candidates.len(), 1);
        let mut cursor = prepare_candidates(&mut candidates, 24, 1.0, &|| false).unwrap();
        assert_eq!(frame_pixels(&mut cursor, 0), [0x56, 0x34, 0x12, 0xff]);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn broken_svg_uses_bitmap_only_when_opted_in() {
        let path = policy_theme();
        write_policy_svg(&path, CursorIcon::Default.name(), true);
        write_policy_bitmap(&path, CursorIcon::Default.name());
        let theme = ThemeResolver::with_roots(path.to_str().unwrap(), &[], &|| false);
        let mut candidates = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].scalable);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_err());
        let mut candidates = source_candidates(&theme, CursorIcon::Default, true, &|| false);
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].scalable);
        assert!(!candidates[1].scalable);
        assert!(prepare_candidates(&mut candidates, 24, 1.0, &|| false).is_ok());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn inherited_alias_and_default_bitmap_sources_are_gated_before_lookup() {
        let path = policy_theme();
        let inherited = path.join("parent");
        std::fs::create_dir(&inherited).unwrap();
        std::fs::write(
            inherited.join("index.theme"),
            format!("[Icon Theme]\nInherits={}\n", inherited.display()),
        )
        .unwrap();
        std::fs::write(
            path.join("index.theme"),
            format!("[Icon Theme]\nInherits={}\n", inherited.display()),
        )
        .unwrap();
        let alias = CursorIcon::Text.alt_names()[0];
        write_policy_bitmap(&inherited, alias);
        write_policy_bitmap(&inherited, CursorIcon::Default.name());
        write_policy_svg(&inherited, CursorIcon::Default.name(), false);
        write_policy_bitmap(&path, CursorIcon::Default.name());
        let theme = ThemeResolver::with_roots(path.to_str().unwrap(), &[], &|| false);
        assert!(source_candidates(&theme, CursorIcon::Text, false, &|| false).is_empty());
        let aliases = source_candidates(&theme, CursorIcon::Text, true, &|| false);
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].path, inherited.join("cursors").join(alias));
        let defaults = source_candidates(&theme, CursorIcon::Default, false, &|| false);
        assert_eq!(defaults.len(), 1);
        assert!(defaults[0].scalable);
        let defaults = source_candidates(&theme, CursorIcon::Default, true, &|| false);
        // Local bitmap outranks inherited SVG: depth remains ahead of format.
        assert!(!defaults[0].scalable);
        assert!(defaults[1].scalable);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn policy_reload_clears_positive_and_negative_cache_and_isolates_generations() {
        let path = policy_theme();
        write_policy_bitmap(&path, CursorIcon::Default.name());
        let theme = ThemeResolver::with_roots(path.to_str().unwrap(), &[], &|| false);
        let (mut manager, shared) = manager_with_mailbox();
        let key = (CursorIcon::Default, 1.0f64.to_bits());
        manager.reload(path.to_str().unwrap(), 24, true);
        manager.prepare([1.0]);
        let old_generation = manager.generation;
        let old_policy = shared.mailbox.lock().unwrap().xcursor_fallback;
        let mut candidates = source_candidates(&theme, key.0, old_policy, &|| false);
        let cursor = prepare_candidates(&mut candidates, 24, 1.0, &|| false).unwrap();
        assert!(shared
            .mailbox
            .lock()
            .unwrap()
            .publish(old_generation, key, Some(cursor)));
        assert!(manager.complete());
        assert!(manager.cache[&key].is_some());

        manager.reload(path.to_str().unwrap(), 24, false);
        assert!(manager.cache.is_empty());
        manager.prepare([1.0]);
        let generation = manager.generation;
        let mut mailbox = shared.mailbox.lock().unwrap();
        assert!(!mailbox.xcursor_fallback);
        assert!(!mailbox.publish(old_generation, key, Some(builtin_cursor(1.0))));
        assert!(mailbox.publish(generation, key, None));
        drop(mailbox);
        manager.complete();
        assert!(manager.cache[&key].is_none());
        assert!(source_candidates(&theme, key.0, false, &|| false).is_empty());
        assert!(!source_candidates(&theme, key.0, old_policy, &|| false).is_empty());
        assert!(Rc::ptr_eq(
            &named_render(&manager, 1.0),
            &manager.scaled_fallbacks[&key.1]
        ));

        manager.reload(path.to_str().unwrap(), 24, true);
        assert!(manager.cache.is_empty());
        manager.prepare([1.0]);
        assert!(shared.mailbox.lock().unwrap().xcursor_fallback);
        let mut candidates = source_candidates(&theme, key.0, true, &|| false);
        let cursor = prepare_candidates(&mut candidates, 24, 1.0, &|| false).unwrap();
        let mut mailbox = shared.mailbox.lock().unwrap();
        assert!(!mailbox.publish(generation, key, None));
        assert!(mailbox.publish(manager.generation, key, Some(cursor)));
        drop(mailbox);
        assert!(manager.complete());
        assert!(manager.cache[&key].is_some());
        std::fs::remove_dir_all(path).unwrap();
    }

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

    fn svg_frame(width: u32, height: u32, xhot: f64, yhot: f64, delay: u32) -> SvgFrame {
        SvgFrame {
            tree: usvg::Tree::from_str(
                &format!("<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}'><path fill='red' d='M0 0H{width}V{height}H0Z'/></svg>"),
                &usvg::Options::default(),
            ).unwrap(),
            metadata: Metadata {
                filename: String::new(),
                nominal_size: 24.0,
                hotspot_x: xhot,
                hotspot_y: yhot,
                delay,
            },
        }
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
    fn union_allocation_limits_are_checked_before_rendering() {
        let source =
            CursorSource::Svg((0..5).map(|_| svg_frame(2048, 2048, 0.0, 0.0, 1)).collect());
        let cancellation_checks = std::cell::Cell::new(0);
        assert!(prepare_source(&source, 24, 1.0, &|| {
            cancellation_checks.set(cancellation_checks.get() + 1);
            false
        })
        .is_err());
        assert_eq!(cancellation_checks.get(), 1, "rendering must not start");
    }

    #[test]
    fn padded_svg_keeps_intrinsic_clipping_and_filter_pixels() {
        let filtered = || {
            let mut frame = svg_frame(8, 8, 0.0, 0.0, 20);
            frame.tree = usvg::Tree::from_str(
                "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><defs><filter id='blur' x='-100%' y='-100%' width='300%' height='300%'><feGaussianBlur stdDeviation='1'/></filter></defs><path fill='red' filter='url(#blur)' d='M-3 -3H11V11H-3Z'/></svg>",
                &usvg::Options::default(),
            ).unwrap();
            frame
        };
        for scale in [1.0, 1.25] {
            let mut direct =
                prepare_source(&CursorSource::Svg(vec![filtered()]), 24, scale, &|| false).unwrap();
            let source = CursorSource::Svg(vec![filtered(), svg_frame(16, 16, 4.0, 4.0, 20)]);
            let mut padded = prepare_source(&source, 24, scale, &|| false).unwrap();
            let pixels = frame_pixels(&mut direct, 0);
            assert!(pixels.iter().any(|channel| *channel != 0));
            let offset = (4.0_f64 * scale).round() as usize;
            let mut expected = vec![0; padded.size().w as usize * padded.size().h as usize * 4];
            let row_bytes = direct.size().w as usize * 4;
            for row in 0..direct.size().h as usize {
                let start = ((row + offset) * padded.size().w as usize + offset) * 4;
                expected[start..start + row_bytes]
                    .copy_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
            }
            assert_eq!(frame_pixels(&mut padded, 0), expected, "at scale {scale}");
        }
    }

    fn svg_resource_source(contents: &str) -> CursorSource {
        let mut frame = svg_frame(24, 24, 0.0, 0.0, 0);
        frame.tree = usvg::Tree::from_str(contents, &usvg::Options::default()).unwrap();
        CursorSource::Svg(vec![frame])
    }

    #[test]
    fn svg_intermediate_pattern_is_rejected_before_rendering() {
        let source = svg_resource_source(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><defs><pattern id='p' patternUnits='userSpaceOnUse' width='10000' height='10000'><rect width='1' height='1'/></pattern></defs><rect width='24' height='24' fill='url(#p)'/></svg>",
        );
        let cancellation_checks = std::cell::Cell::new(0);
        let error = prepare_source(&source, 24, 1.0, &|| {
            cancellation_checks.set(cancellation_checks.get() + 1);
            false
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("SVG intermediate"));
        assert_eq!(cancellation_checks.get(), 1, "rendering must not start");
    }

    #[test]
    fn svg_pattern_preflight_matches_transformed_tiles_and_rounding() {
        let pattern = |width: &str, height: &str, transform: &str| {
            svg_resource_source(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><defs><pattern id='p' patternUnits='userSpaceOnUse' width='{width}' height='{height}' patternTransform='{transform}'><rect width='1' height='1'/></pattern></defs><rect width='24' height='24' fill='url(#p)'/></svg>",
        ))
        };
        // A huge source-space tile can be cheap, and a small tile can be expensive.
        assert!(source_geometry(&pattern("10000", "10000", "scale(.0001)"), 24.0).is_ok());
        assert!(source_geometry(&pattern("100", "100", "scale(100)"), 24.0).is_err());
        // 4096 squared RGBA pixels exactly fit 64 MiB. Do not actually render this tile.
        assert!(source_geometry(&pattern("4096.49", "4096", "scale(1)"), 24.0).is_ok());
        assert!(source_geometry(&pattern("4096.5", "4096", "scale(1)"), 24.0).is_err());
        assert!(source_geometry(&pattern("3276.8", "3276.8", "scale(1)"), 30.0).is_ok());
        assert!(source_geometry(&pattern("3277.6", "3277.6", "scale(1)"), 30.0).is_err());
    }

    #[test]
    fn svg_nested_and_repeated_patterns_share_the_intermediate_budget() {
        let nested = svg_resource_source(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><defs><pattern id='inner' patternUnits='userSpaceOnUse' width='5000' height='5000' patternTransform='scale(2)'><rect width='1' height='1'/></pattern><pattern id='outer' patternUnits='userSpaceOnUse' width='2' height='2'><rect width='2' height='2' fill='url(#inner)'/></pattern></defs><rect width='24' height='24' fill='url(#outer)'/></svg>",
        );
        assert!(source_geometry(&nested, 24.0).is_err());
        let repeated = svg_resource_source(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><defs><pattern id='p' patternUnits='userSpaceOnUse' width='1024' height='1024'><rect width='1' height='1'/></pattern></defs>{}</svg>",
            "<rect width='24' height='24' fill='url(#p)'/>".repeat(17),
        ));
        assert!(source_geometry(&repeated, 24.0).is_err());
    }

    #[test]
    fn svg_layer_bounds_are_clipped_but_filter_regions_are_not() {
        let source = svg_resource_source(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><g opacity='.5'><rect x='-10000' y='-10000' width='20000' height='20000'/></g></svg>",
        );
        let CursorSource::Svg(frames) = &source else {
            unreachable!()
        };
        let size = tiny_skia::IntSize::from_wh(24, 24).unwrap();
        let mut resources = SvgResources {
            remaining: MAX_CURSOR_BYTES as u64,
        };
        resources
            .nodes(
                frames[0].tree.root(),
                tiny_skia::Transform::identity(),
                size,
                svg_max_bbox(size),
            )
            .unwrap();
        assert_eq!(resources.remaining, MAX_CURSOR_BYTES as u64 - 120 * 120 * 4);
        let filtered = svg_resource_source(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><defs><filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='10000' height='10000'><feFlood flood-color='red'/></filter></defs><rect width='24' height='24' filter='url(#f)'/></svg>",
        );
        assert!(source_geometry(&filtered, 24.0).is_err());
    }

    #[test]
    fn svg_resource_validation_preserves_equivalent_pattern_filter_coordinates() {
        for scale in [1.0, 1.25] {
            let mut pixels = Vec::new();
            for multiplier in [1, 1024] {
                let source = svg_resource_source(&format!(
                    "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 {canvas} {canvas}'><defs><pattern id='inner' patternUnits='userSpaceOnUse' width='{tile}' height='{tile}'><rect width='{half}' height='{tile}' fill='red'/></pattern><pattern id='outer' patternUnits='userSpaceOnUse' width='{tile}' height='{tile}'><rect width='{tile}' height='{tile}' fill='url(#inner)'/></pattern><filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='{canvas}' height='{canvas}'><feGaussianBlur stdDeviation='{blur}'/></filter><clipPath id='c'><rect width='{canvas}' height='{canvas}'/></clipPath><mask id='m' maskUnits='userSpaceOnUse' x='0' y='0' width='{canvas}' height='{canvas}'><rect width='{canvas}' height='{canvas}' fill='white'/></mask></defs><g opacity='.75' clip-path='url(#c)' mask='url(#m)'><g filter='url(#f)'><rect width='{canvas}' height='{canvas}' fill='url(#outer)'/></g></g></svg>",
                    canvas = 24 * multiplier,
                    tile = 4 * multiplier,
                    half = 2 * multiplier,
                    blur = f64::from(multiplier) / 2.0,
                ));
                let mut prepared = prepare_source(&source, 24, scale, &|| false).unwrap();
                pixels.push(frame_pixels(&mut prepared, 0));
            }
            assert!(pixels[0].iter().any(|channel| *channel != 0));
            assert_eq!(pixels[0], pixels[1], "at scale {scale}");
        }
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
    fn canvas_preserves_negative_and_outside_svg_hotspots() {
        let source = CursorSource::Svg(vec![
            svg_frame(4, 3, -2.0, 5.0, 10),
            svg_frame(4, 3, 6.0, -1.0, 10),
        ]);
        let mut prepared = prepare_source(&source, 24, 1.0, &|| false).unwrap();
        assert_eq!(prepared.size(), Size::from((12, 9)));
        assert_eq!(prepared.frames().len(), 2);
        for frame in prepared.frames() {
            assert_eq!(frame.hotspot(), Point::from((6, 5)));
        }
        let first = frame_pixels(&mut prepared, 0);
        let offset = 8 * 4;
        assert_eq!(&first[offset..offset + 4], &[0, 0, 255, 255]);
        assert_eq!(&first[..offset], &[0; 32]);
        let second = frame_pixels(&mut prepared, 1);
        let offset = 6 * 12 * 4;
        assert_eq!(&second[offset..offset + 4], &[0, 0, 255, 255]);
        assert_eq!(scaled_hotspot(-4.0, 1.25).unwrap(), -5);
        assert_eq!(scaled_hotspot(40.0, 1.25).unwrap(), 50);
    }

    #[test]
    fn svg_rasterizes_fractional_scale_and_per_frame_nominal_size() {
        let tree = || {
            usvg::Tree::from_str(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='24'><path fill='red' fill-opacity='.5' d='M0 0H32V24H0Z'/></svg>",
            &usvg::Options::default(),
        ).unwrap()
        };
        let source = CursorSource::Svg(vec![
            SvgFrame {
                tree: tree(),
                metadata: Metadata {
                    filename: String::new(),
                    nominal_size: 24.0,
                    hotspot_x: 4.0,
                    hotspot_y: 6.0,
                    delay: 20,
                },
            },
            SvgFrame {
                tree: tree(),
                metadata: Metadata {
                    filename: String::new(),
                    nominal_size: 32.0,
                    hotspot_x: 8.0,
                    hotspot_y: 4.0,
                    delay: 20,
                },
            },
        ]);
        let geometry = source_geometry(&source, 30.0).unwrap();
        assert_eq!(geometry[0].size, Size::from((40, 30)));
        assert_eq!(geometry[0].hotspot, Point::from((5, 8)));
        assert_eq!(geometry[1].size, Size::from((30, 23)));
        assert_eq!(geometry[1].hotspot, Point::from((8, 4)));
        let mut prepared = prepare_source(&source, 24, 1.25, &|| false).unwrap();
        assert_eq!(prepared.size(), Size::from((43, 30)));
        assert_eq!(prepared.frame(0).1.hotspot(), Point::from((8, 8)));
        let pixels = frame_pixels(&mut prepared, 0);
        let pixel = &pixels[3 * 4..4 * 4];
        // Prepared pixels are premultiplied BGRA, including padded SVG frames.
        assert_eq!(pixel[2], pixel[3]);
        assert_eq!(pixel[1], 0);
        assert_eq!(pixel[0], 0);
        assert!((127..=128).contains(&pixel[3]));
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
    fn svg_animation_rejects_zero_or_overflowing_delays() {
        assert!(validate_svg_delays([0], 1).is_ok());
        assert!(validate_svg_delays([20, 30], 2).is_ok());
        assert!(validate_svg_delays([20, 0], 2).is_err());
        assert!(validate_svg_delays([u32::MAX, 1], 2).is_err());
    }

    #[test]
    fn candidates_fall_back_after_raster_and_preparation_failures() {
        let tree = usvg::Tree::from_str(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'/>",
            &usvg::Options::default(),
        )
        .unwrap();
        let oversized = CursorSource::Svg(vec![SvgFrame {
            tree,
            metadata: Metadata {
                filename: String::new(),
                nominal_size: 0.001,
                hotspot_x: 1.0,
                hotspot_y: 1.0,
                delay: 0,
            },
        }]);
        let image = |delay| Image {
            size: 24,
            width: 2,
            height: 2,
            xhot: 1,
            yhot: 1,
            delay,
            pixels_rgba: [0, 0, 255, 255].repeat(4),
            pixels_argb: vec![],
        };
        let overflowing = CursorSource::XCursor(vec![image(u32::MAX), image(1)]);
        let fallback = CursorSource::XCursor(vec![image(0)]);
        let mut candidates = [oversized, overflowing, fallback]
            .into_iter()
            .map(|source| SourceCandidate {
                path: PathBuf::from("/nonexistent-cursor-regression"),
                scalable: false,
                parsed: Some(Some(source)),
            })
            .collect::<Vec<_>>();
        let cursor = prepare_candidates(&mut candidates, 24, 1.25, &|| false).unwrap();
        assert_eq!(cursor.size(), Size::from((3, 3)));
        assert_eq!(cursor.frames().len(), 1);
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

    #[test]
    fn equivalent_svg_coordinate_systems_produce_identical_pixels() {
        let directory = env::temp_dir().join(format!(
            "niri-cursor-svg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut rasters = Vec::new();
        for multiplier in [1, 100] {
            std::fs::write(directory.join("frame.svg"), format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}'><path fill='red' d='M0 0H{}V{}H0Z'/></svg>",
                32 * multiplier, 24 * multiplier, 32 * multiplier, 24 * multiplier,
            )).unwrap();
            std::fs::write(directory.join("metadata.json"), format!(
                r#"[{{"filename":"frame.svg","nominal_size":{},"hotspot_x":{},"hotspot_y":{}}}]"#,
                24 * multiplier, 4 * multiplier, 6 * multiplier,
            )).unwrap();
            let source = load_svg(&directory, &|| false).unwrap();
            rasters.push(prepare_source(&source, 24, 1.0, &|| false).unwrap());
        }
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(rasters[0].size(), Size::from((32, 24)));
        assert_eq!(rasters[0].size(), rasters[1].size());
        assert_eq!(
            rasters[0].frame(0).1.hotspot(),
            rasters[1].frame(0).1.hotspot()
        );
        assert_eq!(
            frame_pixels(&mut rasters[0], 0),
            frame_pixels(&mut rasters[1], 0)
        );
    }

    fn manager_with_mailbox() -> (CursorManager, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let mut manager = CursorManager::new("", 24, false);
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
    fn scale_specific_prepare_failure_does_not_invalidate_preferred_source() {
        let mut candidates = [
            CursorSource::Svg(vec![svg_frame(1200, 4, 0.0, 0.0, 0)]),
            CursorSource::XCursor(vec![image(2, 2, 0, 0, 0)]),
        ]
        .into_iter()
        .map(|source| SourceCandidate {
            path: PathBuf::from("/nonexistent-cursor-regression"),
            scalable: false,
            parsed: Some(Some(source)),
        })
        .collect::<Vec<_>>();
        let fallback = prepare_candidates(&mut candidates, 24, 2.0, &|| false).unwrap();
        assert_eq!(fallback.size(), Size::from((4, 4)));
        let preferred = prepare_candidates(&mut candidates, 24, 1.0, &|| false).unwrap();
        assert_eq!(preferred.size(), Size::from((1200, 4)));
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
            let mut manager = CursorManager::new("", 24, false);
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
                manager.reload("", 24, false);
            }
            assert_eq!(
                done_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                if stop { (0, true) } else { (1, false) }
            );
            waiter.join().unwrap();
        }
    }
}
