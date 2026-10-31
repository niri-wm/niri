use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::{env, thread};

use anyhow::{anyhow, bail, Context};
use librsvg_rebind::prelude::HandleExt;
use pangocairo::cairo::{self, ImageSurface};
use serde::{de, Deserialize, Deserializer};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::calloop::channel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Transform};
use smithay::wayland::compositor::with_states;
use xcursor::parser::{parse_xcursor, Image};
use xcursor::CursorTheme;

/// Some default looking `left_ptr` icon.
static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

struct CachedCursor {
    cursor: Option<Rc<XCursor>>,
    pending: bool,
}

type XCursorCache = HashMap<(CursorIcon, i32), CachedCursor>;

pub struct CursorManager {
    theme: CursorTheme,
    size: u8,
    current_cursor: CursorImageStatus,
    named_cursor_cache: RefCell<XCursorCache>,
    last_cursor: RefCell<Option<(i32, CursorIcon, Rc<XCursor>)>>,
    req_tx: mpsc::Sender<RasterizeRequest>,
    generation: u64,
}

impl CursorManager {
    pub fn new(theme: &str, size: u8, rasterized_tx: channel::Sender<RasterizeResponse>) -> Self {
        Self::ensure_env(theme, size);

        let theme = CursorTheme::load(theme);

        let (req_tx, req_rx) = mpsc::channel();
        if let Err(e) = thread::Builder::new()
            .name("scalable-cursor-rasterizer".to_owned())
            .spawn(move || {
                while let Ok(request) = req_rx.recv() {
                    let result = rasterize_svg(&request);
                    let _ = rasterized_tx.send(RasterizeResponse {
                        icon: request.icon,
                        scale: request.scale,
                        generation: request.generation,
                        result,
                    });
                }
            })
        {
            warn!("error spawning cursor rasterizer thread: {:?}", e);
        }

        Self {
            theme,
            size,
            current_cursor: CursorImageStatus::default_named(),
            named_cursor_cache: Default::default(),
            last_cursor: Default::default(),
            req_tx,
            generation: 0,
        }
    }

    /// Reload the cursor theme.
    pub fn reload(&mut self, theme: &str, size: u8) {
        Self::ensure_env(theme, size);
        self.theme = CursorTheme::load(theme);
        self.size = size;
        self.named_cursor_cache.get_mut().clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Checks if the cursor WlSurface is alive, and if not, cleans it up.
    pub fn check_cursor_image_surface_alive(&mut self) {
        if let CursorImageStatus::Surface(surface) = &self.current_cursor {
            if !surface.alive() {
                self.current_cursor = CursorImageStatus::default_named();
            }
        }
    }

    /// Get the current rendering cursor.
    pub fn get_render_cursor(&self, scale: i32) -> RenderCursor {
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
            CursorImageStatus::Named(icon) => self.get_render_cursor_named(icon, scale),
        }
    }

    fn get_render_cursor_named(&self, icon: CursorIcon, scale: i32) -> RenderCursor {
        let (cursor, pending) = self.get_cached_cursor(icon, scale);

        if pending {
            if let Some((scale, icon, cursor)) = self.last_cursor.borrow().as_ref().cloned() {
                return RenderCursor::Named {
                    icon,
                    scale,
                    cursor,
                };
            }
        }

        let cursor = cursor.unwrap_or_else(|| self.get_default_cursor(scale));
        let changed = match self.last_cursor.borrow().as_ref() {
            Some((last_scale, last_icon, last_cursor)) => {
                *last_scale != scale || *last_icon != icon || !Rc::ptr_eq(last_cursor, &cursor)
            }
            None => true,
        };
        if changed {
            *self.last_cursor.borrow_mut() = Some((scale, icon, Rc::clone(&cursor)));
        }
        RenderCursor::Named {
            icon,
            scale,
            cursor,
        }
    }

