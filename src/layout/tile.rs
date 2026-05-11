use core::f64;
use std::rc::Rc;

use niri_config::utils::MergeWith as _;
use niri_config::{Color, CornerRadius, GradientInterpolation};
use niri_ipc::WindowLayout;
use smithay::backend::renderer::element::{Element, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};

use super::focus_ring::{FocusRing, FocusRingRenderElement};
use super::opening_window::{OpenAnimation, OpeningWindowRenderElement};
use super::shadow::Shadow;
use super::{
    HitType, LayoutElement, LayoutElementRenderElement, LayoutElementRenderSnapshot, Options,
    SizeFrac, RESIZE_ANIMATION_THRESHOLD,
};
use crate::animation::{Animation, Clock};
use crate::layout::SizingMode;
use crate::niri_render_elements;
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::clipped_surface::{ClippedSurfaceRenderElement, RoundedCornerDamage};
use crate::render_helpers::damage::ExtraDamage;
use crate::render_helpers::offscreen::{OffscreenBuffer, OffscreenRenderElement};
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::resize::ResizeRenderElement;
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::snapshot::RenderSnapshot;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::xray::{Xray, XrayPos};
use crate::render_helpers::{RenderCtx, RenderTarget};
use crate::utils::transaction::Transaction;
use crate::utils::{
    baba_is_float_offset, round_logical_in_physical, round_logical_in_physical_max1,
};

/// Decomposes `tile - window` into up to 4 axis-aligned rectangles for the
/// fullscreen backdrop. Each returned rect carries a `CornerRadius` whose
/// non-zero corners coincide with the tile's outer corners; corners that
/// butt against the window's edge are zero.
///
/// Order: left bar, right bar, top middle, bottom middle. Strips that would
/// have zero width or height are omitted. If `window` doesn't overlap `tile`,
/// the whole `tile` is returned with its full corner radius.
fn backdrop_clip_rects(
    tile: Rectangle<f64, Logical>,
    window: Rectangle<f64, Logical>,
    tile_corner_radius: CornerRadius,
) -> Vec<(Rectangle<f64, Logical>, CornerRadius)> {
    let tile_left = tile.loc.x;
    let tile_top = tile.loc.y;
    let tile_right = tile_left + tile.size.w;
    let tile_bottom = tile_top + tile.size.h;

    // Clamp the window into the tile so we never produce negative-dimension strips.
    let window_left = window.loc.x.clamp(tile_left, tile_right);
    let window_top = window.loc.y.clamp(tile_top, tile_bottom);
    let window_right = (window.loc.x + window.size.w).clamp(tile_left, tile_right);
    let window_bottom = (window.loc.y + window.size.h).clamp(tile_top, tile_bottom);

    // Window doesn't overlap the tile: backdrop is the whole tile.
    if window_right <= window_left || window_bottom <= window_top {
        return vec![(tile, tile_corner_radius)];
    }

    let mut out = Vec::with_capacity(4);

    // Left bar: covers the strip left of the window, full tile height.
    // Owns the tile's top-left and bottom-left outer corners.
    if window_left > tile_left {
        out.push((
            Rectangle::new(
                Point::from((tile_left, tile_top)),
                Size::from((window_left - tile_left, tile_bottom - tile_top)),
            ),
            CornerRadius {
                top_left: tile_corner_radius.top_left,
                top_right: 0.,
                bottom_right: 0.,
                bottom_left: tile_corner_radius.bottom_left,
            },
        ));
    }

    // Right bar: strip right of the window, full tile height.
    // Owns the tile's top-right and bottom-right outer corners.
    if window_right < tile_right {
        out.push((
            Rectangle::new(
                Point::from((window_right, tile_top)),
                Size::from((tile_right - window_right, tile_bottom - tile_top)),
            ),
            CornerRadius {
                top_left: 0.,
                top_right: tile_corner_radius.top_right,
                bottom_right: tile_corner_radius.bottom_right,
                bottom_left: 0.,
            },
        ));
    }

    // Top middle: above the window, between (or replacing) the bars. Owns a
    // tile-outer corner only on a side where no L/R bar exists (i.e., the
    // window is flush with that tile edge). When both bars exist, both top
    // corners butt against bars and stay sharp; when neither bar exists
    // (letterbox), both top corners are tile-outer corners.
    if window_top > tile_top {
        out.push((
            Rectangle::new(
                Point::from((window_left, tile_top)),
                Size::from((window_right - window_left, window_top - tile_top)),
            ),
            CornerRadius {
                top_left: if window_left <= tile_left {
                    tile_corner_radius.top_left
                } else {
                    0.
                },
                top_right: if window_right >= tile_right {
                    tile_corner_radius.top_right
                } else {
                    0.
                },
                bottom_right: 0.,
                bottom_left: 0.,
            },
        ));
    }

    // Bottom middle: symmetric to top middle.
    if window_bottom < tile_bottom {
        out.push((
            Rectangle::new(
                Point::from((window_left, window_bottom)),
                Size::from((window_right - window_left, tile_bottom - window_bottom)),
            ),
            CornerRadius {
                top_left: 0.,
                top_right: 0.,
                bottom_right: if window_right >= tile_right {
                    tile_corner_radius.bottom_right
                } else {
                    0.
                },
                bottom_left: if window_left <= tile_left {
                    tile_corner_radius.bottom_left
                } else {
                    0.
                },
            },
        ));
    }

    out
}

/// Toplevel window with decorations.
#[derive(Debug)]
pub struct Tile<W: LayoutElement> {
    /// The toplevel window itself.
    window: W,

    /// The border around the window.
    border: FocusRing,

    /// The focus ring around the window.
    focus_ring: FocusRing,

    /// The shadow around the window.
    shadow: Shadow,

    /// This tile's current sizing mode.
    ///
    /// This will update only when the `window` actually goes maximized or fullscreen, rather than
    /// right away, to avoid black backdrop flicker before the window has had a chance to resize.
    sizing_mode: SizingMode,

    /// The black backdrop for fullscreen windows.
    fullscreen_backdrop: SolidColorBuffer,

    /// Whether the tile should float upon unfullscreening.
    pub(super) restore_to_floating: bool,

    /// The size that the window should assume when going floating.
    ///
    /// This is generally the last size the window had when it was floating. It can be unknown if
    /// the window starts out in the tiling layout or fullscreen.
    pub(super) floating_window_size: Option<Size<i32, Logical>>,

    /// The position that the tile should assume when going floating, relative to the floating
    /// space working area.
    ///
    /// This is generally the last position the tile had when it was floating. It can be unknown if
    /// the window starts out in the tiling layout.
    pub(super) floating_pos: Option<Point<f64, SizeFrac>>,

    /// Currently selected preset width index when this tile is floating.
    pub(super) floating_preset_width_idx: Option<usize>,

    /// Currently selected preset height index when this tile is floating.
    pub(super) floating_preset_height_idx: Option<usize>,

    /// The animation upon opening a window.
    open_animation: Option<OpenAnimation>,

    /// The animation of the window resizing.
    resize_animation: Option<ResizeAnimation>,

    /// The animation of a tile visually moving horizontally.
    move_x_animation: Option<MoveAnimation>,

    /// The animation of a tile visually moving vertically.
    move_y_animation: Option<MoveAnimation>,

    /// The animation of the tile's opacity.
    pub(super) alpha_animation: Option<AlphaAnimation>,

    /// Offset during the initial interactive move rubberband.
    pub(super) interactive_move_offset: Point<f64, Logical>,

    /// Snapshot of the last render for use in the close animation.
    unmap_snapshot: Option<TileRenderSnapshot>,

    /// Extra damage for clipped surface corner radius changes.
    rounded_corner_damage: RoundedCornerDamage,

