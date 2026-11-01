use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

use anyhow::{anyhow, ensure, Context};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Transform};
use smithay::wayland::compositor::with_states;
use xcursor::parser::{parse_xcursor, Image};
use xcursor::CursorTheme;

/// Some default looking `left_ptr` icon.
static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

/// Key is: (CursorIcon, nominal image size)
type XCursorCache = HashMap<(CursorIcon, u32), Rc<XCursor>>;
type IconImagesCache = HashMap<CursorIcon, Option<Rc<IconImages>>>;

pub struct CursorManager {
    theme: CursorTheme,
    size: u8,
    current_cursor: CursorImageStatus,
    /// Every image of each named icon, parsed once from the theme so that changing the cursor
    /// size (e.g. during the shake magnification) never touches the file system.
    icon_images_cache: RefCell<IconImagesCache>,
    named_cursor_cache: RefCell<XCursorCache>,
    dynamic_size_multiplier: Cell<f32>,
}

/// A named cursor resolved for the current size multiplier.
struct NamedCursor {
    cursor: Rc<XCursor>,
    /// Nominal size of the chosen theme image.
    pixel_size: u32,
    /// Scale applied at render time so that the chosen image shows at the desired size.
    rescale: f64,
}

impl CursorManager {
    pub fn new(theme: &str, size: u8) -> Self {
        Self::ensure_env(theme, size);

        let theme = CursorTheme::load(theme);

        Self {
            theme,
            size,
            dynamic_size_multiplier: Cell::new(1.0),
            current_cursor: CursorImageStatus::default_named(),
            icon_images_cache: Default::default(),
            named_cursor_cache: Default::default(),
        }
    }

    /// Reload the cursor theme.
    pub fn reload(&mut self, theme: &str, size: u8) {
        Self::ensure_env(theme, size);
        self.theme = CursorTheme::load(theme);
        self.size = size;
        self.icon_images_cache.get_mut().clear();
        self.named_cursor_cache.get_mut().clear();
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
    pub fn get_render_cursor(&self, scale: i32, fractional_scale: f64) -> RenderCursor {
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
            CursorImageStatus::Named(icon) => {
                self.get_render_cursor_named(icon, scale, fractional_scale)
            }
        }
    }

    fn get_render_cursor_named(
        &self,
        icon: CursorIcon,
        scale: i32,
        fractional_scale: f64,
    ) -> RenderCursor {
        let (icon, named) = match self.resolve_named(icon, scale, fractional_scale) {
            Some(named) => (icon, named),
            None => (
                CursorIcon::Default,
                self.resolve_named(CursorIcon::Default, scale, fractional_scale)
                    .unwrap(),
            ),
        };

        RenderCursor::Named {
            icon,
            scale,
            cursor: named.cursor,
            pixel_size: named.pixel_size,
            rescale: named.rescale,
        }
    }

    pub fn is_current_cursor_animated(&self, scale: i32) -> bool {
        match &self.current_cursor {
            CursorImageStatus::Hidden => false,
            CursorImageStatus::Surface(_) => false,
            CursorImageStatus::Named(icon) => self
                .get_cursor_with_name(*icon, scale, scale as f64)
                .unwrap_or_else(|| self.get_default_cursor(scale, scale as f64))
                .is_animated_cursor(),
        }
    }

    /// Set the common dynamic multiplier for cursor sizes.
    pub fn set_size_multiplier(&mut self, m: f32) {
        self.dynamic_size_multiplier.set(m.max(1.0));
    }

    pub fn size_multiplier(&self) -> f32 {
        self.dynamic_size_multiplier.get()
    }

    /// Pixel size the cursor targets without magnification, considering fractional scaling.
    fn base_pixel_size(&self, scale: i32, fractional_scale: f64) -> u32 {
        let effective_scale = if (fractional_scale - f64::from(scale)).abs() > 0.01 {
            fractional_scale
        } else {
            f64::from(scale)
        };

        (f64::from(self.size) * effective_scale).round().max(1.) as u32
    }

