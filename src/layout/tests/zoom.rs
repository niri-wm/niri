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