    /// The view size for the tile's workspace.
    ///
    /// Used as the fullscreen target size.
    view_size: Size<f64, Logical>,

    /// Scale of the output the tile is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    pub(super) clock: Clock,

    /// Configurable properties of the layout.
    pub(super) options: Rc<Options>,
}

niri_render_elements! {
    TileRenderElement<R> => {
        LayoutElement = LayoutElementRenderElement<R>,
        FocusRing = FocusRingRenderElement,
        SolidColor = SolidColorRenderElement,
        Opening = OpeningWindowRenderElement,
        Resize = ResizeRenderElement,
        Border = BorderRenderElement,
        Shadow = ShadowRenderElement,
        ClippedSurface = ClippedSurfaceRenderElement<R>,
        Offscreen = OffscreenRenderElement,
        ExtraDamage = ExtraDamage,
        BackgroundEffect = BackgroundEffectElement,
    }
}

pub type TileRenderSnapshot =
    RenderSnapshot<TileRenderElement<GlesRenderer>, TileRenderElement<GlesRenderer>>;

#[derive(Debug)]
struct ResizeAnimation {
    anim: Animation,
    size_from: Size<f64, Logical>,
    snapshot: LayoutElementRenderSnapshot,
    offscreen: OffscreenBuffer,
    tile_size_from: Size<f64, Logical>,
    // If the resize involved the fullscreen state at some point, this is the progress toward the
    // fullscreen state. Used for things like fullscreen backdrop alpha.
    //
    // Note that this can be set even if this specific resize is between two non-fullscreen states,
    // for example when issuing a new resize during an unfullscreen resize.
    fullscreen_progress: Option<Animation>,
    // Similar to above but for fullscreen-or-maximized.
    expanded_progress: Option<Animation>,
}

#[derive(Debug)]
struct MoveAnimation {
    anim: Animation,
    from: f64,
}

#[derive(Debug)]
pub(super) struct AlphaAnimation {
    pub(super) anim: Animation,
    /// Whether the animation should persist after it's done.
    ///
    /// This is used by things like interactive move which need to animate alpha to
    /// semitransparent, then hold it at semitransparent for a while, until the operation
    /// completes.
    pub(super) hold_after_done: bool,
    offscreen: OffscreenBuffer,
}

impl<W: LayoutElement> Tile<W> {
    pub fn new(
        window: W,
        view_size: Size<f64, Logical>,
        scale: f64,
        clock: Clock,
        options: Rc<Options>,
    ) -> Self {
        let rules = window.rules();
        let border_config = options.layout.border.merged_with(&rules.border);
        let focus_ring_config = options.layout.focus_ring.merged_with(&rules.focus_ring);
        let shadow_config = options.layout.shadow.merged_with(&rules.shadow);
        let sizing_mode = window.sizing_mode();

        Self {
            window,
            border: FocusRing::new(border_config.into()),
            focus_ring: FocusRing::new(focus_ring_config),
            shadow: Shadow::new(shadow_config),
            sizing_mode,
            fullscreen_backdrop: SolidColorBuffer::new((0., 0.), [0., 0., 0., 1.]),
            restore_to_floating: false,
            floating_window_size: None,
            floating_pos: None,
            floating_preset_width_idx: None,
            floating_preset_height_idx: None,
            open_animation: None,
            resize_animation: None,
            move_x_animation: None,
            move_y_animation: None,
            alpha_animation: None,
            interactive_move_offset: Point::from((0., 0.)),
            unmap_snapshot: None,
            rounded_corner_damage: Default::default(),
            view_size,
            scale,
            clock,
            options,
        }
    }