    /// Resolves the theme image to draw for `icon` at the current size multiplier.
    ///
    /// The unmagnified size is whichever theme image is closest to the target pixel size. While
    /// magnified, the desired size is that image's size times the multiplier, and the smallest
    /// theme image covering it gets downscaled to fit; past the largest image the cursor keeps
    /// upscaling (pixelated, but continuous).
    fn resolve_named(
        &self,
        icon: CursorIcon,
        scale: i32,
        fractional_scale: f64,
    ) -> Option<NamedCursor> {
        let images = self.icon_images(icon)?;

        let base_size = images.nearest_size(self.base_pixel_size(scale, fractional_scale));
        let desired = f64::from(base_size) * f64::from(self.dynamic_size_multiplier.get());
        let pixel_size = images.covering_size(desired);

        Some(NamedCursor {
            cursor: self.cursor_with_size(icon, &images, pixel_size),
            pixel_size,
            rescale: desired / f64::from(pixel_size),
        })
    }

    /// Every cursor `resolve_named()` can pick for `icon` while the multiplier grows up to
    /// `max_mult`, along with its nominal image size.
    ///
    /// Used to prepare the textures before a magnification animation needs them.
    pub fn magnified_cursors(
        &self,
        icon: CursorIcon,
        scale: i32,
        fractional_scale: f64,
        max_mult: f64,
    ) -> Vec<(u32, Rc<XCursor>)> {
        let Some(images) = self.icon_images(icon) else {
            return Vec::new();
        };

        let base_size = images.nearest_size(self.base_pixel_size(scale, fractional_scale));
        let top_size = images.covering_size(f64::from(base_size) * max_mult.max(1.));

        let mut sizes: Vec<u32> = images
            .0
            .iter()
            .map(|image| image.size)
            .filter(|size| (base_size..=top_size).contains(size))
            .collect();
        sizes.sort_unstable();
        sizes.dedup();

        sizes
            .into_iter()
            .map(|size| (size, self.cursor_with_size(icon, &images, size)))
            .collect()
    }

    fn cursor_with_size(&self, icon: CursorIcon, images: &IconImages, size: u32) -> Rc<XCursor> {
        self.named_cursor_cache
            .borrow_mut()
            .entry((icon, size))
            .or_insert_with(|| Rc::new(images.cursor_with_size(size)))
            .clone()
    }

    fn icon_images(&self, icon: CursorIcon) -> Option<Rc<IconImages>> {
        self.icon_images_cache
            .borrow_mut()
            .entry(icon)
            .or_insert_with(|| {
                let mut images = Self::load_xcursor_images(&self.theme, icon.name());

                // Check alternative names to account for non-compliant themes.
                if images.is_err() {
                    for name in icon.alt_names() {
                        images = Self::load_xcursor_images(&self.theme, name);
                        if images.is_ok() {
                            break;
                        }
                    }
                }

                if let Err(err) = &images {
                    warn!("error loading xcursor {}: {err:?}", icon.name());
                }

                // The default cursor must always have a fallback.
                if icon == CursorIcon::Default && images.is_err() {
                    images = Ok(Self::fallback_images());
                }

                images.ok().map(|images| Rc::new(IconImages(images)))
            })
            .clone()
    }

    pub fn get_cursor_with_name(
        &self,
        icon: CursorIcon,
        scale: i32,
        fractional_scale: f64,
    ) -> Option<Rc<XCursor>> {
        self.resolve_named(icon, scale, fractional_scale)
            .map(|named| named.cursor)
    }

    /// Get default cursor.
    pub fn get_default_cursor(&self, scale: i32, fractional_scale: f64) -> Rc<XCursor> {
        self.get_cursor_with_name(CursorIcon::Default, scale, fractional_scale)
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

    /// Load every image of the cursor with the given `name` from the file system.
    fn load_xcursor_images(theme: &CursorTheme, name: &str) -> anyhow::Result<Vec<Image>> {
        let _span = tracy_client::span!("load_xcursor_images");

        let path = theme
            .load_icon(name)
            .ok_or_else(|| anyhow!("no default icon"))?;

        let mut file = File::open(path).context("error opening cursor icon file")?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)
            .context("error reading cursor icon file")?;

        let images = parse_xcursor(&buf).context("error parsing cursor icon file")?;
        ensure!(!images.is_empty(), "cursor icon file has no images");

        Ok(images)
    }

