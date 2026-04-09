//! Perceptual (logarithmic) scaling for audio amplitude values.

/// Convert linear amplitude to perceptual (dB) scale.
///
/// Maps 0 dB (full scale) to 1.0 and -`range_db` to 0.0.
/// Values above 0 dB map to >1.0 (not clamped). Values below
/// the floor map to 0.0.
pub fn linear_to_perceptual(linear: f32, range_db: f32) -> f32 {
    const FLOOR: f32 = 1e-6;
    if linear <= FLOOR {
        return 0.0;
    }
    let db = 20.0 * linear.log10();
    ((db + range_db) / range_db).max(0.0)
}

/// Default dynamic range for the perceptual transform.
/// 20 dB maps the top 20 dB of dynamic range to [0, 1].
/// Anything below -20 dBFS (~0.1 linear) maps to 0.
pub const DEFAULT_RANGE_DB: f32 = 20.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_maps_to_one() {
        let result = linear_to_perceptual(1.0, 60.0);
        assert!((result - 1.0).abs() < 1e-6);
    }

    #[test]
    fn silence_maps_to_zero() {
        assert_eq!(linear_to_perceptual(0.0, 60.0), 0.0);
        assert_eq!(linear_to_perceptual(1e-7, 60.0), 0.0);
    }

    #[test]
    fn minus_60db_maps_to_zero() {
        // -60 dB = 0.001 linear
        let result = linear_to_perceptual(0.001, 60.0);
        assert!(result.abs() < 1e-4);
    }

    #[test]
    fn minus_30db_maps_to_half() {
        // -30 dB = ~0.0316 linear, should map to 0.5 with 60 dB range
        let result = linear_to_perceptual(0.0316227766, 60.0);
        assert!((result - 0.5).abs() < 0.01);
    }

    #[test]
    fn above_full_scale_exceeds_one() {
        let result = linear_to_perceptual(2.0, 60.0);
        // 2.0 linear = +6 dB, maps to (6+60)/60 = 1.1
        assert!(result > 1.0, "Values above 0 dB should map above 1.0, got {result}");
    }

    #[test]
    fn narrower_range_compresses() {
        // With 40 dB range, -20 dB (0.1 linear) should map to 0.5
        let result = linear_to_perceptual(0.1, 40.0);
        assert!((result - 0.5).abs() < 0.01);
    }
}