    pub fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) {
        // If preset widths or heights changed, clear our stored preset index.
        if self.options.layout.preset_column_widths != options.layout.preset_column_widths {
            self.floating_preset_width_idx = None;
        }
        if self.options.layout.preset_window_heights != options.layout.preset_window_heights {
            self.floating_preset_height_idx = None;
        }

        self.view_size = view_size;
        self.scale = scale;
        self.options = options;

        let round_max1 = |logical| round_logical_in_physical_max1(self.scale, logical);

        let rules = self.window.rules();

        let mut border_config = self.options.layout.border.merged_with(&rules.border);
        border_config.width = round_max1(border_config.width);
        self.border.update_config(border_config.into());

        let mut focus_ring_config = self
            .options
            .layout
            .focus_ring
            .merged_with(&rules.focus_ring);
        focus_ring_config.width = round_max1(focus_ring_config.width);
        self.focus_ring.update_config(focus_ring_config);

        let shadow_config = self.options.layout.shadow.merged_with(&rules.shadow);
        self.shadow.update_config(shadow_config);

        self.window.update_config(self.options.blur);
    }

    pub fn update_shaders(&mut self) {
        self.border.update_shaders();
        self.focus_ring.update_shaders();
        self.shadow.update_shaders();
    }

    pub fn update_window(&mut self) {
        let prev_sizing_mode = self.sizing_mode;
        self.sizing_mode = self.window.sizing_mode();

        if let Some(animate_from) = self.window.take_animation_snapshot() {
            let params = if let Some(resize) = self.resize_animation.take() {
                // Compute like in animated_window_size(), but using the snapshot geometry (since
                // the current one is already overwritten).
                let mut size = animate_from.size;

                let val = resize.anim.value();
                let size_from = resize.size_from;
                let tile_size_from = resize.tile_size_from;

                size.w = size_from.w + (size.w - size_from.w) * val;
                size.h = size_from.h + (size.h - size_from.h) * val;

                let mut tile_size = animate_from.size;
                if prev_sizing_mode.is_fullscreen() {
                    tile_size.w = f64::max(tile_size.w, self.view_size.w);
                    tile_size.h = f64::max(tile_size.h, self.view_size.h);
                } else if prev_sizing_mode.is_normal() && !self.border.is_off() {
                    let width = self.border.width();
                    tile_size.w += width * 2.;
                    tile_size.h += width * 2.;
                }

                tile_size.w = tile_size_from.w + (tile_size.w - tile_size_from.w) * val;
                tile_size.h = tile_size_from.h + (tile_size.h - tile_size_from.h) * val;

                let fullscreen_from = resize
                    .fullscreen_progress
                    .map(|anim| anim.clamped_value().clamp(0., 1.))
                    .unwrap_or(if prev_sizing_mode.is_fullscreen() {
                        1.
                    } else {
                        0.
                    });

                let expanded_from = resize
                    .expanded_progress
                    .map(|anim| anim.clamped_value().clamp(0., 1.))
                    .unwrap_or(if prev_sizing_mode.is_normal() { 0. } else { 1. });

                // Also try to reuse the existing offscreen buffer if we have one.
                (
                    size,
                    tile_size,
                    fullscreen_from,
                    expanded_from,
                    resize.offscreen,
                )
            } else {
                let size = animate_from.size;

                // Compute like in tile_size().
                let mut tile_size = size;
                if prev_sizing_mode.is_fullscreen() {
                    tile_size.w = f64::max(tile_size.w, self.view_size.w);
                    tile_size.h = f64::max(tile_size.h, self.view_size.h);
                } else if prev_sizing_mode.is_normal() && !self.border.is_off() {
                    let width = self.border.width();
                    tile_size.w += width * 2.;
                    tile_size.h += width * 2.;
                }

                let fullscreen_from = if prev_sizing_mode.is_fullscreen() {
                    1.
                } else {
                    0.
                };

                let expanded_from = if prev_sizing_mode.is_normal() { 0. } else { 1. };

                (
                    size,
                    tile_size,
                    fullscreen_from,
                    expanded_from,
                    OffscreenBuffer::default(),
                )
            };
            let (size_from, tile_size_from, fullscreen_from, expanded_from, offscreen) = params;

            let change = self.window.size().to_f64().to_point() - size_from.to_point();
            let change = f64::max(change.x.abs(), change.y.abs());
            let tile_change = self.tile_size().to_f64().to_point() - tile_size_from.to_point();
            let tile_change = f64::max(tile_change.x.abs(), tile_change.y.abs());
            let change = f64::max(change, tile_change);
            if change > RESIZE_ANIMATION_THRESHOLD {
                let anim = Animation::new(
                    self.clock.clone(),
                    0.,
                    1.,
                    0.,
                    self.options.animations.window_resize.anim,
                );

                let fullscreen_to = if self.sizing_mode.is_fullscreen() {
                    1.
                } else {
                    0.
                };
                let expanded_to = if self.sizing_mode.is_normal() { 0. } else { 1. };
                let fullscreen_progress = (fullscreen_from != fullscreen_to)
                    .then(|| anim.restarted(fullscreen_from, fullscreen_to, 0.));
                let expanded_progress = (expanded_from != expanded_to)
                    .then(|| anim.restarted(expanded_from, expanded_to, 0.));

                self.resize_animation = Some(ResizeAnimation {
                    anim,
                    size_from,
                    snapshot: animate_from,
                    offscreen,
                    tile_size_from,
                    fullscreen_progress,
                    expanded_progress,
                });
            } else {
                self.resize_animation = None;
            }
        }

        let round_max1 = |logical| round_logical_in_physical_max1(self.scale, logical);

        let rules = self.window.rules();
        let mut border_config = self.options.layout.border.merged_with(&rules.border);
        border_config.width = round_max1(border_config.width);
        self.border.update_config(border_config.into());

        let mut focus_ring_config = self
            .options
            .layout
            .focus_ring
            .merged_with(&rules.focus_ring);
        focus_ring_config.width = round_max1(focus_ring_config.width);
        self.focus_ring.update_config(focus_ring_config);

        let shadow_config = self.options.layout.shadow.merged_with(&rules.shadow);
        self.shadow.update_config(shadow_config);

        let window_size = self.window_size();
        let radius = self
            .window
            .geometry_corner_radius()
            .fit_to(window_size.w as f32, window_size.h as f32);
        self.rounded_corner_damage.set_corner_radius(radius);
    }

    pub fn advance_animations(&mut self) {
        if let Some(open) = &mut self.open_animation {
            if open.is_done() {
                self.open_animation = None;
            }
        }

        if let Some(resize) = &mut self.resize_animation {
            if resize.anim.is_done() {
                self.resize_animation = None;
            }
        }

        if let Some(move_) = &mut self.move_x_animation {
            if move_.anim.is_done() {
                self.move_x_animation = None;
            }
        }
        if let Some(move_) = &mut self.move_y_animation {
            if move_.anim.is_done() {
                self.move_y_animation = None;
            }
        }

        if let Some(alpha) = &mut self.alpha_animation {
            if !alpha.hold_after_done && alpha.anim.is_done() {
                self.alpha_animation = None;
            }
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.are_transitions_ongoing() || self.window.rules().baba_is_float == Some(true)
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        self.open_animation.is_some()
            || self.resize_animation.is_some()
            || self.move_x_animation.is_some()
            || self.move_y_animation.is_some()
            || self
                .alpha_animation
                .as_ref()
                .is_some_and(|alpha| !alpha.anim.is_done())
    }

    pub fn update_render_elements(&mut self, is_active: bool, view_rect: Rectangle<f64, Logical>) {
        let rules = self.window.rules();
        let animated_tile_size = self.animated_tile_size();
        let expanded_progress = self.expanded_progress();

        let draw_border_with_background = rules
            .draw_border_with_background
            .unwrap_or_else(|| !self.window.has_ssd());
        let border_width = self.visual_border_width().unwrap_or(0.);

        // Do the inverse of tile_size() in order to handle the unfullscreen animation for windows
        // that were smaller than the fullscreen size, and therefore their animated_window_size() is
        // currently much smaller than the tile size.
        let mut border_window_size = animated_tile_size;
        border_window_size.w -= border_width * 2.;
        border_window_size.h -= border_width * 2.;

        // FIXME: this takes into account the animation from normal sizing mode to
        // maximized/fullscreen, but it doesn't take into account the corner radius animation from
        // the window itself.
        //
        // Currently, an easy way to see the problem is to start from a window with a nonzero
        // radius, then go from windowed fullscreen (that forces 0 radius) to regular fullscreen.
        // At the start of the animation, windowed fullscreen becomes false, but the window hasn't
        // animated to the normal fullscreen yet, so the radius here jumps to its nonzero value,
        // even though it should remain zero throughout.
        //
        // Later, when windows get the surface shape protocol with radii, this issue will happen
        // when that changes between animated commits.
        let radius = self
            .window
            .geometry_corner_radius()
            .expanded_by(border_width as f32)
            .scaled_by(1. - expanded_progress as f32);
        self.border.update_render_elements(
            border_window_size,
            is_active,
            !draw_border_with_background,
            self.window.is_urgent(),
            Rectangle::new(
                view_rect.loc - Point::from((border_width, border_width)),
                view_rect.size,
            ),
            radius,
            self.scale,
            1. - expanded_progress as f32,
        );

        let radius = if self.visual_border_width().is_some() {
            radius
        } else {
            self.window
                .geometry_corner_radius()
                .scaled_by(1. - expanded_progress as f32)
        };
        self.shadow.update_render_elements(
            animated_tile_size,
            is_active,
            radius,
            self.scale,
            1. - expanded_progress as f32,
        );

        let draw_focus_ring_with_background = if self.border.is_off() {
            draw_border_with_background
        } else {
            false
        };
        let radius = radius.expanded_by(self.focus_ring.width() as f32);
        self.focus_ring.update_render_elements(
            animated_tile_size,
            is_active,
            !draw_focus_ring_with_background,
            self.window.is_urgent(),
            view_rect,
            radius,
            self.scale,
            1. - expanded_progress as f32,
        );

        self.fullscreen_backdrop.resize(animated_tile_size);
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn render_offset(&self) -> Point<f64, Logical> {
        let mut offset = Point::from((0., 0.));

        if let Some(move_) = &self.move_x_animation {
            offset.x += move_.from * move_.anim.value();
        }
        if let Some(move_) = &self.move_y_animation {
            offset.y += move_.from * move_.anim.value();
        }

        offset += self.interactive_move_offset;

        offset
    }

    pub fn start_open_animation(&mut self) {
        self.open_animation = Some(OpenAnimation::new(Animation::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            self.options.animations.window_open.anim,
        )));
    }

    pub fn resize_animation(&self) -> Option<&Animation> {
        self.resize_animation.as_ref().map(|resize| &resize.anim)
    }

    pub fn animate_move_from(&mut self, from: Point<f64, Logical>) {
        self.animate_move_x_from(from.x);
        self.animate_move_y_from(from.y);
    }

    pub fn animate_move_x_from(&mut self, from: f64) {
        self.animate_move_x_from_with_config(from, self.options.animations.window_movement.0);
    }

    pub fn animate_move_x_from_with_config(&mut self, from: f64, config: niri_config::Animation) {
        let current_offset = self.render_offset().x;

        // Preserve the previous config if ongoing.
        let anim = self.move_x_animation.take().map(|move_| move_.anim);
        let anim = anim
            .map(|anim| anim.restarted(1., 0., 0.))
            .unwrap_or_else(|| Animation::new(self.clock.clone(), 1., 0., 0., config));

        self.move_x_animation = Some(MoveAnimation {
            anim,
            from: from + current_offset,
        });
    }

    pub fn animate_move_y_from(&mut self, from: f64) {
        self.animate_move_y_from_with_config(from, self.options.animations.window_movement.0);
    }

    pub fn animate_move_y_from_with_config(&mut self, from: f64, config: niri_config::Animation) {
        let current_offset = self.render_offset().y;

        // Preserve the previous config if ongoing.
        let anim = self.move_y_animation.take().map(|move_| move_.anim);
        let anim = anim
            .map(|anim| anim.restarted(1., 0., 0.))
            .unwrap_or_else(|| Animation::new(self.clock.clone(), 1., 0., 0., config));

        self.move_y_animation = Some(MoveAnimation {
            anim,
            from: from + current_offset,
        });
    }

    pub fn offset_move_y_anim_current(&mut self, offset: f64) {
        if let Some(move_) = self.move_y_animation.as_mut() {
            // If the anim is almost done, there's little point trying to offset it; we can let
            // things jump. If it turns out like a bad idea, we could restart the anim instead.
            let value = move_.anim.value();
            if value > 0.001 {
                move_.from += offset / value;
            }
        }
    }

    pub fn stop_move_animations(&mut self) {
        self.move_x_animation = None;
        self.move_y_animation = None;
    }

    pub fn animate_alpha(&mut self, from: f64, to: f64, config: niri_config::Animation) {
        let from = from.clamp(0., 1.);
        let to = to.clamp(0., 1.);

        let (current, offscreen) = if let Some(alpha) = self.alpha_animation.take() {
            (alpha.anim.clamped_value(), alpha.offscreen)
        } else {
            (from, OffscreenBuffer::default())
        };

        self.alpha_animation = Some(AlphaAnimation {
            anim: Animation::new(self.clock.clone(), current, to, 0., config),
            hold_after_done: false,
            offscreen,
        });
    }

    pub fn ensure_alpha_animates_to_1(&mut self) {
        if let Some(alpha) = &self.alpha_animation {
            if alpha.anim.to() != 1. {
                // Cancel animation instead of starting a new one because the user likely wants to
                // see the tile right away.
                self.alpha_animation = None;
            }
        }
    }

    pub fn hold_alpha_animation_after_done(&mut self) {
        if let Some(alpha) = &mut self.alpha_animation {
            alpha.hold_after_done = true;
        }
    }

    pub fn window(&self) -> &W {
        &self.window
    }

    pub fn window_mut(&mut self) -> &mut W {
        &mut self.window
    }

    pub fn sizing_mode(&self) -> SizingMode {
        self.sizing_mode
    }

    fn fullscreen_progress(&self) -> f64 {
        if let Some(resize) = &self.resize_animation {
            if let Some(anim) = &resize.fullscreen_progress {
                return anim.clamped_value().clamp(0., 1.);
            }
        }

        if self.sizing_mode.is_fullscreen() {
            1.
        } else {
            0.
        }
    }

    fn expanded_progress(&self) -> f64 {
        if let Some(resize) = &self.resize_animation {
            if let Some(anim) = &resize.expanded_progress {
                return anim.clamped_value().clamp(0., 1.);
            }
        }

        if self.sizing_mode.is_normal() {
            0.
        } else {
            1.
        }
    }

    /// Returns `None` if the border is hidden and `Some(width)` if it should be shown.
    pub fn effective_border_width(&self) -> Option<f64> {
        if !self.sizing_mode.is_normal() {
            return None;
        }

        if self.border.is_off() {
            return None;
        }

        Some(self.border.width())
    }

    fn visual_border_width(&self) -> Option<f64> {
        if self.border.is_off() {
            return None;
        }

        let expanded_progress = self.expanded_progress();

        // Only hide the border when fully expanded to avoid jarring border appearance.
        if expanded_progress == 1. {
            return None;
        }

        // FIXME: would be cool to, like, gradually resize the border from full width to 0 during
        // fullscreening, but the rest of the code isn't quite ready for that yet. It needs to
        // handle things like computing intermediate tile size when an animated resize starts during
        // an animated unfullscreen resize.
        Some(self.border.width())
    }

    /// Returns the location of the window's visual geometry within this Tile.
    pub fn window_loc(&self) -> Point<f64, Logical> {
        let mut loc = Point::from((0., 0.));

        let window_size = self.animated_window_size();
        let target_size = self.animated_tile_size();

        // Center the window within its tile.
        //
        // - Without borders, the sizes match, so this difference is zero.
        // - Borders always match from all sides, so this difference is pre-rounded to physical.
        // - In fullscreen, if the window is smaller than the tile, then it gets centered, otherwise
        //   the tile size matches the window.
        // - During animations, the window remains centered within the tile; this is important for
        //   the to/from fullscreen animation.
        loc.x += (target_size.w - window_size.w) / 2.;
        loc.y += (target_size.h - window_size.h) / 2.;

        // Round to physical pixels.
        loc = loc
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);

        loc
    }

    pub fn tile_size(&self) -> Size<f64, Logical> {
        let mut size = self.window_size();

        if self.sizing_mode.is_fullscreen() {
            // Normally we'd just return the fullscreen size here, but this makes things a bit
            // nicer if a fullscreen window is bigger than the fullscreen size for some reason.
            size.w = f64::max(size.w, self.view_size.w);
            size.h = f64::max(size.h, self.view_size.h);
            return size;
        }

        if let Some(width) = self.effective_border_width() {
            size.w += width * 2.;
            size.h += width * 2.;
        }

        size
    }

    pub fn tile_expected_or_current_size(&self) -> Size<f64, Logical> {
        let mut size = self.window_expected_or_current_size();

        if self.sizing_mode.is_fullscreen() {
            // Normally we'd just return the fullscreen size here, but this makes things a bit
            // nicer if a fullscreen window is bigger than the fullscreen size for some reason.
            size.w = f64::max(size.w, self.view_size.w);
            size.h = f64::max(size.h, self.view_size.h);
            return size;
        }

        if let Some(width) = self.effective_border_width() {
            size.w += width * 2.;
            size.h += width * 2.;
        }

        size
    }

    pub fn window_size(&self) -> Size<f64, Logical> {
        let mut size = self.window.size().to_f64();
        size = size
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);
        size
    }

    pub fn window_expected_or_current_size(&self) -> Size<f64, Logical> {
        let size = self.window.expected_size();
        let mut size = size.unwrap_or_else(|| self.window.size()).to_f64();
        size = size
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);
        size
    }

    pub fn animated_window_size(&self) -> Size<f64, Logical> {
        let mut size = self.window_size();

        if let Some(resize) = &self.resize_animation {
            let val = resize.anim.value();
            let size_from = resize.size_from.to_f64();

            size.w = f64::max(1., size_from.w + (size.w - size_from.w) * val);
            size.h = f64::max(1., size_from.h + (size.h - size_from.h) * val);
            size = size
                .to_physical_precise_round(self.scale)
                .to_logical(self.scale);
        }

        size
    }

    pub fn animated_tile_size(&self) -> Size<f64, Logical> {
        let mut size = self.tile_size();

        if let Some(resize) = &self.resize_animation {
            let val = resize.anim.value();
            let size_from = resize.tile_size_from.to_f64();

            size.w = f64::max(1., size_from.w + (size.w - size_from.w) * val);
            size.h = f64::max(1., size_from.h + (size.h - size_from.h) * val);
            size = size
                .to_physical_precise_round(self.scale)
                .to_logical(self.scale);
        }

        size
    }

    pub fn buf_loc(&self) -> Point<f64, Logical> {
        let mut loc = Point::from((0., 0.));
        loc += self.window_loc();
        loc += self.window.buf_loc().to_f64();
        loc
    }

    /// Returns a partially-filled [`WindowLayout`].
    ///
    /// Only the sizing properties that a [`Tile`] can fill are filled.
    pub fn ipc_layout_template(&self) -> WindowLayout {
        WindowLayout {
            pos_in_scrolling_layout: None,
            tile_size: self.tile_size().into(),
            window_size: self.window().size().into(),
            tile_pos_in_workspace_view: None,
            window_offset_in_tile: self.window_loc().into(),
        }
    }

    fn is_in_input_region(&self, mut point: Point<f64, Logical>) -> bool {
        point -= self.window_loc().to_f64();
        self.window.is_in_input_region(point)
    }

    fn is_in_activation_region(&self, point: Point<f64, Logical>) -> bool {
        let activation_region = Rectangle::from_size(self.tile_size());
        activation_region.contains(point)
    }

    pub fn hit(&self, point: Point<f64, Logical>) -> Option<HitType> {
        let offset = self.bob_offset();
        let point = point - offset;

        if self.is_in_input_region(point) {
            let win_pos = self.buf_loc() + offset;
            Some(HitType::Input { win_pos })
        } else if self.is_in_activation_region(point) {
            Some(HitType::Activate {
                is_tab_indicator: false,
            })
        } else {
            None
        }
    }

    pub fn request_tile_size(
        &mut self,
        mut size: Size<f64, Logical>,
        animate: bool,
        transaction: Option<Transaction>,
    ) {
        // Can't go through effective_border_width() because we might be fullscreen.
        if !self.border.is_off() {
            let width = self.border.width();
            size.w = f64::max(1., size.w - width * 2.);
            size.h = f64::max(1., size.h - width * 2.);
        }

        // The size request has to be i32 unfortunately, due to Wayland. We floor here instead of
        // round to avoid situations where proportionally-sized columns don't fit on the screen
        // exactly.
        self.window.request_size(
            size.to_i32_floor(),
            SizingMode::Normal,
            animate,
            transaction,
        );
    }

    pub fn tile_width_for_window_width(&self, size: f64) -> f64 {
        if self.border.is_off() {
            size
        } else {
            size + self.border.width() * 2.
        }
    }

    pub fn tile_height_for_window_height(&self, size: f64) -> f64 {
        if self.border.is_off() {
            size
        } else {
            size + self.border.width() * 2.
        }
    }

    pub fn window_width_for_tile_width(&self, size: f64) -> f64 {
        if self.border.is_off() {
            size
        } else {
            size - self.border.width() * 2.
        }
    }

    pub fn window_height_for_tile_height(&self, size: f64) -> f64 {
        if self.border.is_off() {
            size
        } else {
            size - self.border.width() * 2.
        }
    }

    pub fn request_maximized(
        &mut self,
        size: Size<f64, Logical>,
        animate: bool,
        transaction: Option<Transaction>,
    ) {
        self.window.request_size(
            size.to_i32_round(),
            SizingMode::Maximized,
            animate,
            transaction,
        );
    }

    pub fn request_fullscreen(&mut self, animate: bool, transaction: Option<Transaction>) {
        self.window.request_size(
            self.view_size.to_i32_round(),
            SizingMode::Fullscreen,
            animate,
            transaction,
        );
    }

    pub fn min_size_nonfullscreen(&self) -> Size<f64, Logical> {
        let mut size = self.window.min_size().to_f64();

        // Can't go through effective_border_width() because we might be fullscreen.
        if !self.border.is_off() {
            let width = self.border.width();

            size.w = f64::max(1., size.w);
            size.h = f64::max(1., size.h);

            size.w += width * 2.;
            size.h += width * 2.;
        }

        size
    }

    pub fn max_size_nonfullscreen(&self) -> Size<f64, Logical> {
        let mut size = self.window.max_size().to_f64();

        // Can't go through effective_border_width() because we might be fullscreen.
        if !self.border.is_off() {
            let width = self.border.width();

            if size.w > 0. {
                size.w += width * 2.;
            }
            if size.h > 0. {
                size.h += width * 2.;
            }
        }

        size
    }

    pub fn bob_offset(&self) -> Point<f64, Logical> {
        if self.window.rules().baba_is_float != Some(true) {
            return Point::from((0., 0.));
        }

        let y = baba_is_float_offset(self.clock.now(), self.view_size.h);
        let y = round_logical_in_physical(self.scale, y);
        Point::from((0., y))
    }

    fn render_inner<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        location: Point<f64, Logical>,
        mut xray_pos: XrayPos,
        focus_ring: bool,
        push: &mut dyn FnMut(TileRenderElement<R>),
    ) {
        let _span = tracy_client::span!("Tile::render_inner");

        let scale = Scale::from(self.scale);
        let fullscreen_progress = self.fullscreen_progress();
        let expanded_progress = self.expanded_progress();

        let win_alpha = if self.window.is_ignoring_opacity_window_rule() {
            1.
        } else {
            let alpha = self.window.rules().opacity.unwrap_or(1.).clamp(0., 1.);

            // Interpolate towards alpha = 1. at fullscreen.
            let p = fullscreen_progress as f32;
            alpha * (1. - p) + 1. * p
        };

        // This is here rather than in render_offset() because render_offset() is currently assumed
        // by the code to be temporary. So, for example, interactive move will try to "grab" the
        // tile at its current render offset and reset the render offset to zero by cancelling the
        // tile move animations. On the other hand, bob_offset() is not resettable, so adding it in
        // render_offset() would cause obvious animation glitches.
        //
        // This isn't to say that adding it here is perfect; indeed, it kind of breaks view_rect
        // passed to update_render_elements(). But, it works well enough for what it is.
        let bob_offset = self.bob_offset();
        let location = location + bob_offset;
        xray_pos = xray_pos.offset(bob_offset);

        let window_loc = self.window_loc();
        let window_size = self.window_size();
        let animated_window_size = self.animated_window_size();
        let window_render_loc = location + window_loc;
        let area = Rectangle::new(window_render_loc, animated_window_size);
        xray_pos = xray_pos.offset(window_loc);

        let rules = self.window.rules();

        // Clip to geometry including during the fullscreen animation to help with buggy clients
        // that submit a full-sized buffer before acking the fullscreen state (Firefox).
        let clip_to_geometry = fullscreen_progress < 1. && rules.clip_to_geometry == Some(true);
        let radius = self
            .window
            .geometry_corner_radius()
            .scaled_by(1. - expanded_progress as f32);

        // Popups go on top, whether it's resize or not.
        self.window.render_popups(
            ctx.r(),
            window_render_loc,
            scale,
            win_alpha,
            xray_pos,
            &mut |elem| push(elem.into()),
        );

        // If we're resizing, try to render a shader, or a fallback.
        let mut pushed_resize = false;
        if let Some(resize) = &self.resize_animation {
            if ResizeRenderElement::has_shader(ctx.renderer) {
                let mut ctx = ctx.as_gles();

                if let Some(texture_from) = resize.snapshot.texture(ctx.r(), scale) {
                    let mut window_elements = Vec::new();
                    self.window.render_normal(
                        ctx.r(),
                        Point::from((0., 0.)),
                        scale,
                        1.,
                        &mut |elem| window_elements.push(elem),
                    );

                    let current = resize
                        .offscreen
                        .render(ctx.renderer, scale, &window_elements)
                        .map_err(|err| warn!("error rendering window to texture: {err:?}"))
                        .ok();

                    // Clip blocked-out resizes unconditionally because they use solid color render
                    // elements.
                    let clip_to_geometry =
                        if ctx.target.should_block_out(resize.snapshot.block_out_from)
                            && ctx.target.should_block_out(rules.block_out_from)
                        {
                            true
                        } else {
                            clip_to_geometry
                        };

                    if let Some((elem_current, _sync_point, mut data)) = current {
                        let texture_current = elem_current.texture().clone();
                        // The offset and size are computed in physical pixels and converted to
                        // logical with the same `scale`, so converting them back with rounding
                        // inside the geometry() call gives us the same physical result back.
                        let texture_current_geo = elem_current.geometry(scale);

                        let elem = ResizeRenderElement::new(
                            area,
                            scale,
                            texture_from.clone(),
                            resize.snapshot.size,
                            (texture_current, texture_current_geo),
                            window_size,
                            resize.anim.value() as f32,
                            resize.anim.clamped_value().clamp(0., 1.) as f32,
                            radius,
                            clip_to_geometry,
                            win_alpha,
                        );

                        // We're drawing the resize shader, not the offscreen directly.
                        data.id = elem.id().clone();

                        // This is not a problem for split popups as the code will look for them by
                        // original id when it doesn't find them on the offscreen.
                        self.window.set_offscreen_data(Some(data));
                        push(elem.into());
                        pushed_resize = true;
                    }
                }
            }

            if !pushed_resize {
                let fallback_buffer = SolidColorBuffer::new(area.size, [1., 0., 0., 1.]);
                let elem = SolidColorRenderElement::from_buffer(
                    &fallback_buffer,
                    area.loc,
                    win_alpha,
                    Kind::Unspecified,
                );
                push(elem.into());
                pushed_resize = true;
            }
        }

        // If we're not resizing, render the window itself.
        let has_border_shader = BorderRenderElement::has_shader(ctx.renderer);
        if !pushed_resize {
            let geo = Rectangle::new(window_render_loc, window_size);
            let radius = radius.fit_to(window_size.w as f32, window_size.h as f32);

            let clip_shader = ClippedSurfaceRenderElement::shader(ctx.renderer).cloned();
            let clip = |elem| match elem {
                LayoutElementRenderElement::Wayland(elem) => {
                    // If we should clip to geometry, render a clipped window.
                    if clip_to_geometry {
                        if let Some(shader) = clip_shader.clone() {
                            if ClippedSurfaceRenderElement::will_clip(&elem, scale, geo, radius) {
                                return ClippedSurfaceRenderElement::new(
                                    elem,
                                    scale,
                                    geo,
                                    shader.clone(),
                                    radius,
                                )
                                .into();
                            }
                        }
                    }

                    // Otherwise, render it normally.
                    LayoutElementRenderElement::Wayland(elem).into()
                }
                LayoutElementRenderElement::SolidColor(elem) => {
                    // In this branch we're rendering a blocked-out window with a solid
                    // color. We need to render it with a rounded corner shader even if
                    // clip_to_geometry is false, because in this case we're assuming that
                    // the unclipped window CSD already has corners rounded to the
                    // user-provided radius, so our blocked-out rendering should match that
                    // radius.
                    if radius != CornerRadius::default() && has_border_shader {
                        return BorderRenderElement::new(
                            geo.size,
                            Rectangle::from_size(geo.size),
                            GradientInterpolation::default(),
                            Color::from_color32f(elem.color()),
                            Color::from_color32f(elem.color()),
                            0.,
                            Rectangle::from_size(geo.size),
                            0.,
                            radius,
                            scale.x as f32,
                            1.,
                        )
                        .with_location(geo.loc)
                        .into();
                    }

                    // Otherwise, render the solid color as is.
                    LayoutElementRenderElement::SolidColor(elem).into()
                }
                elem @ LayoutElementRenderElement::BackgroundEffect(_) => {
                    // This is only used on popups for now. If subsurface blur is implemented, this
                    // will need to be handled somehow.
                    error!("background effect clipping is unimplemented");
                    elem.into()
                }
            };

            if clip_to_geometry && clip_shader.is_some() {
                let damage = self.rounded_corner_damage.render(geo);
                push(damage.into());
            }

            self.window
                .render_normal(ctx.r(), window_render_loc, scale, win_alpha, &mut |elem| {
                    push(clip(elem))
                });
        }

        if fullscreen_progress > 0. {
            let alpha = fullscreen_progress as f32;

            // Opt-in: when set, clip the backdrop to tile-minus-window so translucent fullscreen
            // windows (e.g. kitty with background_opacity<1.0) compose against the wallpaper
            // instead of the opaque backdrop. Off by default to match xdg-shell's requirement
            // that the compositor hide other screen content behind a non-opaque fullscreen
            // surface.
            let clip_backdrop = rules.clip_fullscreen_backdrop_to_window == Some(true);

            // During the un/fullscreen animation, render a border element in order to use the
            // animated corner radius.
            if fullscreen_progress < 1. && has_border_shader {
                let border_width = self.visual_border_width().unwrap_or(0.);
                let radius = self
                    .window
                    .geometry_corner_radius()
                    .expanded_by(border_width as f32)
                    .scaled_by(1. - expanded_progress as f32);

                let color = self.fullscreen_backdrop.color();

                if clip_backdrop {
                    // Per-strip CornerRadius preserves the animated tile-outer-corner shrink:
                    // strips at tile corners get the radius, strips that butt against the
                    // window's edge stay sharp.
                    let tile_rect = Rectangle::new(location, self.fullscreen_backdrop.size());
                    for (geo, per_rect_radius) in backdrop_clip_rects(tile_rect, area, radius) {
                        let elem = BorderRenderElement::new(
                            geo.size,
                            Rectangle::from_size(geo.size),
                            GradientInterpolation::default(),
                            Color::from_color32f(color),
                            Color::from_color32f(color),
                            0.,
                            Rectangle::from_size(geo.size),
                            0.,
                            per_rect_radius,
                            scale.x as f32,
                            alpha,
                        )
                        .with_location(geo.loc);
                        push(elem.into());
                    }
                } else {
                    let size = self.fullscreen_backdrop.size();
                    let elem = BorderRenderElement::new(
                        size,
                        Rectangle::from_size(size),
                        GradientInterpolation::default(),
                        Color::from_color32f(color),
                        Color::from_color32f(color),
                        0.,
                        Rectangle::from_size(size),
                        0.,
                        radius,
                        scale.x as f32,
                        alpha,
                    )
                    .with_location(location);
                    push(elem.into());
                }
            } else if clip_backdrop {
                let tile_rect = Rectangle::new(location, self.fullscreen_backdrop.size());
                for (geo, _) in backdrop_clip_rects(tile_rect, area, CornerRadius::default()) {
                    let elem = SolidColorRenderElement::from_buffer_at(
                        &self.fullscreen_backdrop,
                        geo,
                        alpha,
                        Kind::Unspecified,
                    );
                    push(elem.into());
                }
            } else {
                let elem = SolidColorRenderElement::from_buffer(
                    &self.fullscreen_backdrop,
                    location,
                    alpha,
                    Kind::Unspecified,
                );
                push(elem.into());
            }
        }

        if let Some(width) = self.visual_border_width() {
            self.border.render(
                ctx.renderer,
                location + Point::from((width, width)),
                &mut |elem| push(elem.into()),
            );
        }

        // Hide the focus ring when maximized/fullscreened. It's not normally visible anyway due to
        // being outside the monitor or obscured by a solid colored bar, but it is visible under
        // semitransparent bars in maximized state (which is a bit weird) and in the overview (also
        // a bit weird).
        if focus_ring && expanded_progress < 1. {
            self.focus_ring
                .render(ctx.renderer, location, &mut |elem| push(elem.into()));
        }

        if expanded_progress < 1. {
            self.shadow
                .render(ctx.renderer, location, &mut |elem| push(elem.into()));
        }

        let surface_anim_scale = animated_window_size / window_size;
        self.window.render_background_effect(
            ctx.as_gles(),
            area,
            self.scale,
            clip_to_geometry,
            surface_anim_scale,
            radius,
            xray_pos,
            &mut |elem| push(elem.into()),
        );
    }

    pub fn render<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        focus_ring: bool,
        push: &mut dyn FnMut(TileRenderElement<R>),
    ) {
        let _span = tracy_client::span!("Tile::render");

        let scale = Scale::from(self.scale);

        let tile_alpha = self
            .alpha_animation
            .as_ref()
            .map_or(1., |alpha| alpha.anim.clamped_value()) as f32;

        let mut pushed = false;
        self.window().set_offscreen_data(None);

        if let Some(open) = &self.open_animation {
            let mut ctx = ctx.as_gles();
            let mut elements = Vec::new();
            self.render_inner(
                ctx.r(),
                Point::new(0., 0.),
                xray_pos,
                focus_ring,
                &mut |elem| elements.push(elem),
            );
            match open.render(
                ctx.renderer,
                &elements,
                self.animated_tile_size(),
                location,
                scale,
                tile_alpha,
            ) {
                Ok((elem, data)) => {
                    self.window().set_offscreen_data(Some(data));
                    push(elem.into());
                    pushed = true;
                }
                Err(err) => {
                    warn!("error rendering window opening animation: {err:?}");
                }
            }
        } else if let Some(alpha) = &self.alpha_animation {
            let mut ctx = ctx.as_gles();
            let mut elements = Vec::new();
            self.render_inner(
                ctx.r(),
                Point::new(0., 0.),
                xray_pos,
                focus_ring,
                &mut |elem| elements.push(elem),
            );
            match alpha.offscreen.render(ctx.renderer, scale, &elements) {
                Ok((elem, _sync, data)) => {
                    let offset = elem.offset();
                    let elem = elem.with_alpha(tile_alpha).with_offset(location + offset);

                    self.window().set_offscreen_data(Some(data));
                    push(elem.into());
                    pushed = true;
                }
                Err(err) => {
                    warn!("error rendering tile to offscreen for alpha animation: {err:?}");
                }
            }
        }

        if !pushed {
            self.render_inner(ctx, location, xray_pos, focus_ring, &mut |elem| push(elem));
        }
    }

    pub fn store_unmap_snapshot_if_empty(
        &mut self,
        renderer: &mut GlesRenderer,
        xray: Option<&mut Xray>,
        xray_has_blocked_out_layers: bool,
        xray_pos: XrayPos,
    ) {
        if self.unmap_snapshot.is_some() {
            return;
        }

        self.unmap_snapshot =
            Some(self.render_snapshot(renderer, xray, xray_has_blocked_out_layers, xray_pos));
    }

    fn render_snapshot(
        &self,
        renderer: &mut GlesRenderer,
        mut xray: Option<&mut Xray>,
        xray_has_blocked_out_layers: bool,
        xray_pos: XrayPos,
    ) -> TileRenderSnapshot {
        let _span = tracy_client::span!("Tile::render_snapshot");

        let mut contents = Vec::new();
        self.render(
            RenderCtx {
                target: RenderTarget::Output,
                renderer,
                xray: xray.as_deref(),
            },
            Point::from((0., 0.)),
            xray_pos,
            false,
            &mut |elem| contents.push(elem),
        );

        let mut contents_with_blocked_out_bg = None;

        // Do a bit of pointer surgery on Xray.
        //
        // The idea is to avoid the combinatorial combination of rendering snapshots for target
        // (Output, Screencast) × Xray target (Output, Screencast, ScreenCapture).
        //
        // Our main goals:
        // - Everything must look unblocked for RenderTarget::Output.
        // - If anything is potentially blocked-out, it must not show up on any screen capture.
        //
        // Right above we rendered a fully-unblocked snapshot for the Output, so that's covered.
        //
        // Next, *only if Xray has any blocked-out surfaces* (which is a rare case), we will render
        // a snapshot where the window itself is unblocked, but the Xray background is blocked. To
        // do this, we swap the Output target buffers in Xray with the Screencast target buffers
        // (which were prepared for us higher up the stack).
        //
        // Finally, we render a fully blocked-out snapshot. If Xray has blocked-out surfaces, then
        // Xray's Screencast buffers are already filled-in, but if not, then we swap in the Output
        // buffers, to avoid an extra render. This is safe since we know there are no blocked
        // surfaces there.
        let output_idx = RenderTarget::Output as usize;
        let screencast_idx = RenderTarget::Screencast as usize;
        let mut screencast_background = None;
        let mut screencast_backdrop = None;
        let mut output_background = None;
        let mut output_backdrop = None;
        if let Some(xray) = &mut xray {
            screencast_background = Some(Rc::clone(&xray.background[screencast_idx]));
            screencast_backdrop = Some(Rc::clone(&xray.backdrop[screencast_idx]));
            output_background = Some(Rc::clone(&xray.background[output_idx]));
            output_backdrop = Some(Rc::clone(&xray.backdrop[output_idx]));

            if xray_has_blocked_out_layers {
                xray.background[output_idx] = screencast_background.clone().unwrap();
                xray.backdrop[output_idx] = screencast_backdrop.clone().unwrap();

                let mut contents = Vec::new();
                self.render(
                    RenderCtx {
                        target: RenderTarget::Output,
                        renderer,
                        xray: Some(xray),
                    },
                    Point::from((0., 0.)),
                    xray_pos,
                    false,
                    &mut |elem| contents.push(elem),
                );
                contents_with_blocked_out_bg = Some(contents);
            } else {
                xray.background[screencast_idx] = output_background.clone().unwrap();
                xray.backdrop[screencast_idx] = output_backdrop.clone().unwrap();
            }
        }

        // A bit of a hack to render blocked out as for screencast, but I think it's fine here.
        let mut blocked_out_contents = Vec::new();
        self.render(
            RenderCtx {
                target: RenderTarget::Screencast,
                renderer,
                xray: xray.as_deref(),
            },
            Point::from((0., 0.)),
            xray_pos,
            false,
            &mut |elem| blocked_out_contents.push(elem),
        );

        // Put everything back to normal.
        if let Some(xray) = &mut xray {
            if xray_has_blocked_out_layers {
                xray.background[output_idx] = output_background.take().unwrap();
                xray.backdrop[output_idx] = output_backdrop.take().unwrap();
            } else {
                xray.background[screencast_idx] = screencast_background.take().unwrap();
                xray.backdrop[screencast_idx] = screencast_backdrop.take().unwrap();
            }
        }

        RenderSnapshot {
            contents,
            contents_with_blocked_out_bg,
            blocked_out_contents,
            block_out_from: self.window.rules().block_out_from,
            size: self.animated_tile_size(),
            texture: Default::default(),
            texture_with_blocked_out_bg: Default::default(),
            blocked_out_texture: Default::default(),
        }
    }

    pub fn take_unmap_snapshot(&mut self) -> Option<TileRenderSnapshot> {
        self.unmap_snapshot.take()
    }

    pub fn border(&self) -> &FocusRing {
        &self.border
    }

    pub fn focus_ring(&self) -> &FocusRing {
        &self.focus_ring
    }

    pub fn options(&self) -> &Rc<Options> {
        &self.options
    }

    #[cfg(test)]
    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    #[cfg(test)]
    pub fn verify_invariants(&self) {
        use approx::assert_abs_diff_eq;

        assert_eq!(self.sizing_mode, self.window.sizing_mode());

        let scale = self.scale;
        let size = self.tile_size();
        let rounded = size.to_physical_precise_round(scale).to_logical(scale);
        assert_abs_diff_eq!(size.w, rounded.w, epsilon = 1e-5);
        assert_abs_diff_eq!(size.h, rounded.h, epsilon = 1e-5);
    }
}

