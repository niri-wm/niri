use std::cmp::max;
use std::iter::zip;
use std::rc::Rc;

use niri_config::utils::MergeWith as _;
use niri_config::{PresetSize, RelativeTo};
use niri_ipc::{PositionChange, SizeChange, WindowLayout};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Serial, Size};

use super::closing_window::{ClosingWindow, ClosingWindowRenderElement};
use super::scrolling::ColumnWidth;
use super::tile::{Tile, TileRenderElement, TileRenderSnapshot};
use super::workspace::{InteractiveResize, ResolvedSize};
use super::{
    ConfigureIntent, InteractiveResizeData, LayoutElement, Options, RemovedTile, SizeFrac,
};
use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::RenderTarget;
use crate::utils::transaction::TransactionBlocker;
use crate::utils::{
    center_preferring_top_left_in_area, clamp_preferring_top_left_in_area, ensure_min_max_size,
    ensure_min_max_size_maybe_zero, ResizeEdge,
};
use crate::window::ResolvedWindowRules;

/// By how many logical pixels the directional move commands move floating windows.
pub const DIRECTIONAL_MOVE_PX: f64 = 50.;

/// Space for floating windows.
#[derive(Debug)]
pub struct FloatingSpace<W: LayoutElement> {
    /// Tiles in top-to-bottom order.
    tiles: Vec<Tile<W>>,

    /// Extra per-tile data.
    data: Vec<Data>,

    /// Id of the active window.
    ///
    /// The active window is not necessarily the topmost window. Focus-follows-mouse should
    /// activate a window, but not bring it to the top, because that's very annoying.
    ///
    /// This is always set to `Some()` when `tiles` isn't empty.
    active_window_id: Option<W::Id>,

    /// Ongoing interactive resize.
    interactive_resize: Option<InteractiveResize<W>>,

    /// Windows in the closing animation.
    closing_windows: Vec<ClosingWindow>,

    /// View size for this space.
    view_size: Size<f64, Logical>,

    /// Working area for this space.
    working_area: Rectangle<f64, Logical>,

    /// Scale of the output the space is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// Configurable properties of the layout.
    options: Rc<Options>,
}

niri_render_elements! {
    FloatingSpaceRenderElement<R> => {
        Tile = TileRenderElement<R>,
        ClosingWindow = ClosingWindowRenderElement,
    }
}

/// Extra per-tile data.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Data {
    /// Position relative to the working area.
    pos: Point<f64, SizeFrac>,

    /// Cached position in logical coordinates.
    ///
    /// Not rounded to physical pixels.
    logical_pos: Point<f64, Logical>,

    /// Cached actual size of the tile.
    size: Size<f64, Logical>,

    /// Working area used for conversions.
    working_area: Rectangle<f64, Logical>,
}

impl Data {
    pub fn new<W: LayoutElement>(
        working_area: Rectangle<f64, Logical>,
        tile: &Tile<W>,
        logical_pos: Point<f64, Logical>,
    ) -> Self {
        let mut rv = Self {
            pos: Point::default(),
            logical_pos: Point::default(),
            size: Size::default(),
            working_area,
        };
        rv.update(tile);
        rv.set_logical_pos(logical_pos);
        rv
    }

    pub fn scale_by_working_area(
        area: Rectangle<f64, Logical>,
        pos: Point<f64, SizeFrac>,
    ) -> Point<f64, Logical> {
        let mut logical_pos = Point::from((pos.x, pos.y));
        logical_pos.x *= area.size.w;
        logical_pos.y *= area.size.h;
        logical_pos += area.loc;
        logical_pos
    }

    pub fn logical_to_size_frac_in_working_area(
        area: Rectangle<f64, Logical>,
        logical_pos: Point<f64, Logical>,
    ) -> Point<f64, SizeFrac> {
        let pos = logical_pos - area.loc;
        let mut pos = Point::from((pos.x, pos.y));
        pos.x /= f64::max(area.size.w, 1.0);
        pos.y /= f64::max(area.size.h, 1.0);
        pos
    }

    fn recompute_logical_pos(&mut self) {
        let mut logical_pos = Self::scale_by_working_area(self.working_area, self.pos);

        // Make sure the window doesn't go too much off-screen. Numbers taken from Mutter.
        let min_on_screen_hor = f64::clamp(self.size.w / 4., 10., 75.);
        let min_on_screen_ver = f64::clamp(self.size.h / 4., 10., 75.);
        let max_off_screen_hor = f64::max(0., self.size.w - min_on_screen_hor);
        let max_off_screen_ver = f64::max(0., self.size.h - min_on_screen_ver);

        logical_pos -= self.working_area.loc;
        logical_pos.x = f64::max(logical_pos.x, -max_off_screen_hor);
        logical_pos.y = f64::max(logical_pos.y, -max_off_screen_ver);
        logical_pos.x = f64::min(
            logical_pos.x,
            self.working_area.size.w - self.size.w + max_off_screen_hor,
        );
        logical_pos.y = f64::min(
            logical_pos.y,
            self.working_area.size.h - self.size.h + max_off_screen_ver,
        );
        logical_pos += self.working_area.loc;

        self.logical_pos = logical_pos;
    }

    pub fn update_config(&mut self, working_area: Rectangle<f64, Logical>) {
        if self.working_area == working_area {
            return;
        }

        self.working_area = working_area;
        self.recompute_logical_pos();
    }