    pub fn is_current_cursor_animated(&self, scale: i32) -> bool {
        match &self.current_cursor {
            CursorImageStatus::Hidden => false,
            CursorImageStatus::Surface(_) => false,
            CursorImageStatus::Named(icon) => {
                let (cursor, pending) = self.get_cached_cursor(*icon, scale);
                if pending {
                    let last = self.last_cursor.borrow();
                    if let Some((_, _, last_cursor)) = last.as_ref() {
                        return last_cursor.is_animated_cursor();
                    }
                }
                cursor
                    .unwrap_or_else(|| self.get_default_cursor(scale))
                    .is_animated_cursor()
            }
        }
    }

    /// Get named cursor for the given `icon` and `scale`.
    pub fn get_cursor_with_name(&self, icon: CursorIcon, scale: i32) -> Option<Rc<XCursor>> {
        self.get_cached_cursor(icon, scale).0
    }

    /// Returns the cached cursor and whether its scalable version is still rasterizing.
    fn get_cached_cursor(&self, icon: CursorIcon, scale: i32) -> (Option<Rc<XCursor>>, bool) {
        let mut cache = self.named_cursor_cache.borrow_mut();
        let entry = cache
            .entry((icon, scale))
            .or_insert_with_key(|(icon, scale)| {
                let size = self.size as i32 * scale;
                let mut cursor = Self::load_xcursor(&self.theme, icon.name(), size);

                // Check alternative names to account for non-compliant themes.
                if cursor.is_err() {
                    for name in icon.alt_names() {
                        cursor = Self::load_xcursor(&self.theme, name, size);
                        if cursor.is_ok() {
                            break;
                        }
                    }
                }

                if let Err(err) = &cursor {
                    warn!("error loading xcursor {}@{size}: {err:?}", icon.name());
                }

                // The default cursor must always have a fallback.
                if *icon == CursorIcon::Default && cursor.is_err() {
                    cursor = Ok(Self::fallback_cursor());
                }

                let pending = if let Some(dir) =
                    self.theme.load_scalable(icon.name()).or_else(|| {
                        icon.alt_names()
                            .iter()
                            .find_map(|name| self.theme.load_scalable(name))
                    }) {
                    self.req_tx
                        .send(RasterizeRequest {
                            icon: *icon,
                            scale: *scale,
                            size,
                            dir,
                            generation: self.generation,
                        })
                        .is_ok()
                } else {
                    false
                };
                CachedCursor {
                    cursor: cursor.ok().map(Rc::new),
                    pending,
                }
            });

        (entry.cursor.as_ref().map(Rc::clone), entry.pending)
    }

    pub fn apply_rasterized(&self, resp: RasterizeResponse) -> bool {
        if resp.generation != self.generation {
            return false;
        }

        let frames = match resp.result {
            Ok(frames) => frames,
            Err(e) => {
                warn!(
                    "error rasterizing SVG cursor {:?}@{}: {}",
                    resp.icon, resp.scale, e
                );
                if let Some(cached) = self
                    .named_cursor_cache
                    .borrow_mut()
                    .get_mut(&(resp.icon, resp.scale))
                {
                    cached.pending = false;
                }
                return true;
            }
        };

        let animation_duration = frames.iter().fold(0, |dur, f| dur + f.delay);
        self.named_cursor_cache.borrow_mut().insert(
            (resp.icon, resp.scale),
            CachedCursor {
                cursor: Some(Rc::new(XCursor {
                    images: frames
                        .into_iter()
                        .map(|f| Image {
                            size: self.size as u32 * resp.scale as u32,
                            width: f.width,
                            height: f.height,
                            xhot: f.xhot,
                            yhot: f.yhot,
                            delay: f.delay,
                            pixels_rgba: f.pixels.into(),
                            pixels_argb: Vec::new(),
                        })
                        .collect(),
                    animation_duration,
                })),
                pending: false,
            },
        );
        true
    }

    /// Get default cursor.
    pub fn get_default_cursor(&self, scale: i32) -> Rc<XCursor> {
        // The default cursor always has a fallback.
        self.get_cursor_with_name(CursorIcon::Default, scale)
            .unwrap()
    }

