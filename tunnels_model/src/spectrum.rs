//! The audio spectrum laid out as one period of a waveform.

use std::fmt;

use tunnels_lib::audio::{AudioFrame, SPECTRUM_BANDS};
use tunnels_lib::number::{Phase, UnipolarFloat};

/// The index of the highest band.
const TOP_BAND: usize = SPECTRUM_BANDS - 1;

/// How many band widths one period spans: out from band 0 to the top band,
/// and back.
const BANDS_PER_PERIOD: f64 = (2 * TOP_BAND) as f64;

/// Entries of the smooth curve per band interval.
const ENTRIES_PER_BAND: usize = 32;

/// Entries of the smooth curve, from band 0 to the top band inclusive.
const SMOOTH_ENTRIES: usize = TOP_BAND * ENTRIES_PER_BAND + 1;

/// One audio frame's spectrum, as a unipolar shape over one period.
///
/// The period is folded: it runs out from band 0 at the seam to the top band
/// at the half period, then back, so its two ends meet on the same band.
///
/// Read stepped, each band is a flat step at its own level. A period holds
/// twice as many equal steps as there are band intervals; band 0 and the top
/// band each take one step, centred on the seam and on the half period, and
/// every other band takes one on the way out and one on the way back.
///
/// Read smooth, a Catmull-Rom curve passes through every band's level at the
/// centre of its step, meets itself level at each turn, and is held to
/// [0, 1].
#[derive(Clone)]
pub struct SpectrumTables {
    /// Each band's level, in ascending frequency order.
    bands: [f32; SPECTRUM_BANDS],
    /// The smooth curve over the out half of the period, sampled so that
    /// every band's level falls exactly on an entry.
    smooth: [f32; SMOOTH_ENTRIES],
}

impl SpectrumTables {
    /// No level in any band.
    pub const SILENT: Self = Self {
        bands: [0.0; SPECTRUM_BANDS],
        smooth: [0.0; SMOOTH_ENTRIES],
    };

    /// The shape of a frame's spectrum.
    pub fn new(frame: &AudioFrame) -> Self {
        Self::from_bands(frame.spectrum().map(|level| level.val() as f32))
    }

    /// The shape of a set of band levels, each in [0, 1].
    fn from_bands(bands: [f32; SPECTRUM_BANDS]) -> Self {
        let mut smooth = [0.0; SMOOTH_ENTRIES];
        for (i, entry) in smooth.iter_mut().enumerate() {
            let position = i as f64 / ENTRIES_PER_BAND as f64;
            *entry = catmull_rom(&bands, position).clamp(0.0, 1.0) as f32;
        }
        Self { bands, smooth }
    }

    /// The level at a phase of the period, blended from the stepped reading
    /// at a `smoothing` of zero to the smooth reading at one.
    #[inline]
    pub fn value(&self, phase: Phase, smoothing: UnipolarFloat) -> f64 {
        let position = band_position(phase);
        let mix = smoothing.val();
        self.stepped_at(position) * (1.0 - mix) + self.smooth_at(position) * mix
    }

    /// The level of the band nearest a position, in band units from band 0.
    ///
    /// A position halfway between two bands reads the higher one.
    #[inline]
    fn stepped_at(&self, position: f64) -> f64 {
        let band = (position.round() as usize).min(TOP_BAND);
        f64::from(self.bands[band])
    }

    /// The smooth curve at a position, in band units from band 0, read
    /// linearly between the two entries either side of it.
    #[inline]
    fn smooth_at(&self, position: f64) -> f64 {
        let scaled = position * ENTRIES_PER_BAND as f64;
        let below = (scaled as usize).min(SMOOTH_ENTRIES - 2);
        let t = scaled - below as f64;
        f64::from(self.smooth[below]) * (1.0 - t) + f64::from(self.smooth[below + 1]) * t
    }
}

/// Summarised: the levels are a frame's worth of data rather than state worth
/// reading in a dump.
impl fmt::Debug for SpectrumTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpectrumTables").finish_non_exhaustive()
    }
}

/// Where a phase falls along the bands, in band units from band 0: rising
/// over the first half of the period and falling back over the second.
#[inline]
fn band_position(phase: Phase) -> f64 {
    let x = phase.val();
    let out = if x < 0.5 { x } else { 1.0 - x };
    out * BANDS_PER_PERIOD
}

/// A band's level by an index that may lie past either end, mirrored back in
/// about the end it passed.
fn mirrored(bands: &[f32; SPECTRUM_BANDS], index: isize) -> f64 {
    let i = index.unsigned_abs();
    let i = if i > TOP_BAND { 2 * TOP_BAND - i } else { i };
    f64::from(bands[i.min(TOP_BAND)])
}