    pub fn update<W: LayoutElement>(&mut self, tile: &Tile<W>) {
        let size = tile.tile_size();
        if self.size == size {
            return;
        }

        self.size = size;
        self.recompute_logical_pos();
    }

    pub fn set_logical_pos(&mut self, logical_pos: Point<f64, Logical>) {
        self.pos = Self::logical_to_size_frac_in_working_area(self.working_area, logical_pos);

        // This will clamp the logical position to the current working area.
        self.recompute_logical_pos();
    }

    pub fn center(&self) -> Point<f64, Logical> {
        self.logical_pos + self.size.downscale(2.)
    }

    #[cfg(test)]
    fn verify_invariants(&self) {
        let mut temp = *self;
        temp.recompute_logical_pos();
        assert_eq!(
            self.logical_pos, temp.logical_pos,
            "cached logical pos must be up to date"
        );
    }
}

impl<W: LayoutElement> FloatingSpace<W> {
    pub fn new(
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        scale: f64,
        clock: Clock,
        options: Rc<Options>,
    ) -> Self {
        Self {
            tiles: Vec::new(),
            data: Vec::new(),
            active_window_id: None,
            interactive_resize: None,
            closing_windows: Vec::new(),
            view_size,
            working_area,
            scale,
            clock,
            options,
        }
    }