    /// Currently used cursor_image as a cursor provider.
    pub fn cursor_image(&self) -> &CursorImageStatus {
        &self.current_cursor
    }

    /// Set new cursor image provider.
    pub fn set_cursor_image(&mut self, cursor: CursorImageStatus) {
        self.current_cursor = cursor;
    }

    /// Load the cursor with the given `name` from the file system picking the closest
    /// one to the given `size`.
    fn load_xcursor(theme: &CursorTheme, name: &str, size: i32) -> anyhow::Result<XCursor> {
        let _span = tracy_client::span!("load_xcursor");

        let path = theme
            .load_icon(name)
            .ok_or_else(|| anyhow!("no default icon"))?;

        let mut file = File::open(path).context("error opening cursor icon file")?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)
            .context("error reading cursor icon file")?;

        let mut images = parse_xcursor(&buf).context("error parsing cursor icon file")?;

        let (width, height) = images
            .iter()
            .min_by_key(|image| (size - image.size as i32).abs())
            .map(|image| (image.width, image.height))
            .unwrap();

        images.retain(move |image| image.width == width && image.height == height);

        let animation_duration = images.iter().fold(0, |acc, image| acc + image.delay);

        Ok(XCursor {
            images,
            animation_duration,
        })
    }

    /// Set the common XCURSOR env variables.
    fn ensure_env(theme: &str, size: u8) {
        env::set_var("XCURSOR_THEME", theme);
        env::set_var("XCURSOR_SIZE", size.to_string());
    }

    fn fallback_cursor() -> XCursor {
        let images = vec![Image {
            size: 32,
            width: 64,
            height: 64,
            xhot: 1,
            yhot: 1,
            delay: 0,
            pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
            pixels_argb: vec![],
        }];

        XCursor {
            images,
            animation_duration: 0,
        }
    }
}

// See https://invent.kde.org/plasma/breeze/-/blob/master/cursors/svg-cursor-format.schema.json
#[derive(Debug, Clone, Deserialize)]
struct SvgCursorMeta {
    filename: String,
    #[serde(deserialize_with = "unsigned_float")]
    nominal_size: f64,
    hotspot_x: f64,
    hotspot_y: f64,
    #[serde(default)]
    delay: u32,
}

fn unsigned_float<'d, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'d>,
{
    let res = f64::deserialize(deserializer)?;
    if res <= 0. {
        Err(de::Error::custom(format!(
            "{} should be greater than 0",
            res
        )))
    } else {
        Ok(res)
    }
}

pub struct RasterizeRequest {
    pub icon: CursorIcon,
    pub scale: i32,
    pub size: i32,
    pub dir: PathBuf,
    pub generation: u64,
}

pub struct RasterizedFrame {
    pub width: u32,
    pub height: u32,
    pub xhot: u32,
    pub yhot: u32,
    pub delay: u32,
    pub pixels: Box<[u8]>,
}

pub struct RasterizeResponse {
    pub icon: CursorIcon,
    pub scale: i32,
    pub generation: u64,
    pub result: anyhow::Result<Vec<RasterizedFrame>>,
}