/// The uniform Catmull-Rom curve through the band levels at a position, in
/// band units from band 0, unclamped.
///
/// The levels are mirrored about each end, so the curve's slope there is
/// zero.
fn catmull_rom(bands: &[f32; SPECTRUM_BANDS], position: f64) -> f64 {
    let i = position.floor() as isize;
    let t = position - i as f64;
    let p0 = mirrored(bands, i - 1);
    let p1 = mirrored(bands, i);
    let p2 = mirrored(bands, i + 1);
    let p3 = mirrored(bands, i + 2);
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

#[cfg(test)]
mod test {
    use super::*;

    /// Levels that differ from band to band, so that every band is
    /// recognisable by its level.
    fn distinct() -> SpectrumTables {
        SpectrumTables::from_bands(std::array::from_fn(|b| {
            ((b * 7) % SPECTRUM_BANDS) as f32 / TOP_BAND as f32
        }))
    }

    /// How far either side of a step's edge a reading is taken.
    const EDGE: f64 = 1e-9;

    /// Read stepped, a period is fifty equal steps. Step `k` is centred on
    /// `k / 50`, so its edges are exactly halfway between, and it holds band
    /// `k` on the way out and band `50 - k` on the way back: band 0 straddles
    /// the seam and the top band straddles the half period, each as one step.
    #[test]
    fn a_stepped_period_is_fifty_equal_steps_folded_at_each_turn() {
        let tables = distinct();
        let steps = 2 * TOP_BAND;
        for k in 0..steps {
            let band = if k > TOP_BAND { steps - k } else { k };
            let level = f64::from(tables.bands[band]);
            let centre = k as f64 / steps as f64;
            let half_step = 0.5 / steps as f64;
            for x in [centre - half_step + EDGE, centre, centre + half_step - EDGE] {
                let read = tables.value(Phase::new(x), UnipolarFloat::ZERO);
                assert_eq!(
                    read, level,
                    "step {k} read {read} at phase {x}, where band {band} is {level}"
                );
            }
            // Just past the step's far edge is the next step, whose band
            // differs.
            let next = tables.value(Phase::new(centre + half_step + EDGE), UnipolarFloat::ZERO);
            assert_ne!(
                next,
                level,
                "step {k} ran on past phase {}",
                centre + half_step
            );
        }
    }

    /// Read smooth, the curve passes exactly through every band's level,
    /// reads the same either way out from a turn, and leaves each turn level.
    #[test]
    fn a_smooth_period_passes_through_each_band_and_turns_level() {
        let tables = distinct();
        for b in 0..SPECTRUM_BANDS {
            assert_eq!(
                tables.smooth_at(b as f64),
                f64::from(tables.bands[b]),
                "the smooth curve missed band {b}"
            );
        }

        for i in 1..1000 {
            let x = i as f64 / 2000.0;
            let out = tables.value(Phase::new(x), UnipolarFloat::ONE);
            let back = tables.value(Phase::new(1.0 - x), UnipolarFloat::ONE);
            assert!(
                (out - back).abs() < 1e-12,
                "phase {x} read {out} on the way out and {back} on the way back"
            );
        }

        // Leaving a turn, the curve moves at a small fraction of the pace of
        // the straight line to the next band.
        for (turn, next, first) in [(0, 1, 0), (TOP_BAND, TOP_BAND - 1, SMOOTH_ENTRIES - 1)] {
            let chord = f64::from(tables.bands[next] - tables.bands[turn]).abs();
            assert!(chord > 0.0, "bands {turn} and {next} are level already");
            let second = if first == 0 { 1 } else { first - 1 };
            let leaving = f64::from(tables.smooth[second] - tables.smooth[first]).abs();
            assert!(
                leaving < chord / ENTRIES_PER_BAND as f64 / 8.0,
                "the curve leaves band {turn} by {leaving} in its first entry, against \
                 a chord of {chord} across the whole band"
            );
        }
    }

    /// Catmull-Rom overshoots either side of a sharp plateau; the smooth
    /// reading is held to the unit range anyway.
    #[test]
    fn a_smooth_period_is_held_to_the_unit_range() {
        let bands: [f32; SPECTRUM_BANDS] =
            std::array::from_fn(|b| if matches!(b % 4, 1 | 2) { 1.0 } else { 0.0 });
        let positions = (0..=TOP_BAND * 64).map(|i| i as f64 / 64.0);
        let raw = positions.clone().map(|p| catmull_rom(&bands, p));
        assert!(
            raw.clone().any(|v| v < 0.0) && raw.clone().any(|v| v > 1.0),
            "the curve stayed in range unaided, so the clamp is untested"
        );
        let tables = SpectrumTables::from_bands(bands);
        for p in positions {
            let read = tables.smooth_at(p);
            assert!(
                (0.0..=1.0).contains(&read),
                "the smooth curve read {read} at band position {p}"
            );
        }
    }

    /// Smoothing blends linearly from the stepped reading to the smooth one,
    /// arriving exactly at each.
    #[test]
    fn smoothing_blends_from_stepped_to_smooth() {
        let tables = distinct();
        for i in 0..500 {
            let phase = Phase::new(i as f64 / 500.0 + 0.0007);
            let position = band_position(phase);
            let (stepped, smooth) = (tables.stepped_at(position), tables.smooth_at(position));
            let at = |s: f64| tables.value(phase, UnipolarFloat::new(s));
            assert_eq!(at(0.0), stepped, "phase {phase:?} at no smoothing");
            assert_eq!(at(1.0), smooth, "phase {phase:?} at full smoothing");
            assert!(
                (at(0.5) - (stepped + smooth) / 2.0).abs() < 1e-12,
                "phase {phase:?} at half smoothing read {} between {stepped} and {smooth}",
                at(0.5)
            );
        }
    }
}