#[cfg(test)]
mod tests {
    use niri_config::CornerRadius;
    use smithay::utils::{Logical, Point, Rectangle, Size};

    use super::backdrop_clip_rects;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn zero_radius() -> CornerRadius {
        CornerRadius::default()
    }

    fn uniform_radius(r: f32) -> CornerRadius {
        CornerRadius::from(r)
    }

    #[test]
    fn backdrop_clip_rects_window_equals_tile_returns_empty() {
        let tile = rect(0., 0., 1920., 1080.);
        let window = tile;
        let result = backdrop_clip_rects(tile, window, zero_radius());
        assert!(result.is_empty(), "expected no rects, got {:?}", result);
    }

    #[test]
    fn backdrop_clip_rects_window_centered_returns_4_strips() {
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(480., 270., 960., 540.);
        let result = backdrop_clip_rects(tile, window, zero_radius());

        assert_eq!(result.len(), 4, "expected 4 strips, got {:?}", result);

        // Documented decomposition: left bar, right bar, top middle, bottom middle.
        assert_eq!(result[0].0, rect(0., 0., 480., 1080.), "left bar");
        assert_eq!(result[1].0, rect(1440., 0., 480., 1080.), "right bar");
        assert_eq!(result[2].0, rect(480., 0., 960., 270.), "top middle");
        assert_eq!(result[3].0, rect(480., 810., 960., 270.), "bottom middle");

        // None of the strips intersects the window.
        for (r, _) in &result {
            assert!(
                r.intersection(window).is_none(),
                "strip {:?} intersects window {:?}",
                r,
                window
            );
        }

        // With zero input radius, every strip's CornerRadius is all-zero.
        for (_, cr) in &result {
            assert_eq!(*cr, zero_radius());
        }
    }

