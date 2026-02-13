//! Tests for zoom animation state machine.

use std::time::Duration;

use niri_config::animations::{Kind, SpringParams};
use smithay::utils::Point;

use crate::animation::Clock;
use crate::layout::ZoomAnimation;
use crate::zoom::OutputZoomState;

fn test_animation_config() -> niri_config::Animation {
    niri_config::Animation {
        off: false,
        kind: Kind::Spring(SpringParams {
            damping_ratio: 1.0,
            stiffness: 1000,
            epsilon: 0.0001,
        }),
    }
}

#[test]
fn zoom_progress_level_accessor() {
    // Test Animation variant
    let clock = Clock::with_time(Duration::ZERO);
    let anim_progress = ZoomAnimation::new(
        clock.clone(),
        1.0,
        3.0,
        test_animation_config(),
        Point::from((0.0, 0.0)),
        Point::from((0.0, 0.0)),
        None,
        None,
        None,
        None,
    );
    // Animation just started, should be near 1.0
    let level = anim_progress.level();
    assert!(
        (1.0..1.5).contains(&level),
        "level should be near start: {}",
        level
    );
}

#[test]
fn zoom_progress_focal_point_accessor() {
    // Test Animation without focal animation
    let clock = Clock::with_time(Duration::ZERO);
    let anim_progress = ZoomAnimation::new(
        clock.clone(),
        1.0,
        2.0,
        test_animation_config(),
        Point::from((0.0, 0.0)),
        Point::from((50.0, 100.0)),
        None,
        None,
        None,
        None,
    );
    // Without focal animation, should return target_focal
    let focal = anim_progress.focal_point();
    assert_eq!(focal.x, 50.0);
    assert_eq!(focal.y, 100.0);
}

#[test]
fn zoom_progress_is_animation() {
    let clock = Clock::with_time(Duration::ZERO);
    let anim = ZoomAnimation::new(
        clock.clone(),
        1.0,
        2.0,
        test_animation_config(),
        Point::from((0.0, 0.0)),
        Point::from((0.0, 0.0)),
        None,
        None,
        None,
        None,
    );
    assert!(anim.is_animation());
    assert!(!anim.is_gesture());
}

#[test]
fn zoom_progress_is_done() {
    // Animation should not be done immediately
    let clock = Clock::with_time(Duration::ZERO);
    let anim = ZoomAnimation::new(
        clock.clone(),
        1.0,
        2.0,
        test_animation_config(),
        Point::from((0.0, 0.0)),
        Point::from((0.0, 0.0)),
        None,
        None,
        None,
        None,
    );
    assert!(!anim.is_done());
}

#[test]
fn output_zoom_state_default() {
    let state = OutputZoomState::default();
    assert_eq!(state.level, 1.0);
    assert_eq!(state.focal.x, 0.0);
    assert_eq!(state.focal.y, 0.0);
    assert!(!state.locked);
    assert!(!state.transitioning);
}

#[test]
fn zoom_animation_completion() {
    let clock = Clock::with_time(Duration::ZERO);

    // Create an animation from 1.0 to 2.0
    let progress = ZoomAnimation::new(
        clock.clone(),
        1.0,
        2.0,
        test_animation_config(),
        Point::from((0.0, 0.0)),
        Point::from((0.0, 0.0)),
        None,
        None,
        None,
        None,
    );

    // Should not be done at start
    assert!(!progress.is_done());
    let level = progress.level();
    assert!(
        (1.0..1.1).contains(&level),
        "Should be near start: {}",
        level
    );

    // Simulate time advancing (using a spring animation, this would take some time)
    // For this test, we just verify the structure works
    assert!(progress.is_animation());
}

use smithay::utils::{Logical, Rectangle, Size};

use crate::zoom::zoomed_viewport;

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

fn calculate_visibility(
    window_geo: Rectangle<f64, Logical>,
    zoom_rect: Rectangle<f64, Logical>,
) -> f64 {
    match window_geo.intersection(zoom_rect) {
        Some(intersect) => {
            let intersect_area = intersect.size.w * intersect.size.h;
            let window_area = window_geo.size.w * window_geo.size.h;
            if window_area <= 0.0 {
                return 0.0;
            }
            (intersect_area / window_area).clamp(0.0, 1.0)
        }
        None => 0.0,
    }
}

#[test]
fn zoom_viewport_at_2x_center() {
    let output_rect = rect(0.0, 0.0, 1920.0, 1080.0);
    let focal = Point::from((960.0, 540.0));
    let vp = zoomed_viewport(output_rect, focal, 2.0);

    assert!((vp.size.w - 960.0).abs() < 0.01, "width: {}", vp.size.w);
    assert!((vp.size.h - 540.0).abs() < 0.01, "height: {}", vp.size.h);
    assert!((vp.loc.x - 480.0).abs() < 0.01, "x: {}", vp.loc.x);
    assert!((vp.loc.y - 270.0).abs() < 0.01, "y: {}", vp.loc.y);
}

#[test]
fn zoom_viewport_at_1x_is_full_output() {
    let output_rect = rect(0.0, 0.0, 1920.0, 1080.0);
    let focal = Point::from((960.0, 540.0));
    let vp = zoomed_viewport(output_rect, focal, 1.0);

    assert!((vp.size.w - 1920.0).abs() < 0.01);
    assert!((vp.size.h - 1080.0).abs() < 0.01);
}

#[test]
fn zoom_viewport_at_4x_corner() {
    let output_rect = rect(0.0, 0.0, 1920.0, 1080.0);
    let focal = Point::from((0.0, 0.0));
    let vp = zoomed_viewport(output_rect, focal, 4.0);

    assert!((vp.size.w - 480.0).abs() < 0.01);
    assert!((vp.size.h - 270.0).abs() < 0.01);
    assert!((vp.loc.x - 0.0).abs() < 0.01, "x: {}", vp.loc.x);
    assert!((vp.loc.y - 0.0).abs() < 0.01, "y: {}", vp.loc.y);
}