    pub fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) {
        for (tile, data) in zip(&mut self.tiles, &mut self.data) {
            tile.update_config(view_size, scale, options.clone());
            data.update(tile);
            data.update_config(working_area);
        }

        self.view_size = view_size;
        self.working_area = working_area;
        self.scale = scale;
        self.options = options;
    }

    pub fn update_shaders(&mut self) {
        for tile in &mut self.tiles {
            tile.update_shaders();
        }
    }

    pub fn advance_animations(&mut self) {
        for tile in &mut self.tiles {
            tile.advance_animations();
        }

        self.closing_windows.retain_mut(|closing| {
            closing.advance_animations();
            closing.are_animations_ongoing()
        });
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.tiles.iter().any(Tile::are_animations_ongoing) || !self.closing_windows.is_empty()
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        self.tiles.iter().any(Tile::are_transitions_ongoing) || !self.closing_windows.is_empty()
    }

    pub fn update_render_elements(&mut self, is_active: bool, view_rect: Rectangle<f64, Logical>) {
        let active = self.active_window_id.clone();
        for (tile, offset) in self.tiles_with_offsets_mut() {
            let id = tile.window().id();
            let is_active = is_active && Some(id) == active.as_ref();

            let mut tile_view_rect = view_rect;
            tile_view_rect.loc -= offset + tile.render_offset();
            tile.update_render_elements(is_active, tile_view_rect);
        }
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile<W>> + '_ {
        self.tiles.iter()
    }

    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<W>> + '_ {
        self.tiles.iter_mut()
    }

    pub fn tiles_with_offsets(&self) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> + '_ {
        let offsets = self.data.iter().map(|d| d.logical_pos);
        zip(&self.tiles, offsets)
    }

    pub fn tiles_with_offsets_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> + '_ {
        let offsets = self.data.iter().map(|d| d.logical_pos);
        zip(&mut self.tiles, offsets)
    }

    pub fn tiles_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> {
        let scale = self.scale;
        self.tiles_with_offsets().map(move |(tile, offset)| {
            let pos = offset + tile.render_offset();
            // Round to physical pixels.
            let pos = pos.to_physical_precise_round(scale).to_logical(scale);
            (tile, pos)
        })
    }

    pub fn tiles_with_render_positions_mut(
        &mut self,
        round: bool,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> {
        let scale = self.scale;
        self.tiles_with_offsets_mut().map(move |(tile, offset)| {
            let mut pos = offset + tile.render_offset();
            // Round to physical pixels.
            if round {
                pos = pos.to_physical_precise_round(scale).to_logical(scale);
            }
            (tile, pos)
        })
    }

    pub fn tiles_with_ipc_layouts(&self) -> impl Iterator<Item = (&Tile<W>, WindowLayout)> {
        let scale = self.scale;
        self.tiles_with_offsets().map(move |(tile, offset)| {
            // Do not include animated render offset here to avoid IPC spam.
            let pos = offset;
            // Round to physical pixels.
            let pos = pos.to_physical_precise_round(scale).to_logical(scale);

            let layout = WindowLayout {
                tile_pos_in_workspace_view: Some(pos.into()),
                ..tile.ipc_layout_template()
            };
            (tile, layout)
        })
    }

    pub fn new_window_toplevel_bounds(&self, rules: &ResolvedWindowRules) -> Size<i32, Logical> {
        let border_config = self.options.layout.border.merged_with(&rules.border);
        compute_toplevel_bounds(border_config, self.working_area.size)
    }

    /// Returns the geometry of the active tile relative to and clamped to the working area.
    ///
    /// During animations, assumes the final tile position.
    pub fn active_tile_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        let (tile, offset) = self.tiles_with_offsets().next()?;

        let tile_size = tile.tile_size();
        let tile_rect = Rectangle::new(offset, tile_size);

        self.working_area.intersection(tile_rect)
    }

    pub fn popup_target_rect(&self, id: &W::Id) -> Option<Rectangle<f64, Logical>> {
        for (tile, pos) in self.tiles_with_offsets() {
            if tile.window().id() == id {
                // Position within the working area.
                let mut target = self.working_area;
                target.loc -= pos;
                target.loc -= tile.window_loc();

                return Some(target);
            }
        }
        None
    }

    fn idx_of(&self, id: &W::Id) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.window().id() == id)
    }

    fn contains(&self, id: &W::Id) -> bool {
        self.idx_of(id).is_some()
    }

    pub fn active_window(&self) -> Option<&W> {
        let id = self.active_window_id.as_ref()?;
        self.tiles
            .iter()
            .find(|tile| tile.window().id() == id)
            .map(Tile::window)
    }

    pub fn active_window_mut(&mut self) -> Option<&mut W> {
        let id = self.active_window_id.as_ref()?;
        self.tiles
            .iter_mut()
            .find(|tile| tile.window().id() == id)
            .map(Tile::window_mut)
    }

    pub fn has_window(&self, id: &W::Id) -> bool {
        self.tiles.iter().any(|tile| tile.window().id() == id)
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn add_tile(&mut self, tile: Tile<W>, activate: bool) {
        self.add_tile_at(0, tile, activate);
    }

    fn add_tile_at(&mut self, mut idx: usize, mut tile: Tile<W>, activate: bool) {
        tile.update_config(self.view_size, self.scale, self.options.clone());

        // Restore the previous floating window size, and in case the tile is fullscreen,
        // unfullscreen it.
        let floating_size = tile.floating_window_size;
        let win = tile.window_mut();
        let mut size = if !win.pending_sizing_mode().is_normal() {
            // If the window was fullscreen or maximized without a floating size, ask for (0, 0).
            floating_size.unwrap_or_default()
        } else {
            // If the window wasn't fullscreen without a floating size (e.g. it was tiled before),
            // ask for the current size. If the current size is unknown (the window was only ever
            // fullscreen until now), fall back to (0, 0).
            floating_size.unwrap_or_else(|| win.expected_size().unwrap_or_default())
        };

        // Apply min/max size window rules. If requesting a concrete size, apply completely; if
        // requesting (0, 0), apply only when min/max results in a fixed size.
        let min_size = win.min_size();
        let max_size = win.max_size();
        size.w = ensure_min_max_size_maybe_zero(size.w, min_size.w, max_size.w);
        size.h = ensure_min_max_size_maybe_zero(size.h, min_size.h, max_size.h);

        win.request_size_once(size, true);

        if activate || self.tiles.is_empty() {
            self.active_window_id = Some(win.id().clone());
        }

        // Make sure the tile isn't inserted below its parent.
        for (i, tile_above) in self.tiles.iter().enumerate().take(idx) {
            if win.is_child_of(tile_above.window()) {
                idx = i;
                break;
            }
        }

        let pos = self.stored_or_default_tile_pos(&tile).unwrap_or_else(|| {
            center_preferring_top_left_in_area(self.working_area, tile.tile_size())
        });

        let data = Data::new(self.working_area, &tile, pos);
        self.data.insert(idx, data);
        self.tiles.insert(idx, tile);

        self.bring_up_descendants_of(idx);
    }

    pub fn add_tile_above(&mut self, above: &W::Id, mut tile: Tile<W>, activate: bool) {
        let idx = self.idx_of(above).unwrap();

        let above_pos = self.data[idx].logical_pos;
        let above_size = self.data[idx].size;
        let tile_size = tile.tile_size();
        let pos = above_pos + (above_size.to_point() - tile_size.to_point()).downscale(2.);
        let pos = self.clamp_within_working_area(pos, tile_size);
        tile.floating_pos = Some(self.logical_to_size_frac(pos));

        self.add_tile_at(idx, tile, activate);
    }

    fn bring_up_descendants_of(&mut self, idx: usize) {
        let tile = &self.tiles[idx];
        let win = tile.window();

        // We always maintain the correct stacking order, so walking descendants back to front
        // should give us all of them.
        let mut descendants: Vec<usize> = Vec::new();
        for (i, tile_below) in self.tiles.iter().enumerate().skip(idx + 1).rev() {
            let win_below = tile_below.window();
            if win_below.is_child_of(win)
                || descendants
                    .iter()
                    .any(|idx| win_below.is_child_of(self.tiles[*idx].window()))
            {
                descendants.push(i);
            }
        }

        // Now, descendants is in back-to-front order, and repositioning them in the front-to-back
        // order will preserve the subsequent indices and work out right.
        let mut idx = idx;
        for descendant_idx in descendants.into_iter().rev() {
            self.raise_window(descendant_idx, idx);
            idx += 1;
        }
    }

    pub fn remove_active_tile(&mut self) -> Option<RemovedTile<W>> {
        let id = self.active_window_id.clone()?;
        Some(self.remove_tile(&id))
    }

    pub fn remove_tile(&mut self, id: &W::Id) -> RemovedTile<W> {
        let idx = self.idx_of(id).unwrap();
        self.remove_tile_by_idx(idx)
    }

    fn remove_tile_by_idx(&mut self, idx: usize) -> RemovedTile<W> {
        let mut tile = self.tiles.remove(idx);
        let data = self.data.remove(idx);

        if self.tiles.is_empty() {
            self.active_window_id = None;
        } else if Some(tile.window().id()) == self.active_window_id.as_ref() {
            // The active tile was removed, make the topmost tile active.
            self.active_window_id = Some(self.tiles[0].window().id().clone());
        }

        // Stop interactive resize.
        if let Some(resize) = &self.interactive_resize {
            if tile.window().id() == &resize.window {
                self.interactive_resize = None;
            }
        }

        // Store the floating size if we have one.
        if let Some(size) = tile.window().expected_size() {
            tile.floating_window_size = Some(size);
        }
        // Store the floating position.
        tile.floating_pos = Some(data.pos);

        let width = ColumnWidth::Fixed(tile.tile_expected_or_current_size().w);
        RemovedTile {
            tile,
            width,
            is_full_width: false,
            is_floating: true,
        }
    }

    pub fn start_close_animation_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        id: &W::Id,
        blocker: TransactionBlocker,
    ) {
        let (tile, tile_pos) = self
            .tiles_with_render_positions_mut(false)
            .find(|(tile, _)| tile.window().id() == id)
            .unwrap();

        let Some(snapshot) = tile.take_unmap_snapshot() else {
            return;
        };

        let tile_size = tile.tile_size();

        self.start_close_animation_for_tile(renderer, snapshot, tile_size, tile_pos, blocker);
    }

    pub fn activate_window_without_raising(&mut self, id: &W::Id) -> bool {
        if !self.contains(id) {
            return false;
        }

        self.active_window_id = Some(id.clone());
        true
    }

    pub fn activate_window(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };

        self.raise_window(idx, 0);
        self.active_window_id = Some(id.clone());
        self.bring_up_descendants_of(0);

        true
    }

    fn raise_window(&mut self, from_idx: usize, to_idx: usize) {
        assert!(to_idx <= from_idx);

        let tile = self.tiles.remove(from_idx);
        let data = self.data.remove(from_idx);
        self.tiles.insert(to_idx, tile);
        self.data.insert(to_idx, data);
    }

    pub fn start_close_animation_for_tile(
        &mut self,
        renderer: &mut GlesRenderer,
        snapshot: TileRenderSnapshot,
        tile_size: Size<f64, Logical>,
        tile_pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
    ) {
        let anim = Animation::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            self.options.animations.window_close.anim,
        );

        let blocker = if self.options.disable_transactions {
            TransactionBlocker::completed()
        } else {
            blocker
        };

        let scale = Scale::from(self.scale);
        let res = ClosingWindow::new(
            renderer, snapshot, scale, tile_size, tile_pos, blocker, anim,
        );
        match res {
            Ok(closing) => {
                self.closing_windows.push(closing);
            }
            Err(err) => {
                warn!("error creating a closing window animation: {err:?}");
            }
        }
    }

    pub fn toggle_window_width(&mut self, id: Option<&W::Id>, forwards: bool) {
        let Some(id) = id.or(self.active_window_id.as_ref()).cloned() else {
            return;
        };
        let idx = self.idx_of(&id).unwrap();

        let available_size = self.working_area.size.w;

        let len = self.options.layout.preset_column_widths.len();
        let tile = &mut self.tiles[idx];
        let preset_idx = if let Some(idx) = tile.floating_preset_width_idx {
            (idx + if forwards { 1 } else { len - 1 }) % len
        } else {
            let current_window = tile.window_expected_or_current_size().w;
            let current_tile = tile.tile_expected_or_current_size().w;

            let mut it = self
                .options
                .layout
                .preset_column_widths
                .iter()
                .map(|preset| resolve_preset_size(*preset, available_size));

            if forwards {
                it.position(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => current_tile + 1. < resolved,
                        ResolvedSize::Window(resolved) => current_window + 1. < resolved,
                    }
                })
                .unwrap_or(0)
            } else {
                it.rposition(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => resolved + 1. < current_tile,
                        ResolvedSize::Window(resolved) => resolved + 1. < current_window,
                    }
                })
                .unwrap_or(len - 1)
            }
        };

        let preset = self.options.layout.preset_column_widths[preset_idx];
        self.set_window_width(Some(&id), SizeChange::from(preset), true);

        self.tiles[idx].floating_preset_width_idx = Some(preset_idx);

        self.interactive_resize_end(Some(&id));
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };

        self.tiles[idx].start_open_animation();
        true
    }

    pub fn toggle_window_height(&mut self, id: Option<&W::Id>, forwards: bool) {
        let Some(id) = id.or(self.active_window_id.as_ref()).cloned() else {
            return;
        };
        let idx = self.idx_of(&id).unwrap();

        let available_size = self.working_area.size.h;

        let len = self.options.layout.preset_window_heights.len();
        let tile = &mut self.tiles[idx];
        let preset_idx = if let Some(idx) = tile.floating_preset_height_idx {
            (idx + if forwards { 1 } else { len - 1 }) % len
        } else {
            let current_window = tile.window_expected_or_current_size().h;
            let current_tile = tile.tile_expected_or_current_size().h;

            let mut it = self
                .options
                .layout
                .preset_window_heights
                .iter()
                .map(|preset| resolve_preset_size(*preset, available_size));

            if forwards {
                it.position(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => current_tile + 1. < resolved,
                        ResolvedSize::Window(resolved) => current_window + 1. < resolved,
                    }
                })
                .unwrap_or(0)
            } else {
                it.rposition(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => resolved + 1. < current_tile,
                        ResolvedSize::Window(resolved) => resolved + 1. < current_window,
                    }
                })
                .unwrap_or(len - 1)
            }
        };

        let preset = self.options.layout.preset_window_heights[preset_idx];
        self.set_window_height(Some(&id), SizeChange::from(preset), true);

        let tile = &mut self.tiles[idx];
        tile.floating_preset_height_idx = Some(preset_idx);

        self.interactive_resize_end(Some(&id));
    }

    pub fn set_window_width(&mut self, id: Option<&W::Id>, change: SizeChange, animate: bool) {
        let Some(id) = id.or(self.active_window_id.as_ref()) else {
            return;
        };
        let idx = self.idx_of(id).unwrap();

        let tile = &mut self.tiles[idx];
        tile.floating_preset_width_idx = None;

        let available_size = self.working_area.size.w;
        let win = tile.window();
        let current_window = win.expected_size().unwrap_or_else(|| win.size()).w;
        let current_tile = tile.tile_expected_or_current_size().w;

        const MAX_PX: f64 = 100000.;
        const MAX_F: f64 = 10000.;

        let win_width = match change {
            SizeChange::SetFixed(win_width) => f64::from(win_width),
            SizeChange::SetProportion(prop) => {
                let prop = (prop / 100.).clamp(0., MAX_F);
                let tile_width = available_size * prop;
                tile.window_width_for_tile_width(tile_width)
            }
            SizeChange::AdjustFixed(delta) => f64::from(current_window.saturating_add(delta)),
            SizeChange::AdjustProportion(delta) => {
                let current_prop = current_tile / available_size;
                let prop = (current_prop + delta / 100.).clamp(0., MAX_F);
                let tile_width = available_size * prop;
                tile.window_width_for_tile_width(tile_width)
            }
        };
        let win_width = win_width.round().clamp(1., MAX_PX) as i32;

        let win = tile.window_mut();
        let min_size = win.min_size();
        let max_size = win.max_size();

        let win_width = ensure_min_max_size(win_width, min_size.w, max_size.w);

        let win_height = win.expected_size().unwrap_or_default().h;
        let win_height = ensure_min_max_size(win_height, min_size.h, max_size.h);

        let win_size = Size::from((win_width, win_height));
        win.request_size_once(win_size, animate);
    }

    pub fn set_window_height(&mut self, id: Option<&W::Id>, change: SizeChange, animate: bool) {
        let Some(id) = id.or(self.active_window_id.as_ref()) else {
            return;
        };
        let idx = self.idx_of(id).unwrap();

        let tile = &mut self.tiles[idx];
        tile.floating_preset_height_idx = None;

        let available_size = self.working_area.size.h;
        let win = tile.window();
        let current_window = win.expected_size().unwrap_or_else(|| win.size()).h;
        let current_tile = tile.tile_expected_or_current_size().h;

        const MAX_PX: f64 = 100000.;
        const MAX_F: f64 = 10000.;

        let win_height = match change {
            SizeChange::SetFixed(win_height) => f64::from(win_height),
            SizeChange::SetProportion(prop) => {
                let prop = (prop / 100.).clamp(0., MAX_F);
                let tile_height = available_size * prop;
                tile.window_height_for_tile_height(tile_height)
            }
            SizeChange::AdjustFixed(delta) => f64::from(current_window.saturating_add(delta)),
            SizeChange::AdjustProportion(delta) => {
                let current_prop = current_tile / available_size;
                let prop = (current_prop + delta / 100.).clamp(0., MAX_F);
                let tile_height = available_size * prop;
                tile.window_height_for_tile_height(tile_height)
            }
        };
        let win_height = win_height.round().clamp(1., MAX_PX) as i32;

        let win = tile.window_mut();
        let min_size = win.min_size();
        let max_size = win.max_size();

        let win_height = ensure_min_max_size(win_height, min_size.h, max_size.h);

        let win_width = win.expected_size().unwrap_or_default().w;
        let win_width = ensure_min_max_size(win_width, min_size.w, max_size.w);

        let win_size = Size::from((win_width, win_height));
        win.request_size_once(win_size, animate);
    }

    fn focus_directional(
        &mut self,
        distance: impl Fn(Point<f64, Logical>, Point<f64, Logical>) -> f64,
    ) -> bool {
        let Some(active_id) = &self.active_window_id else {
            return false;
        };
        let active_idx = self.idx_of(active_id).unwrap();
        let center = self.data[active_idx].center();

        let result = zip(&self.tiles, &self.data)
            .filter(|(tile, _)| tile.window().id() != active_id)
            .map(|(tile, data)| (tile, distance(center, data.center())))
            .filter(|(_, dist)| *dist > 0.)
            .min_by(|(_, dist_a), (_, dist_b)| f64::total_cmp(dist_a, dist_b));
        if let Some((tile, _)) = result {
            let id = tile.window().id().clone();
            self.activate_window(&id);
            true
        } else {
            false
        }
    }

    pub fn focus_left(&mut self) -> bool {
        self.focus_directional(|focus, other| focus.x - other.x)
    }

    pub fn focus_right(&mut self) -> bool {
        self.focus_directional(|focus, other| other.x - focus.x)
    }

    pub fn focus_up(&mut self) -> bool {
        self.focus_directional(|focus, other| focus.y - other.y)
    }

    pub fn focus_down(&mut self) -> bool {
        self.focus_directional(|focus, other| other.y - focus.y)
    }

    pub fn focus_leftmost(&mut self) {
        let result = self
            .tiles_with_offsets()
            .min_by(|(_, pos_a), (_, pos_b)| f64::total_cmp(&pos_a.x, &pos_b.x));
        if let Some((tile, _)) = result {
            let id = tile.window().id().clone();
            self.activate_window(&id);
        }
    }

    pub fn focus_rightmost(&mut self) {
        let result = self
            .tiles_with_offsets()
            .max_by(|(_, pos_a), (_, pos_b)| f64::total_cmp(&pos_a.x, &pos_b.x));
        if let Some((tile, _)) = result {
            let id = tile.window().id().clone();
            self.activate_window(&id);
        }
    }

    pub fn focus_topmost(&mut self) {
        let result = self
            .tiles_with_offsets()
            .min_by(|(_, pos_a), (_, pos_b)| f64::total_cmp(&pos_a.y, &pos_b.y));
        if let Some((tile, _)) = result {
            let id = tile.window().id().clone();
            self.activate_window(&id);
        }
    }

    pub fn focus_bottommost(&mut self) {
        let result = self
            .tiles_with_offsets()
            .max_by(|(_, pos_a), (_, pos_b)| f64::total_cmp(&pos_a.y, &pos_b.y));
        if let Some((tile, _)) = result {
            let id = tile.window().id().clone();
            self.activate_window(&id);
        }
    }

    fn move_to(&mut self, idx: usize, new_pos: Point<f64, Logical>, animate: bool) {
        if animate {
            self.move_and_animate(idx, new_pos);
        } else {
            self.data[idx].set_logical_pos(new_pos);
        }

        self.interactive_resize_end(None);
    }

    fn move_by(&mut self, amount: Point<f64, Logical>) {
        let Some(active_id) = &self.active_window_id else {
            return;
        };
        let idx = self.idx_of(active_id).unwrap();

        let new_pos = self.data[idx].logical_pos + amount;
        self.move_to(idx, new_pos, true)
    }

    pub fn move_left(&mut self) {
        self.move_by(Point::from((-DIRECTIONAL_MOVE_PX, 0.)));
    }

    pub fn move_right(&mut self) {
        self.move_by(Point::from((DIRECTIONAL_MOVE_PX, 0.)));
    }

    pub fn move_up(&mut self) {
        self.move_by(Point::from((0., -DIRECTIONAL_MOVE_PX)));
    }

    pub fn move_down(&mut self) {
        self.move_by(Point::from((0., DIRECTIONAL_MOVE_PX)));
    }

    pub fn move_window(
        &mut self,
        id: Option<&W::Id>,
        x: PositionChange,
        y: PositionChange,
        animate: bool,
    ) {
        let Some(id) = id.or(self.active_window_id.as_ref()) else {
            return;
        };
        let idx = self.idx_of(id).unwrap();

        let mut pos = self.data[idx].logical_pos;

        let available_width = self.working_area.size.w;
        let available_height = self.working_area.size.h;
        let working_area_loc = self.working_area.loc;

        const MAX_F: f64 = 10000.;

        match x {
            PositionChange::SetFixed(x) => pos.x = x + working_area_loc.x,
            PositionChange::SetProportion(prop) => {
                let prop = (prop / 100.).clamp(0., MAX_F);
                pos.x = available_width * prop + working_area_loc.x;
            }
            PositionChange::AdjustFixed(x) => pos.x += x,
            PositionChange::AdjustProportion(prop) => {
                let current_prop = (pos.x - working_area_loc.x) / available_width.max(1.);
                let prop = (current_prop + prop / 100.).clamp(0., MAX_F);
                pos.x = available_width * prop + working_area_loc.x;
            }
        }
        match y {
            PositionChange::SetFixed(y) => pos.y = y + working_area_loc.y,
            PositionChange::SetProportion(prop) => {
                let prop = (prop / 100.).clamp(0., MAX_F);
                pos.y = available_height * prop + working_area_loc.y;
            }
            PositionChange::AdjustFixed(y) => pos.y += y,
            PositionChange::AdjustProportion(prop) => {
                let current_prop = (pos.y - working_area_loc.y) / available_height.max(1.);
                let prop = (current_prop + prop / 100.).clamp(0., MAX_F);
                pos.y = available_height * prop + working_area_loc.y;
            }
        }

        self.move_to(idx, pos, animate);
    }

    pub fn center_window(&mut self, id: Option<&W::Id>) {
        let Some(id) = id.or(self.active_window_id.as_ref()).cloned() else {
            return;
        };
        let idx = self.idx_of(&id).unwrap();

        let new_pos = center_preferring_top_left_in_area(self.working_area, self.data[idx].size);
        self.move_to(idx, new_pos, true);
    }

    pub fn descendants_added(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };

        self.bring_up_descendants_of(idx);
        true
    }

    pub fn update_window(&mut self, id: &W::Id, serial: Option<Serial>) -> bool {
        let Some(tile_idx) = self.idx_of(id) else {
            return false;
        };

        let tile = &mut self.tiles[tile_idx];
        let data = &mut self.data[tile_idx];

        let resize = tile.window_mut().interactive_resize_data();

        // Do this before calling update_window() so it can get up-to-date info.
        if let Some(serial) = serial {
            tile.window_mut().on_commit(serial);
        }

        let prev_size = data.size;

        tile.update_window();
        data.update(tile);

        // When resizing by top/left edge, update the position accordingly.
        if let Some(resize) = resize {
            let mut offset = Point::from((0., 0.));
            if resize.edges.contains(ResizeEdge::LEFT) {
                offset.x += prev_size.w - data.size.w;
            }
            if resize.edges.contains(ResizeEdge::TOP) {
                offset.y += prev_size.h - data.size.h;
            }
            data.set_logical_pos(data.logical_pos + offset);
        }

        true
    }

    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        view_rect: Rectangle<f64, Logical>,
        target: RenderTarget,
        focus_ring: bool,
        push: &mut dyn FnMut(FloatingSpaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);

        // Draw the closing windows on top of the other windows.
        //
        // FIXME: I guess this should rather preserve the stacking order when the window is closed.
        for closing in self.closing_windows.iter().rev() {
            let elem = closing.render(renderer.as_gles_renderer(), view_rect, scale, target);
            push(elem.into());
        }

        let active = self.active_window_id.clone();
        for (tile, tile_pos) in self.tiles_with_render_positions() {
            // For the active tile, draw the focus ring.
            let focus_ring = focus_ring && Some(tile.window().id()) == active.as_ref();

            tile.render(renderer, tile_pos, focus_ring, target, &mut |elem| {
                push(elem.into())
            });
        }
    }

    pub fn interactive_resize_begin(&mut self, window: W::Id, edges: ResizeEdge) -> bool {
        if self.interactive_resize.is_some() {
            return false;
        }

        let tile = self
            .tiles
            .iter_mut()
            .find(|tile| tile.window().id() == &window)
            .unwrap();

        let original_window_size = tile.window_size();

        let resize = InteractiveResize {
            window,
            original_window_size,
            data: InteractiveResizeData { edges },
        };
        self.interactive_resize = Some(resize);

        true
    }

    pub fn interactive_resize_update(
        &mut self,
        window: &W::Id,
        delta: Point<f64, Logical>,
    ) -> bool {
        let Some(resize) = &self.interactive_resize else {
            return false;
        };

        if window != &resize.window {
            return false;
        }

        let original_window_size = resize.original_window_size;
        let edges = resize.data.edges;

        if edges.intersects(ResizeEdge::LEFT_RIGHT) {
            let mut dx = delta.x;
            if edges.contains(ResizeEdge::LEFT) {
                dx = -dx;
            };

            let window_width = (original_window_size.w + dx).round() as i32;
            self.set_window_width(Some(window), SizeChange::SetFixed(window_width), false);
        }

        if edges.intersects(ResizeEdge::TOP_BOTTOM) {
            let mut dy = delta.y;
            if edges.contains(ResizeEdge::TOP) {
                dy = -dy;
            };

            let window_height = (original_window_size.h + dy).round() as i32;
            self.set_window_height(Some(window), SizeChange::SetFixed(window_height), false);
        }

        true
    }

    pub fn interactive_resize_end(&mut self, window: Option<&W::Id>) {
        let Some(resize) = &self.interactive_resize else {
            return;
        };

        if let Some(window) = window {
            if window != &resize.window {
                return;
            }
        }

        self.interactive_resize = None;
    }

    pub fn refresh(&mut self, is_active: bool, is_focused: bool) {
        let active = self.active_window_id.clone();
        for tile in &mut self.tiles {
            let win = tile.window_mut();

            win.set_active_in_column(true);
            win.set_floating(true);

            let mut is_active = is_active && Some(win.id()) == active.as_ref();
            if self.options.deactivate_unfocused_windows {
                is_active &= is_focused;
            }
            win.set_activated(is_active);

            let resize_data = self
                .interactive_resize
                .as_ref()
                .filter(|resize| &resize.window == win.id())
                .map(|resize| resize.data);
            win.set_interactive_resize(resize_data);

            let border_config = self.options.layout.border.merged_with(&win.rules().border);
            let bounds = compute_toplevel_bounds(border_config, self.working_area.size);
            win.set_bounds(bounds);

            // If transactions are disabled, also disable combined throttling, for more
            // intuitive behavior.
            let intent = if self.options.disable_resize_throttling {
                ConfigureIntent::CanSend
            } else {
                win.configure_intent()
            };

            if matches!(
                intent,
                ConfigureIntent::CanSend | ConfigureIntent::ShouldSend
            ) {
                win.send_pending_configure();
            }

            win.refresh();
        }
    }

    pub fn clamp_within_working_area(
        &self,
        pos: Point<f64, Logical>,
        size: Size<f64, Logical>,
    ) -> Point<f64, Logical> {
        let mut rect = Rectangle::new(pos, size);
        clamp_preferring_top_left_in_area(self.working_area, &mut rect);
        rect.loc
    }

    pub fn scale_by_working_area(&self, pos: Point<f64, SizeFrac>) -> Point<f64, Logical> {
        Data::scale_by_working_area(self.working_area, pos)
    }

    pub fn logical_to_size_frac(&self, logical_pos: Point<f64, Logical>) -> Point<f64, SizeFrac> {
        Data::logical_to_size_frac_in_working_area(self.working_area, logical_pos)
    }

    fn move_and_animate(&mut self, idx: usize, new_pos: Point<f64, Logical>) {
        // Moves up to this logical pixel distance are not animated.
        const ANIMATION_THRESHOLD_SQ: f64 = 10. * 10.;

        let tile = &mut self.tiles[idx];
        let data = &mut self.data[idx];

        let prev_pos = data.logical_pos;
        data.set_logical_pos(new_pos);
        let new_pos = data.logical_pos;

        let diff = prev_pos - new_pos;
        if diff.x * diff.x + diff.y * diff.y > ANIMATION_THRESHOLD_SQ {
            tile.animate_move_from(prev_pos - new_pos);
        }
    }

    pub fn new_window_size(
        &self,
        width: Option<PresetSize>,
        height: Option<PresetSize>,
        rules: &ResolvedWindowRules,
    ) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);

        let resolve = |size: Option<PresetSize>, working_area_size: f64| {
            if let Some(size) = size {
                let size = match resolve_preset_size(size, working_area_size) {
                    ResolvedSize::Tile(mut size) => {
                        if !border.off {
                            size -= border.width * 2.;
                        }
                        size
                    }
                    ResolvedSize::Window(size) => size,
                };

                max(1, size.floor() as i32)
            } else {
                0
            }
        };

        let width = resolve(width, self.working_area.size.w);
        let height = resolve(height, self.working_area.size.h);

        Size::from((width, height))
    }

    pub fn stored_or_default_tile_pos(&self, tile: &Tile<W>) -> Option<Point<f64, Logical>> {
        let pos = tile.floating_pos.map(|pos| self.scale_by_working_area(pos));
        pos.or_else(|| {
            tile.window().rules().default_floating_position.map(|pos| {
                let relative_to = pos.relative_to;
                let size = tile.tile_size();
                let area = self.working_area;

                let mut pos = Point::from((pos.x.0, pos.y.0));
                if relative_to == RelativeTo::TopRight
                    || relative_to == RelativeTo::BottomRight
                    || relative_to == RelativeTo::Right
                {
                    pos.x = area.size.w - size.w - pos.x;
                }
                if relative_to == RelativeTo::BottomLeft
                    || relative_to == RelativeTo::BottomRight
                    || relative_to == RelativeTo::Bottom
                {
                    pos.y = area.size.h - size.h - pos.y;
                }
                if relative_to == RelativeTo::Top || relative_to == RelativeTo::Bottom {
                    pos.x += area.size.w / 2.0 - size.w / 2.0
                }
                if relative_to == RelativeTo::Left || relative_to == RelativeTo::Right {
                    pos.y += area.size.h / 2.0 - size.h / 2.0
                }

                pos + self.working_area.loc
            })
        })
    }

    #[cfg(test)]
    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    pub fn working_area(&self) -> Rectangle<f64, Logical> {
        self.working_area
    }

    #[cfg(test)]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    #[cfg(test)]
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    #[cfg(test)]
    pub fn options(&self) -> &Rc<Options> {
        &self.options
    }

    #[cfg(test)]
    pub fn verify_invariants(&self) {
        assert!(self.scale > 0.);
        assert!(self.scale.is_finite());
        assert_eq!(self.tiles.len(), self.data.len());

        for (i, (tile, data)) in zip(&self.tiles, &self.data).enumerate() {
            use crate::layout::SizingMode;

            assert!(Rc::ptr_eq(&self.options, &tile.options));
            assert_eq!(self.view_size, tile.view_size());
            assert_eq!(self.clock, tile.clock);
            assert_eq!(self.scale, tile.scale());
            tile.verify_invariants();

            if let Some(idx) = tile.floating_preset_width_idx {
                assert!(idx < self.options.layout.preset_column_widths.len());
            }
            if let Some(idx) = tile.floating_preset_height_idx {
                assert!(idx < self.options.layout.preset_window_heights.len());
            }

            assert_eq!(
                tile.window().pending_sizing_mode(),
                SizingMode::Normal,
                "floating windows cannot be maximized or fullscreen"
            );

            data.verify_invariants();

            let mut data2 = *data;
            data2.update(tile);
            data2.update_config(self.working_area);
            assert_eq!(data, &data2, "tile data must be up to date");

            for tile_below in &self.tiles[i + 1..] {
                assert!(
                    !tile_below.window().is_child_of(tile.window()),
                    "children must be stacked above parents"
                );
            }
        }

        if let Some(id) = &self.active_window_id {
            assert!(!self.tiles.is_empty());
            assert!(self.contains(id), "active window must be present in tiles");
        } else {
            assert!(self.tiles.is_empty());
        }

        if let Some(resize) = &self.interactive_resize {
            assert!(
                self.contains(&resize.window),
                "interactive resize window must be present in tiles"
            );
        }
    }
}

fn compute_toplevel_bounds(
    border_config: niri_config::Border,
    working_area_size: Size<f64, Logical>,
) -> Size<i32, Logical> {
    let mut border = 0.;
    if !border_config.off {
        border = border_config.width * 2.;
    }

    Size::from((
        f64::max(working_area_size.w - border, 1.),
        f64::max(working_area_size.h - border, 1.),
    ))
    .to_i32_floor()
}

fn resolve_preset_size(preset: PresetSize, view_size: f64) -> ResolvedSize {
    match preset {
        PresetSize::Proportion(proportion) => ResolvedSize::Tile(view_size * proportion),
        PresetSize::Fixed(width) => ResolvedSize::Window(f64::from(width)),
    }
}
