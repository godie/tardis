//! Pure-logic audio utilities.
//!
//! These helpers know nothing about CPAL, the audio thread, or real
//! microphones so they can be exercised directly by unit tests in
//! `tests` below.

/// Average absolute amplitude across `samples`.
/// Returns `0.0` for empty input so callers can invoke it unconditionally.
pub fn calculate_average_volume(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for &sample in samples {
        sum += sample.abs();
    }
    sum / samples.len() as f32
}

/// Is `volume` strictly above `threshold`? Equal-to-threshold returns false.
pub fn is_above_threshold(volume: f32, threshold: f32) -> bool {
    volume > threshold
}

/// Convert an i16 PCM sample to a f32 in approximately `[-1.0, 1.0]`.
/// Silence is `0.0`; full-scale positive is near `1.0`; full-scale negative
/// is `-1.0`. Divisor is `32768` so the range is symmetric.
#[allow(dead_code)] // used by tests + reserved for future callers
pub fn normalize_sample_i16(sample: i16) -> f32 {
    sample as f32 / 32768.0
}

/// Convert a u16 PCM sample to a f32 centered around `0.0`.
/// The midpoint `32768` maps to `~0.0`; full-scale clips to roughly `±1.0`.
#[allow(dead_code)] // used by tests + reserved for future callers
pub fn normalize_sample_u16(sample: u16) -> f32 {
    (sample as f32 - 32768.0) / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance for floating-point comparisons. Comfortably larger than
    /// `32767/32768 - 1` (≈ 3e-5) to keep tests robust across platforms.
    const EPS: f32 = 0.001;

    fn assert_approx_eq(actual: f32, expected: f32, epsilon: f32) {
        let diff = (actual - expected).abs();
        assert!(
            diff <= epsilon,
            "actual: {actual}, expected: {expected}, |diff|: {diff}"
        );
    }

    // ---- calculate_average_volume --------------------------------------

    #[test]
    fn empty_samples_return_zero() {
        assert_approx_eq(calculate_average_volume(&[]), 0.0, EPS);
    }

    #[test]
    fn silent_samples_return_zero() {
        let samples = [0.0_f32, 0.0, 0.0];
        assert_approx_eq(calculate_average_volume(&samples), 0.0, EPS);
    }

    #[test]
    fn average_uses_absolute_values() {
        // Without abs the result would be 0.0; with abs it must be 1.0.
        let samples = [1.0_f32, -1.0];
        assert_approx_eq(calculate_average_volume(&samples), 1.0, EPS);
        let samples = [3.0_f32, -3.0, 3.0];
        assert_approx_eq(calculate_average_volume(&samples), 3.0, EPS);
    }

    #[test]
    fn positive_and_negative_values_balance() {
        let samples = [0.5_f32, -0.5, 0.5, -0.5];
        assert_approx_eq(calculate_average_volume(&samples), 0.5, EPS);
    }

    #[test]
    fn averages_preserve_larger_amplitude() {
        let samples = [-2.0_f32, 2.0];
        assert_approx_eq(calculate_average_volume(&samples), 2.0, EPS);
    }

    // ---- is_above_threshold --------------------------------------------

    #[test]
    fn below_threshold_returns_false() {
        assert!(!is_above_threshold(0.005, 0.01));
    }

    #[test]
    fn equal_to_threshold_returns_false() {
        // Spec: `volume > threshold` is the only true case.
        assert!(!is_above_threshold(0.01, 0.01));
    }

    #[test]
    fn above_threshold_returns_true() {
        assert!(is_above_threshold(0.011, 0.01));
    }

    // ---- normalize_sample_i16 ------------------------------------------

    #[test]
    fn i16_zero_maps_to_zero() {
        assert_approx_eq(normalize_sample_i16(0), 0.0, EPS);
    }

    #[test]
    fn i16_max_maps_near_one() {
        assert_approx_eq(normalize_sample_i16(i16::MAX), 1.0, EPS);
    }

    #[test]
    fn i16_min_maps_near_minus_one() {
        assert_approx_eq(normalize_sample_i16(i16::MIN), -1.0, EPS);
    }

    // ---- normalize_sample_u16 ------------------------------------------

    #[test]
    fn u16_midpoint_maps_near_zero() {
        assert_approx_eq(normalize_sample_u16(32768), 0.0, EPS);
    }

    #[test]
    fn u16_max_maps_near_one() {
        assert_approx_eq(normalize_sample_u16(u16::MAX), 1.0, EPS);
    }

    #[test]
    fn u16_min_maps_near_minus_one() {
        assert_approx_eq(normalize_sample_u16(u16::MIN), -1.0, EPS);
    }
}