    #[test]
    fn backdrop_clip_rects_aspect_ratio_left_right_only() {
        // Window is full-height, narrower-width: aspect-ratio padding bars on left and right only.
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(240., 0., 1440., 1080.);
        let result = backdrop_clip_rects(tile, window, zero_radius());

        assert_eq!(result.len(), 2, "expected 2 strips, got {:?}", result);
        assert_eq!(result[0].0, rect(0., 0., 240., 1080.), "left bar");
        assert_eq!(result[1].0, rect(1680., 0., 240., 1080.), "right bar");
    }

    #[test]
    fn backdrop_clip_rects_window_flush_top_and_bottom_returns_2_strips() {
        // Window full-width, partial-height (vertical letterbox geometry):
        // only top middle and bottom middle strips, no L/R bars.
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(0., 100., 1920., 880.);
        let result = backdrop_clip_rects(tile, window, zero_radius());

        assert_eq!(result.len(), 2, "expected 2 strips, got {:?}", result);
        assert_eq!(result[0].0, rect(0., 0., 1920., 100.), "top middle");
        assert_eq!(result[1].0, rect(0., 980., 1920., 100.), "bottom middle");
    }

    #[test]
    fn backdrop_clip_rects_window_flush_one_edge_returns_3_strips() {
        // Window flush against the left tile edge, partial in the other directions.
        // Expected: right bar + top middle + bottom middle (no left bar).
        // The top/bottom middle strips' LEFT edges coincide with tile-outer-left,
        // so they own the top-left/bottom-left tile corners respectively.
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(0., 100., 1500., 880.);
        let result = backdrop_clip_rects(tile, window, uniform_radius(16.));

        assert_eq!(result.len(), 3, "expected 3 strips, got {:?}", result);

        // Right bar: owns top-right and bottom-right tile corners; left edge butts window.
        assert_eq!(result[0].0, rect(1500., 0., 420., 1080.), "right bar geo");
        assert_eq!(
            result[0].1,
            CornerRadius {
                top_left: 0.,
                top_right: 16.,
                bottom_right: 16.,
                bottom_left: 0.,
            },
            "right bar should round only its outer (right) corners"
        );

        // Top middle: left edge is tile-outer-left (window flush there), so top-left rounded.
        // Right edge butts against window, so top-right is sharp.
        assert_eq!(result[1].0, rect(0., 0., 1500., 100.), "top middle geo");
        assert_eq!(
            result[1].1,
            CornerRadius {
                top_left: 16.,
                top_right: 0.,
                bottom_right: 0.,
                bottom_left: 0.,
            },
            "top middle should round its top-left (= tile top-left)"
        );

        // Bottom middle: symmetric, bottom-left rounded.
        assert_eq!(
            result[2].0,
            rect(0., 980., 1500., 100.),
            "bottom middle geo"
        );
        assert_eq!(
            result[2].1,
            CornerRadius {
                top_left: 0.,
                top_right: 0.,
                bottom_right: 0.,
                bottom_left: 16.,
            },
            "bottom middle should round its bottom-left (= tile bottom-left)"
        );
    }