fn rasterize_svg(request: &RasterizeRequest) -> anyhow::Result<Vec<RasterizedFrame>> {
    let metadata = fs::read(request.dir.join("metadata.json"))?;
    let metas: Vec<SvgCursorMeta> = serde_json::from_slice(&metadata)?;

    let mut frames = Vec::with_capacity(metas.len());
    for meta in &metas {
        let svg_path = request.dir.join(&meta.filename);
        let Some(svg_path) = svg_path.to_str() else {
            bail!("failed to get svg path {svg_path:?}");
        };
        let Some(handle) = librsvg_rebind::Handle::from_file(svg_path)? else {
            bail!("failed to get handle from file {svg_path}");
        };
        let Some((svg_width, svg_height)) = handle.intrinsic_size_in_pixels() else {
            bail!("cursor has no intrinsic size");
        };
        let factor = request.size as f64 / meta.nominal_size;
        let width = ((svg_width * factor).ceil() as u32).max(1);
        let height = ((svg_height * factor).ceil() as u32).max(1);
        let surface = ImageSurface::create(cairo::Format::ARgb32, width as i32, height as i32)?;
        let cr = cairo::Context::new(&surface)?;
        let viewport = librsvg_rebind::Rectangle::new(0., 0., width as f64, height as f64);
        handle.render_document(&cr, &viewport)?;

        let stride = surface.stride();
        let row_len = width as usize * 4;
        let mut pixels = Vec::with_capacity(row_len * height as usize);
        surface.with_data(|data| {
            for row in data.chunks_exact(stride as usize).take(height as usize) {
                pixels.extend_from_slice(&row[..row_len]);
            }
        })?;
        frames.push(RasterizedFrame {
            width,
            height,
            xhot: ((meta.hotspot_x * factor).round() as u32).clamp(0, width),
            yhot: ((meta.hotspot_y * factor).round() as u32).clamp(0, height),
            delay: meta.delay,
            pixels: pixels.into_boxed_slice(),
        });
    }

    if frames.is_empty() {
        bail!("no image found in {:?}", request.dir);
    }

    Ok(frames)
}

/// The cursor prepared for renderer.
pub enum RenderCursor {
    Hidden,
    Surface {
        hotspot: Point<i32, Logical>,
        surface: WlSurface,
    },
    Named {
        icon: CursorIcon,
        scale: i32,
        cursor: Rc<XCursor>,
    },
}

type TextureCache = HashMap<(CursorIcon, i32), Vec<MemoryRenderBuffer>>;

#[derive(Default)]
pub struct CursorTextureCache {
    cache: RefCell<TextureCache>,
}

impl CursorTextureCache {
    pub fn clear(&mut self) {
        self.cache.get_mut().clear();
    }

    pub fn get(
        &self,
        icon: CursorIcon,
        scale: i32,
        cursor: &XCursor,
        idx: usize,
    ) -> MemoryRenderBuffer {
        self.cache
            .borrow_mut()
            .entry((icon, scale))
            .or_insert_with(|| {
                cursor
                    .frames()
                    .iter()
                    .map(|frame| {
                        MemoryRenderBuffer::from_slice(
                            &frame.pixels_rgba,
                            Fourcc::Argb8888,
                            (frame.width as i32, frame.height as i32),
                            scale,
                            Transform::Normal,
                            None,
                        )
                    })
                    .collect()
            })[idx]
            .clone()
    }
}

// The XCursorBuffer implementation is inspired by `wayland-rs`, thus provided under MIT license.

/// The state of the `NamedCursor`.
pub struct XCursor {
    /// The image for the underlying named cursor.
    images: Vec<Image>,
    /// The total duration of the animation.
    animation_duration: u32,
}

impl XCursor {
    /// Given a time, calculate which frame to show, and how much time remains until the next frame.
    ///
    /// Time will wrap, so if for instance the cursor has an animation lasting 100ms,
    /// then calling this function with 5ms and 105ms as input gives the same output.
    pub fn frame(&self, mut millis: u32) -> (usize, &Image) {
        if self.animation_duration == 0 {
            return (0, &self.images[0]);
        }

        millis %= self.animation_duration;

        let mut res = 0;
        for (i, img) in self.images.iter().enumerate() {
            if millis < img.delay {
                res = i;
                break;
            }
            millis -= img.delay;
        }

        (res, &self.images[res])
    }

    /// Get the frames for the given `XCursor`.
    pub fn frames(&self) -> &[Image] {
        &self.images
    }

    /// Check whether the cursor is animated.
    pub fn is_animated_cursor(&self) -> bool {
        self.images.len() > 1
    }

    /// Get hotspot for the given `image`.
    pub fn hotspot(image: &Image) -> Point<i32, Physical> {
        (image.xhot as i32, image.yhot as i32).into()
    }
}
