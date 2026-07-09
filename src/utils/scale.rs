//! Default monitor scale calculation.
//!
//! This module follows logic and tests from Mutter:
//! <https://gitlab.gnome.org/GNOME/mutter/-/blob/gnome-46/src/backends/meta-monitor.c>

use smithay::utils::{Physical, Raw, Size};

const MIN_SCALE: i32 = 1;
const MAX_SCALE: i32 = 4;
const STEPS: i32 = 4;
const MIN_LOGICAL_AREA: i32 = 800 * 480;

const MOBILE_TARGET_DPI: f64 = 135.;
const LARGE_TARGET_DPI: f64 = 110.;
const LARGE_MIN_SIZE_INCHES: f64 = 20.;

/// Calculates the ideal scale for a monitor.
pub fn guess_monitor_scale(size_mm: Size<i32, Raw>, resolution: Size<i32, Physical>) -> f64 {
    if size_mm.w == 0 || size_mm.h == 0 {
        return 1.;
    }

    let diag_inches = f64::from(size_mm.w * size_mm.w + size_mm.h * size_mm.h).sqrt() / 25.4;

    let target_dpi = if diag_inches < LARGE_MIN_SIZE_INCHES {
        MOBILE_TARGET_DPI
    } else {
        LARGE_TARGET_DPI
    };

    let physical_dpi =
        f64::from(resolution.w * resolution.w + resolution.h * resolution.h).sqrt() / diag_inches;
    let perfect_scale = physical_dpi / target_dpi;

    supported_scales(resolution)
        .map(|scale| (scale, (scale - perfect_scale).abs()))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map_or(1., |(scale, _)| scale)
}

pub fn supported_scales(resolution: Size<i32, Physical>) -> impl Iterator<Item = f64> {
    (MIN_SCALE * STEPS..=MAX_SCALE * STEPS)
        .map(|x| f64::from(x) / f64::from(STEPS))
        .filter(move |scale| is_valid_for_resolution(resolution, *scale))
}

fn is_valid_for_resolution(resolution: Size<i32, Physical>, scale: f64) -> bool {
    let logical = resolution.to_f64().to_logical(scale).to_i32_round::<i32>();
    logical.w * logical.h >= MIN_LOGICAL_AREA
}

/// Adjusts the scale to the closest exactly-representable value.
pub fn closest_representable_scale(scale: f64) -> f64 {
    // Current fractional-scale Wayland protocol can only represent N / 120 scales.
    const FRACTIONAL_SCALE_DENOM: f64 = 120.;

    (scale * FRACTIONAL_SCALE_DENOM).round() / FRACTIONAL_SCALE_DENOM
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    fn check(size_mm: (i32, i32), resolution: (i32, i32)) -> f64 {
        guess_monitor_scale(Size::from(size_mm), Size::from(resolution))
    }

    #[test]
    fn test_guess_monitor_scale() {
        // Librem 5; not enough logical area when scaled
        assert_snapshot!(check((65, 129), (720, 1440)), @"1.5");
        // OnePlus 6
        assert_snapshot!(check((68, 144), (1080, 2280)), @"2.5");
        // Google Pixel 6a
        assert_snapshot!(check((64, 142), (1080, 2400)), @"2.5");
        // 13" MacBook Retina
        assert_snapshot!(check((286, 179), (2560, 1600)), @"1.75");
        // Surface Laptop Studio
        assert_snapshot!(check((303, 202), (2400, 1600)), @"1.5");
        // Dell XPS 9320
        assert_snapshot!(check((290, 180), (3840, 2400)), @"2.5");
        // Lenovo ThinkPad X1 Yoga Gen 6
        assert_snapshot!(check((300, 190), (3840, 2400)), @"2.5");
        // Generic 23" 1080p
        assert_snapshot!(check((509, 286), (1920, 1080)), @"1");
        // Generic 23" 4K
        assert_snapshot!(check((509, 286), (3840, 2160)), @"1.75");
        // Generic 27" 4K
        assert_snapshot!(check((598, 336), (3840, 2160)), @"1.5");
        // Generic 32" 4K
        assert_snapshot!(check((708, 398), (3840, 2160)), @"1.25");
        // Generic 25" 4K; ideal scale is 1.60, should round to 1.5 and 1.0
        assert_snapshot!(check((554, 312), (3840, 2160)), @"1.5");
        // Generic 23.5" 4K; ideal scale is 1.70, should round to 1.75 and 2.0
        assert_snapshot!(check((522, 294), (3840, 2160)), @"1.75");
        // Lenovo Legion 7 Gen 7 AMD 16"
        assert_snapshot!(check((340, 210), (2560, 1600)), @"1.5");
        // Acer Nitro XV320QU LV 31.5"
        assert_snapshot!(check((700, 390), (2560, 1440)), @"1");
        // Surface Pro 6
        assert_snapshot!(check((260, 170), (2736, 1824)), @"2");
    }

    #[test]
    fn guess_monitor_scale_unknown_size() {
        assert_eq!(check((0, 0), (1920, 1080)), 1.);
    }

    #[test]
    fn test_round_scale() {
        assert_snapshot!(closest_representable_scale(1.3), @"1.3");
        assert_snapshot!(closest_representable_scale(1.31), @"1.3083333333333333");
        assert_snapshot!(closest_representable_scale(1.32), @"1.3166666666666667");
        assert_snapshot!(closest_representable_scale(1.33), @"1.3333333333333333");
        assert_snapshot!(closest_representable_scale(1.34), @"1.3416666666666666");
        assert_snapshot!(closest_representable_scale(1.35), @"1.35");
    }
}