    #[test]
    fn backdrop_clip_rects_letterbox_top_bottom_own_all_tile_outer_corners() {
        // Window same width as tile, smaller height: only top + bottom middle strips.
        // No L/R bars exist, so the top/bottom strips own all 4 tile-outer corners.
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(0., 100., 1920., 880.);
        let result = backdrop_clip_rects(tile, window, uniform_radius(16.));

        assert_eq!(result.len(), 2, "expected 2 strips, got {:?}", result);

        // Top middle owns top-left AND top-right tile-outer corners (both side bars absent).
        assert_eq!(
            result[0].1,
            CornerRadius {
                top_left: 16.,
                top_right: 16.,
                bottom_right: 0.,
                bottom_left: 0.,
            },
            "top middle should round both top tile-outer corners when L/R bars are absent"
        );

        // Bottom middle owns bottom-left AND bottom-right.
        assert_eq!(
            result[1].1,
            CornerRadius {
                top_left: 0.,
                top_right: 0.,
                bottom_right: 16.,
                bottom_left: 16.,
            },
            "bottom middle should round both bottom tile-outer corners when L/R bars are absent"
        );
    }

    #[test]
    fn backdrop_clip_rects_window_outside_tile_returns_whole_tile() {
        // Window doesn't overlap tile: backdrop is the entire tile with full radius.
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(-500., -500., 100., 100.);
        let result = backdrop_clip_rects(tile, window, uniform_radius(16.));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, tile);
        assert_eq!(result[0].1, uniform_radius(16.));
    }

    #[test]
    fn backdrop_clip_rects_nonzero_radius_only_outer_corners_rounded() {
        let tile = rect(0., 0., 1920., 1080.);
        let window = rect(480., 270., 960., 540.);
        let result = backdrop_clip_rects(tile, window, uniform_radius(16.));

        assert_eq!(result.len(), 4);

        // Left bar owns the tile's top-left and bottom-left corners; right edge butts against the
        // window.
        assert_eq!(
            result[0].1,
            CornerRadius {
                top_left: 16.,
                top_right: 0.,
                bottom_right: 0.,
                bottom_left: 16.,
            },
            "left bar should round only its outer (left) corners"
        );

        // Right bar owns top-right and bottom-right tile corners.
        assert_eq!(
            result[1].1,
            CornerRadius {
                top_left: 0.,
                top_right: 16.,
                bottom_right: 16.,
                bottom_left: 0.,
            },
            "right bar should round only its outer (right) corners"
        );

        // Top middle is sandwiched between the two bars and the window — no tile-outer corners.
        assert_eq!(
            result[2].1,
            zero_radius(),
            "top middle should have no rounded corners"
        );

        // Bottom middle: same reasoning.
        assert_eq!(
            result[3].1,
            zero_radius(),
            "bottom middle should have no rounded corners"
        );
    }
}