    /// Set the common XCURSOR env variables.
    fn ensure_env(theme: &str, size: u8) {
        env::set_var("XCURSOR_THEME", theme);
        env::set_var("XCURSOR_SIZE", size.to_string());
    }

    fn fallback_images() -> Vec<Image> {
        vec![Image {
            size: 32,
            width: 64,
            height: 64,
            xhot: 1,
            yhot: 1,
            delay: 0,
            pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
            pixels_argb: vec![],
        }]
    }
}

/// All images of a named cursor, across every size the theme provides.
///
/// Never empty: `load_xcursor_images()` rejects files without images.
struct IconImages(Vec<Image>);

impl IconImages {
    /// Nominal size of the image closest to `size`.
    fn nearest_size(&self, size: u32) -> u32 {
        self.0
            .iter()
            .map(|image| image.size)
            .min_by_key(|s| s.abs_diff(size))
            .unwrap_or(size)
    }

    /// Nominal size of the smallest image at least as large as `size`, or of the largest image
    /// if none is.
    fn covering_size(&self, size: f64) -> u32 {
        let sizes = self.0.iter().map(|image| image.size);
        sizes
            .clone()
            .filter(|&s| f64::from(s) >= size - 0.001)
            .min()
            .or_else(|| sizes.max())
            .unwrap_or(1)
    }

    /// Builds the (possibly animated) cursor from the images closest to `size`.
    ///
    /// All frames end up the same size, since frames with other dimensions are dropped.
    fn cursor_with_size(&self, size: u32) -> XCursor {
        let (width, height) = self
            .0
            .iter()
            .min_by_key(|image| image.size.abs_diff(size))
            .map_or((0, 0), |image| (image.width, image.height));
        let images: Vec<Image> = self
            .0
            .iter()
            .filter(|image| image.width == width && image.height == height)
            .cloned()
            .collect();
        let animation_duration = images.iter().fold(0, |acc, image| acc + image.delay);

        XCursor {
            images,
            animation_duration,
        }
    }
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
        pixel_size: u32,
        /// Scale applied around the hotspot at render time so that the theme image
        /// of `pixel_size` shows at the magnified size. Equal to 1.0 when not
        /// magnified; below 1.0 when a larger image is downscaled to fit.
        rescale: f64,
    },
}

/// Key is: (CursorIcon, pixel_size, output_integer_scale)
type TextureCache = HashMap<(CursorIcon, u32, i32), Vec<MemoryRenderBuffer>>;

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
        pixel_size: u32,
        output_scale: i32,
        cursor: &XCursor,
        idx: usize,
    ) -> MemoryRenderBuffer {
        self.cache
            .borrow_mut()
            .entry((icon, pixel_size, output_scale))
            .or_insert_with(|| {
                cursor
                    .frames()
                    .iter()
                    .map(|frame| {
                        MemoryRenderBuffer::from_slice(
                            &frame.pixels_rgba,
                            Fourcc::Argb8888,
                            (frame.width as i32, frame.height as i32),
                            output_scale,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn images(sizes: &[u32]) -> IconImages {
        IconImages(
            sizes
                .iter()
                .map(|&size| Image {
                    size,
                    width: size,
                    height: size,
                    xhot: 0,
                    yhot: 0,
                    delay: 0,
                    pixels_rgba: vec![],
                    pixels_argb: vec![],
                })
                .collect(),
        )
    }

    #[test]
    fn magnified_size_is_continuous_across_theme_sizes() {
        let images = images(&[24, 48, 96]);
        let base = images.nearest_size(24);

        let mut last_visible = 0.;
        for step in 0..=60 {
            let desired = f64::from(base) * (1. + f64::from(step) * 0.1);
            let size = images.covering_size(desired);
            let rescale = desired / f64::from(size);
            let visible = f64::from(images.cursor_with_size(size).frames()[0].size) * rescale;

            assert!(
                (visible - desired).abs() < 1e-9,
                "visible size must match desired"
            );
            assert!(visible > last_visible, "size must grow monotonically");
            last_visible = visible;
        }
    }

    #[test]
    fn unmagnified_cursor_is_not_rescaled() {
        let images = images(&[24, 32, 48]);
        let base = images.nearest_size(30);
        assert_eq!(base, 32);
        assert_eq!(images.covering_size(f64::from(base)), 32);
    }
}
