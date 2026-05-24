use super::*;

/// Rubber-banding is smooth, not a hard clamp — levels within [min, max]
/// should pass through approximately unchanged.
#[test]
fn rubber_band_identity_within_bounds() {
    let level = clamp_zoom_level_with_rubber_band(2.0, 1.0, 10.0);
    assert!(
        (level - 2.0).abs() < 1e-6,
        "level within bounds should pass through, got {level}"
    );
}

/// Levels far below the minimum get pulled up toward it by the rubber band
/// (smooth transition, not a hard clamp).
#[test]
fn rubber_band_pulls_up_below_min() {
    let level = clamp_zoom_level_with_rubber_band(0.001, 1.0, 10.0);
    // The rubber band pulls 0.001 up toward 1.0, but doesn't hard-clamp.
    // Verify it moved significantly toward the boundary.
    assert!(
        level > 0.001,
        "level {level} should be pulled above original 0.001"
    );
    assert!(
        level < 1.0 + 0.1,
        "level {level} should approach but not exceed min 1.0"
    );
}

/// Levels far above the maximum get pulled down toward it by the rubber band
/// (smooth transition, not a hard clamp).
#[test]
fn rubber_band_pulls_down_above_max() {
    let level = clamp_zoom_level_with_rubber_band(100.0, 1.0, 10.0);
    // The rubber band pulls 100 down toward 10.0, but doesn't hard-clamp.
    assert!(
        level < 100.0,
        "level {level} should be pulled below original 100.0"
    );
    assert!(level > 9.0, "level {level} should be near max 10.0");
}

/// Level exactly at the minimum boundary passes through unchanged.
#[test]
fn rubber_band_at_min_boundary() {
    let level = clamp_zoom_level_with_rubber_band(1.0, 1.0, 10.0);
    assert!(
        (level - 1.0).abs() < 1e-9,
        "level at min should pass through"
    );
}

/// Level exactly at the maximum boundary passes through unchanged.
#[test]
fn rubber_band_at_max_boundary() {
    let level = clamp_zoom_level_with_rubber_band(10.0, 1.0, 10.0);
    assert!(
        (level - 10.0).abs() < 1e-9,
        "level at max should pass through"
    );
}

/// log_pos = 0 should return the start level unchanged.
#[test]
fn log_pos_zero_returns_start() {
    assert!((log_pos_to_zoom_level(2.5, 0.0) - 2.5).abs() < 1e-9);
}

/// Positive log_pos increases the level exponentially.
#[test]
fn log_pos_positive_increases_level() {
    let level = log_pos_to_zoom_level(1.0, 2.0_f64.ln());
    assert!(
        (level - 2.0).abs() < 1e-9,
        "ln(2) from 1.0 should give 2.0, got {level}"
    );
}

/// Negative log_pos decreases the level exponentially.
#[test]
fn log_pos_negative_decreases_level() {
    let level = log_pos_to_zoom_level(2.0, 0.5_f64.ln());
    assert!(
        (level - 1.0).abs() < 1e-9,
        "ln(0.5) from 2.0 should give 1.0, got {level}"
    );
}
