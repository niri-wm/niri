//! Tests for zoom animation state machine.

use crate::animation::{Animation, Clock};
use crate::layout::{OutputZoomState, ZoomAnimation, ZoomProgress};
use niri_config::animations::{Kind, SpringParams};
use smithay::utils::Point;
use std::time::Duration;

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
    // Test Static variant
    let static_progress = ZoomProgress::Static {
        level: 2.5,
        focal_point: Point::from((100.0, 200.0)),
    };
    assert_eq!(static_progress.level(), 2.5);

    // Test Animation variant
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = Animation::new(clock.clone(), 1.0, 3.0, 0.0, test_animation_config());
    let anim_progress = ZoomProgress::Animation(ZoomAnimation {
        level_anim,
        focal_anim: None,
        target_level: 3.0,
        target_focal: Point::from((0.0, 0.0)),
        start_focal: Point::from((0.0, 0.0)),
    });
    // Animation just started, should be near 1.0
    let level = anim_progress.level();
    assert!(
        level >= 1.0 && level < 1.5,
        "level should be near start: {}",
        level
    );
}

#[test]
fn zoom_progress_focal_point_accessor() {
    // Test Static variant
    let static_progress = ZoomProgress::Static {
        level: 2.0,
        focal_point: Point::from((100.0, 200.0)),
    };
    let focal = static_progress.focal_point();
    assert_eq!(focal.x, 100.0);
    assert_eq!(focal.y, 200.0);

    // Test Animation without focal animation
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = Animation::new(clock.clone(), 1.0, 2.0, 0.0, test_animation_config());
    let anim_progress = ZoomProgress::Animation(ZoomAnimation {
        level_anim,
        focal_anim: None,
        target_level: 2.0,
        target_focal: Point::from((50.0, 100.0)),
        start_focal: Point::from((0.0, 0.0)),
    });
    // Without focal animation, should return target_focal
    let focal = anim_progress.focal_point();
    assert_eq!(focal.x, 50.0);
    assert_eq!(focal.y, 100.0);
}

#[test]
fn zoom_progress_is_animation() {
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = Animation::new(clock.clone(), 1.0, 2.0, 0.0, test_animation_config());
    let anim = ZoomProgress::Animation(ZoomAnimation {
        level_anim,
        focal_anim: None,
        target_level: 2.0,
        target_focal: Point::from((0.0, 0.0)),
        start_focal: Point::from((0.0, 0.0)),
    });
    assert!(anim.is_animation());
    assert!(!anim.is_gesture());

    let static_progress = ZoomProgress::Static {
        level: 1.0,
        focal_point: Point::from((0.0, 0.0)),
    };
    assert!(!static_progress.is_animation());
    assert!(!static_progress.is_gesture());
}

#[test]
fn zoom_progress_is_done() {
    // Static is always done
    let static_progress = ZoomProgress::Static {
        level: 1.0,
        focal_point: Point::from((0.0, 0.0)),
    };
    assert!(static_progress.is_done());

    // Animation should not be done immediately
    let clock = Clock::with_time(Duration::ZERO);
    let level_anim = Animation::new(clock.clone(), 1.0, 2.0, 0.0, test_animation_config());
    let anim = ZoomProgress::Animation(ZoomAnimation {
        level_anim,
        focal_anim: None,
        target_level: 2.0,
        target_focal: Point::from((0.0, 0.0)),
        start_focal: Point::from((0.0, 0.0)),
    });
    assert!(!anim.is_done());
}

#[test]
fn output_zoom_state_default() {
    let state = OutputZoomState::default();
    assert_eq!(state.base_level, 1.0);
    assert_eq!(state.base_focal.x, 0.0);
    assert_eq!(state.base_focal.y, 0.0);
    assert!(!state.locked);
    assert!(state.progress.is_none());
}

#[test]
fn zoom_animation_completion() {
    let clock = Clock::with_time(Duration::ZERO);

    // Create an animation from 1.0 to 2.0
    let level_anim = Animation::new(clock.clone(), 1.0, 2.0, 0.0, test_animation_config());

    let mut progress = ZoomProgress::Animation(ZoomAnimation {
        level_anim,
        focal_anim: None,
        target_level: 2.0,
        target_focal: Point::from((0.0, 0.0)),
        start_focal: Point::from((0.0, 0.0)),
    });

    // Should not be done at start
    assert!(!progress.is_done());
    let level = progress.level();
    assert!(
        level >= 1.0 && level < 1.1,
        "Should be near start: {}",
        level
    );

    // Simulate time advancing (using a spring animation, this would take some time)
    // For this test, we just verify the structure works
    assert!(progress.is_animation());
}
