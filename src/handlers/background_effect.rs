use std::sync::{Arc, Mutex};

use smithay::delegate_background_effect;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::background_effect::{
    self, BackgroundEffectSurfaceCachedState, ExtBackgroundEffectHandler,
};
use smithay::wayland::compositor::{
    add_post_commit_hook, with_states, RegionAttributes, SurfaceData,
};

use crate::delegate_kde_blur;
use crate::niri::State;
use crate::protocols::kde_blur::{KdeBlurHandler, KdeBlurRegion, KdeBlurSurfaceCachedState};
use crate::utils::region::region_to_non_overlapping_rects;

/// Per-surface cache for processed blur region (non-overlapping rects).
#[derive(Default)]
struct CachedBlurRegionUserData(Mutex<CachedBlurRegionInner>);

#[derive(Default)]
struct CachedBlurRegionInner {
    /// Whether a region change is pending to be committed.
    pending_dirty: bool,
    /// Whether the region must be recomputed.
    dirty: bool,
    /// Whether the post-commit hook has been registered for this surface.
    hook_registered: bool,
    /// Cached non-overlapping rects in surface-local coordinates.
    ///
    /// `None` means there's no blur region.
    rects: Option<Arc<Vec<Rectangle<i32, Logical>>>>,
}

/// Gets the cached blur region for a surface, lazily recomputing if dirty.
pub fn get_cached_blur_region(states: &SurfaceData) -> Option<Arc<Vec<Rectangle<i32, Logical>>>> {
    let cache = states
        .data_map
        .get_or_insert_threadsafe(CachedBlurRegionUserData::default);
    let mut guard = cache.0.lock().unwrap();

    if guard.dirty {
        guard.dirty = false;
        recompute_blur_region(states, &mut guard);
    }

    guard.rects.clone()
}

fn recompute_blur_region(states: &SurfaceData, inner: &mut CachedBlurRegionInner) {
    let cached = &states.cached_state;

    let rects = if let Some(arc) = &mut inner.rects {
        if Arc::strong_count(arc) > 1 {
            debug!("cloning rects due to non-unique reference");
        }
        arc
    } else {
        inner.rects.insert(Arc::new(Vec::new()))
    };
    let rects = Arc::make_mut(rects);

    // Prefer ext-background-effect.
    if cached.has::<BackgroundEffectSurfaceCachedState>() {
        let mut guard = cached.get::<BackgroundEffectSurfaceCachedState>();
        if let Some(region) = &guard.current().blur_region {
            region_to_non_overlapping_rects(region, rects);
        } else {
            inner.rects = None;
        }
        return;
    }

    if cached.has::<KdeBlurSurfaceCachedState>() {
        let mut guard = cached.get::<KdeBlurSurfaceCachedState>();
        match &guard.current().blur_region {
            Some(KdeBlurRegion::WholeSurface) => {
                // Store a single "infinite" rect that gets naturally clipped.
                let infinite = Rectangle::new(
                    Point::new(-i32::MAX / 2, -i32::MAX / 2),
                    Size::new(i32::MAX, i32::MAX),
                );
                rects.clear();
                rects.push(infinite);
            }
            Some(KdeBlurRegion::Region(region)) => {
                region_to_non_overlapping_rects(region, rects);
            }
            None => {
                inner.rects = None;
            }
        }
        return;
    }

    // Neither is present.
    inner.rects = None;
}

fn mark_blur_region_pending_dirty(wl_surface: &WlSurface) {
    let register_hook = with_states(wl_surface, |states| {
        let cache = states
            .data_map
            .get_or_insert_threadsafe(CachedBlurRegionUserData::default);
        let mut guard = cache.0.lock().unwrap();
        guard.pending_dirty = true;

        if guard.hook_registered {
            false
        } else {
            guard.hook_registered = true;
            true
        }
    });

    if register_hook {
        add_post_commit_hook::<State, _>(wl_surface, |_state, _dh, surface| {
            with_states(surface, |states| {
                if let Some(cache) = states.data_map.get::<CachedBlurRegionUserData>() {
                    let mut guard = cache.0.lock().unwrap();
                    if guard.pending_dirty {
                        guard.pending_dirty = false;
                        guard.dirty = true;
                    }
                } else {
                    error!("unexpected missing CachedBlurRegionUserData");
                }
            });
        });
    }
}

impl ExtBackgroundEffectHandler for State {
    fn capabilities(&self) -> background_effect::Capability {
        background_effect::Capability::Blur
    }

    fn set_blur_region(&mut self, wl_surface: WlSurface, _region: RegionAttributes) {
        mark_blur_region_pending_dirty(&wl_surface);
    }

    fn unset_blur_region(&mut self, wl_surface: WlSurface) {
        mark_blur_region_pending_dirty(&wl_surface);
    }
}
delegate_background_effect!(State);

impl KdeBlurHandler for State {
    fn set_blur_region(&mut self, wl_surface: WlSurface) {
        mark_blur_region_pending_dirty(&wl_surface);
    }

    fn unset_blur_region(&mut self, wl_surface: WlSurface) {
        mark_blur_region_pending_dirty(&wl_surface);
    }
}
delegate_kde_blur!(State);
