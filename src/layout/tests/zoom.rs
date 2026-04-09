//! Tests for zoom animation state machine.

use std::time::Duration;

use niri_config::animations::{Kind, SpringParams};
use smithay::utils::Point;

use crate::animation::Clock;
use crate::layout::{ZoomFocalAnimation, ZoomLevelAnimation, ZoomLevelProgress};
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
fn zoom_level_animation_value() {
    let clock = Clock::with_time(Duration::ZERO);
    let anim = ZoomLevelAnimation::new(clock.clone(), 1.0, 3.0, test_animation_config());

    // Animation just started, should be near 1.0
    let level = anim.value();
    assert!(
        (1.0..1.5).contains(&level),
        "level should be near start: {}",
        level
    );
}

#[test]
fn zoom_focal_animation_value() {
    let clock = Clock::with_time(Duration::ZERO);
    let anim = ZoomFocalAnimation::new(
        clock.clone(),
        Point::from((0.0, 0.0)),
        Point::from((50.0, 100.0)),
        test_animation_config(),
    );

    // Animation just started, should be near start
    let focal = anim.value();
    assert!(focal.x < 10.0, "focal x should be near start: {}", focal.x);
    assert!(focal.y < 20.0, "focal y should be near start: {}", focal.y);
}

#[test]
fn zoom_level_progress_animation_variant() {
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = ZoomLevelAnimation::new(clock.clone(), 1.0, 2.0, test_animation_config());
    let progress = ZoomLevelProgress::Animation(level_anim);

    assert!(progress.is_animation());
    assert!(!progress.is_gesture());
    assert!(!progress.is_done());

    let level = progress.level();
    assert!(
        (1.0..1.5).contains(&level),
        "level should be near start: {}",
        level
    );
}

#[test]
fn zoom_level_progress_is_done() {
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = ZoomLevelAnimation::new(clock.clone(), 1.0, 2.0, test_animation_config());
    let progress = ZoomLevelProgress::Animation(level_anim);

    // Should not be done at start
    assert!(!progress.is_done());
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
fn zoom_focal_animation_is_done() {
    let clock = Clock::with_time(Duration::ZERO);
    let anim = ZoomFocalAnimation::new(
        clock.clone(),
        Point::from((0.0, 0.0)),
        Point::from((100.0, 100.0)),
        test_animation_config(),
    );

    // Should not be done at start
    assert!(!anim.is_done());
}