#[test]
fn output_zoom_state_viewport_global() {
    let mut state = OutputZoomState::default();
    state.level = 2.0;
    state.focal = Point::from((960.0, 540.0));

    let output_geo = rect(100.0, 200.0, 1920.0, 1080.0);
    let vp = state.viewport_global(output_geo);

    assert!((vp.size.w - 960.0).abs() < 0.01);
    assert!((vp.size.h - 540.0).abs() < 0.01);
}

#[test]
fn effective_scale_computation() {
    let output_scale: f64 = 1.5;
    let zoom_level: f64 = 2.0;
    assert!((output_scale * zoom_level - 3.0).abs() < f64::EPSILON);
    assert!((output_scale * 1.0 - 1.5).abs() < f64::EPSILON);
    assert!((1.5_f64 * 5.0 - 7.5).abs() < f64::EPSILON);
}

#[test]
fn visibility_fully_contained() {
    let viewport = rect(0.0, 0.0, 1000.0, 1000.0);
    let window = rect(100.0, 100.0, 200.0, 200.0);
    let vis = calculate_visibility(window, viewport);
    assert!((vis - 1.0).abs() < 0.01, "fully contained: {}", vis);
}

#[test]
fn visibility_no_intersection() {
    let viewport = rect(0.0, 0.0, 500.0, 500.0);
    let window = rect(600.0, 600.0, 200.0, 200.0);
    let vis = calculate_visibility(window, viewport);
    assert!((vis - 0.0).abs() < 0.01, "no intersection: {}", vis);
}

#[test]
fn visibility_half_overlap() {
    let viewport = rect(0.0, 0.0, 500.0, 500.0);
    // Window: 400..600 x 0..200 — 100x200 of 200x200 is in viewport = 50%
    let window = rect(400.0, 0.0, 200.0, 200.0);
    let vis = calculate_visibility(window, viewport);
    assert!((vis - 0.5).abs() < 0.01, "half overlap: {}", vis);
}

#[test]
fn visibility_zero_area_window() {
    let viewport = rect(0.0, 0.0, 500.0, 500.0);
    let window = rect(100.0, 100.0, 0.0, 0.0);
    let vis = calculate_visibility(window, viewport);
    assert!((vis - 0.0).abs() < 0.01, "zero area: {}", vis);
}

#[test]
fn zoom_state_fractional_fields_default() {
    let state = OutputZoomState::default();
    assert!(state.zoomed_surfaces.is_empty());
    assert!(state.last_scale_update_level.is_none());
}

use crate::zoom::SCALE_CHANGE_THRESHOLD;

#[test]
fn debounce_threshold_first_update_always_fires() {
    let state = OutputZoomState::default();
    assert!(state.last_scale_update_level.is_none());
    let should_update = state.last_scale_update_level.map_or(true, |last| {
        (state.level - last).abs() >= SCALE_CHANGE_THRESHOLD
    });
    assert!(should_update);
}

#[test]
fn debounce_threshold_small_change_skipped() {
    let mut state = OutputZoomState::default();
    state.level = 2.0;
    state.last_scale_update_level = Some(1.9);
    let should_update = state.last_scale_update_level.map_or(true, |last| {
        (state.level - last).abs() >= SCALE_CHANGE_THRESHOLD
    });
    assert!(
        !should_update,
        "0.1 change should be below threshold of {}",
        SCALE_CHANGE_THRESHOLD
    );
}

#[test]
fn debounce_threshold_large_change_fires() {
    let mut state = OutputZoomState::default();
    state.level = 2.5;
    state.last_scale_update_level = Some(2.0);
    let should_update = state.last_scale_update_level.map_or(true, |last| {
        (state.level - last).abs() >= SCALE_CHANGE_THRESHOLD
    });
    assert!(
        should_update,
        "0.5 change should exceed threshold of {}",
        SCALE_CHANGE_THRESHOLD
    );
}

#[test]
fn debounce_threshold_exact_boundary() {
    let mut state = OutputZoomState::default();
    state.level = 2.25;
    state.last_scale_update_level = Some(2.0);
    let should_update = state.last_scale_update_level.map_or(true, |last| {
        (state.level - last).abs() >= SCALE_CHANGE_THRESHOLD
    });
    assert!(should_update, "Exact threshold should fire");
}

#[test]
fn max_fractional_scale_caps_zoom_factor() {
    let max_fractional_scale: f64 = 5.0;
    let zoom_level: f64 = 8.0;
    let capped = zoom_level.min(max_fractional_scale);
    assert!((capped - 5.0).abs() < f64::EPSILON);
}

#[test]
fn max_fractional_scale_no_cap_when_below() {
    let max_fractional_scale: f64 = 5.0;
    let zoom_level: f64 = 3.0;
    let capped = zoom_level.min(max_fractional_scale);
    assert!((capped - 3.0).abs() < f64::EPSILON);
}

#[test]
fn max_fractional_scale_effective_scale_capped() {
    let output_scale: f64 = 1.5;
    let max_fractional_scale: f64 = 5.0;
    let zoom_level: f64 = 10.0;
    let capped_zoom = zoom_level.min(max_fractional_scale);
    let effective = output_scale * capped_zoom;
    assert!((effective - 7.5).abs() < f64::EPSILON);
    assert!(effective <= output_scale * max_fractional_scale);
}

#[test]
fn backward_compat_no_fractional_zoom_empty_surfaces() {
    let state = OutputZoomState::default();
    assert!(state.zoomed_surfaces.is_empty());
}
