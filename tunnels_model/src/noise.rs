//! Two-dimensional simplex noise in `f32`, built to be evaluated in batches.
//!
//! The field is Perlin's 2D simplex noise with the radial kernel `t²(2 + t²)`,
//! `t = 1 − 2r²`, and eight gradients: the four axes and the four unit
//! diagonals. Each lattice cell picks its gradient from the top three bits of an
//! integer hash, with no table, so the field does not repeat within 2³² cells
//! in either direction. Its values lie in about ±0.99.
//!
//! The field makes every choice by a mask or a select rather than a branch, and
//! is plain `f32` arithmetic, with no fused multiply-add and no library
//! function beyond a floor. Every operation is exactly rounded, so the same
//! inputs give the same bits on every IEEE-754 machine, and a loop over points
//! vectorises.
//!
//! `f32` holds only small coordinates accurately. A [`NoiseBase`] therefore
//! carries the large part of a position, resolved once in `f64` into a lattice
//! cell and small remainders, and points are given as `f32` offsets from it.
//! Within 128 of its base, a point's value is within about 5e-5 of the value at
//! that point resolved exactly; the error grows in proportion to the offset.

/// Skew from input space to the simplex lattice: `(√3 − 1) / 2`.
const SKEW_64: f64 = 0.366_025_403_784_438_6;
/// Unskew from the simplex lattice to input space: `(3 − √3) / 6`.
const UNSKEW_64: f64 = 0.211_324_865_405_187_1;
const SKEW: f32 = SKEW_64 as f32;
const UNSKEW: f32 = UNSKEW_64 as f32;
const TWO_UNSKEW: f32 = (2.0 * UNSKEW_64) as f32;

/// Odd multipliers that spread a cell's two coordinates across 32 bits.
const HASH_I: u32 = 0x8da6_b343;
const HASH_J: u32 = 0xd816_3841;

const SIGN: u32 = 0x8000_0000;

/// The `lowbias32` finaliser: two multiply-xorshift rounds.
///
/// A cell's hash is this over its two coordinates, combined by the odd
/// multipliers. Arithmetic wraps, so the hash is defined for every cell and
/// repeats only with a period of 2³² in each coordinate.
#[inline(always)]
fn mix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// A position in the noise field, resolved for `f32` offsets from it.
///
/// Holds the lattice cell the position falls in and the position's small
/// remainders within that cell, so a point near it is evaluated with `f32`
/// arithmetic on small numbers however far the position itself is from the
/// origin. Cells wrap at 2³², as the hash does.
#[derive(Clone, Copy, Debug)]
pub struct NoiseBase {
    /// The cell's coordinates, multiplied into the hash.
    hash_i: u32,
    hash_j: u32,
    /// The position's skewed coordinates less the cell's: each in [0, 1].
    skewed: [f32; 2],
    /// The position less the cell's corner, in input space.
    unskewed: [f32; 2],
}

impl NoiseBase {
    /// The base at `(x, y)`.
    ///
    /// The position is resolved in `f64`, so it is exact to `f64`'s precision
    /// at that magnitude.
    pub fn new(x: f64, y: f64) -> Self {
        let skew = (x + y) * SKEW_64;
        let (a, b) = (x + skew, y + skew);
        let (i, j) = (a.floor(), b.floor());
        let unskew = (i + j) * UNSKEW_64;
        Self {
            hash_i: wrap_to_u32(i).wrapping_mul(HASH_I),
            hash_j: wrap_to_u32(j).wrapping_mul(HASH_J),
            skewed: [(a - i) as f32, (b - j) as f32],
            unskewed: [(x - (i - unskew)) as f32, (y - (j - unskew)) as f32],
        }
    }

    /// The noise field at this base plus `(x, y)`, in about ±0.99.
    ///
    /// Within about 5e-5 of the field at that point resolved exactly while the
    /// offset is within 128, and in proportion to the offset beyond.
    #[inline(always)]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        // Skewed coordinates relative to the base's cell, and the cell the point
        // falls in, as an offset from that one.
        let skew = (x + y) * SKEW;
        let lx = self.skewed[0] + (x + skew);
        let ly = self.skewed[1] + (y + skew);
        let di = lx.floor();
        let dj = ly.floor();
        // The point less the cell's corner, in input space.
        let unskew = (di + dj) * UNSKEW;
        let x0 = (self.unskewed[0] + x) - (di - unskew);
        let y0 = (self.unskewed[1] + y) - (dj - unskew);

        // The middle corner is one step along whichever skewed axis the point is
        // further along: x where `lower` is all ones, y where it is all zeros.
        let lower = sign_mask((y0 - x0).to_bits());
        let step_i = (lower & 1) as f32;
        let x1 = (x0 - step_i) + UNSKEW;
        let y1 = (y0 - (1.0 - step_i)) + UNSKEW;
        let x2 = (x0 - 1.0) + TWO_UNSKEW;
        let y2 = (y0 - 1.0) + TWO_UNSKEW;

        // The three corners' hash inputs. A step of one cell adds the multiplier,
        // so only the first corner multiplies.
        let hi = self
            .hash_i
            .wrapping_add(integral_bits(di).wrapping_mul(HASH_I));
        let hj = self
            .hash_j
            .wrapping_add(integral_bits(dj).wrapping_mul(HASH_J));
        let middle_i = HASH_I & lower;
        let middle_j = HASH_J & !lower;
        let h0 = mix(hi ^ hj);
        let h1 = mix(hi.wrapping_add(middle_i) ^ hj.wrapping_add(middle_j));
        let h2 = mix(hi.wrapping_add(HASH_I) ^ hj.wrapping_add(HASH_J));

        surflet(h0, x0, y0) + surflet(h1, x1, y1) + surflet(h2, x2, y2)
    }
}

/// An integral `f64` reduced modulo 2³².
fn wrap_to_u32(v: f64) -> u32 {
    v as i64 as u32
}

/// A whole number of magnitude below 2²², as a two's-complement integer.
///
/// Adding 1.5 × 2²³ puts the number in a binade whose spacing is one, so its
/// low mantissa bits are the integer, exactly. Any other input gives some
/// integer rather than a saturated one, which keeps the conversion to a few
/// lane-wise operations.
#[inline(always)]
fn integral_bits(v: f32) -> u32 {
    (v + 12_582_912.0).to_bits().wrapping_sub(0x4b40_0000)
}

/// All ones if `v`'s top bit is set, else all zeros.
#[inline(always)]
fn sign_mask(v: u32) -> u32 {
    ((v as i32) >> 31) as u32
}

/// The bits of `a` where `mask` is set and of `b` where it is clear.
#[inline(always)]
fn blend(mask: u32, a: u32, b: u32) -> u32 {
    b ^ ((a ^ b) & mask)
}

/// One corner's contribution at offset `(x, y)` from it.
#[inline(always)]
fn surflet(hash: u32, x: f32, y: f32) -> f32 {
    let t = 1.0 - 2.0 * (x * x + y * y);
    let t = if t > 0.0 { t } else { 0.0 };
    let t2 = t * t;
    t2 * (t2 + 2.0) * gradient_dot(hash, x, y)
}

/// The dot product of `(x, y)` with the gradient the hash's top three bits
/// pick, by sign flips and selects only.
///
/// Bit 31 picks a diagonal over an axis. For an axis, bit 30 picks y over x
/// and bit 29 negates it. For a diagonal, bit 29 negates x and bit 30 negates
/// y, and the sum is scaled to unit length.
#[inline(always)]
fn gradient_dot(hash: u32, x: f32, y: f32) -> f32 {
    let flip_x = (hash << 2) & SIGN;
    let flip_y = (hash << 1) & SIGN;
    let fx = x.to_bits() ^ flip_x;
    let axis = blend(sign_mask(hash << 1), y.to_bits() ^ flip_x, fx);
    let diag = std::f32::consts::FRAC_1_SQRT_2
        * (f32::from_bits(fx) + f32::from_bits(y.to_bits() ^ flip_y));
    let diagonal = hash & SIGN != 0;
    f32::from_bits(std::hint::select_unpredictable(
        diagonal,
        diag.to_bits(),
        axis,
    ))
}

/// Where a noise pulse stops being dark, as a magnitude of the noise.
///
/// The magnitude of simplex noise is close to uniform across the lower part of
/// its range, so where this knee sits in that range is directly how much of the
/// time a pulse rests: about a seventh of the way up it, and so dark about a
/// seventh of the time.
pub const GATE_LOW: f32 = 0.09;

/// Where a noise pulse reaches full, as a magnitude of the noise.
///
/// Placed so that about the top twentieth of noise magnitudes saturate. Noise
/// approaches the end of its own range too rarely to arrive at full any other
/// way, and the flat top that arriving costs is worth no more of the period
/// than it takes.
pub const GATE_HIGH: f32 = 0.71;

/// Shape bipolar noise into a unipolar pulse.
///
/// Noise reaches the ends of its own range only rarely, so a curve that merely
/// rescales it settles into a dim middle: never resolving to black, never
/// arriving at full. The gate is flat at both ends instead — dark below one
/// knee, full above the other — which is what buys a pulse that starts and
/// ends at rest and a peak that lands rather than approaches.
///
/// Between the knees it is smoothstepped, so brightness enters and leaves a
/// pulse with no kink. The dark zone also holds the fold that taking a
/// magnitude puts at zero, keeping that corner off the sloped part of the
/// curve.
#[inline(always)]
pub fn gate(v: f32) -> f32 {
    let t = ((v.abs() - GATE_LOW) / (GATE_HIGH - GATE_LOW)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod test {
    use super::*;

    /// The field reads what a plain `f64` evaluation reads, near and far from
    /// the origin and anywhere within 128 of its base. So the hash it computes
    /// incrementally for a cell's three corners is each corner's own, across all
    /// the cells an offset can cross, and `f32` carries the offset accurately.
    #[test]
    fn the_field_matches_a_plain_evaluation_within_128_of_its_base() {
        for (x, y) in [(0.3, 0.1), (-7.25, 3.5), (1e6 + 0.37, 12.0), (-3e9, 2e9)] {
            let base = NoiseBase::new(x, y);
            for k in 0..4096 {
                let (dx, dy) = (
                    (k as f32 * 0.173) % 180.0 - 90.0,
                    (k as f32 * 0.091) % 180.0 - 90.0,
                );
                let direct = direct_field(x + f64::from(dx), y + f64::from(dy));
                let v = base.sample(dx, dy);
                assert!(
                    (f64::from(v) - direct).abs() < 1e-4,
                    "field at ({x}, {y}) + ({dx}, {dy}) read {v}, directly {direct}"
                );
            }
        }
    }

    /// The field evaluated plainly in `f64`, hashing each corner's cell.
    fn direct_field(x: f64, y: f64) -> f64 {
        let skew = (x + y) * SKEW_64;
        let (i, j) = ((x + skew).floor(), (y + skew).floor());
        let unskew = (i + j) * UNSKEW_64;
        let (x0, y0) = (x - (i - unskew), y - (j - unskew));
        let (oi, oj) = if x0 > y0 { (1.0, 0.0) } else { (0.0, 1.0) };
        let corners = [
            (i, j, x0, y0),
            (i + oi, j + oj, x0 - oi + UNSKEW_64, y0 - oj + UNSKEW_64),
            (
                i + 1.0,
                j + 1.0,
                x0 - 1.0 + 2.0 * UNSKEW_64,
                y0 - 1.0 + 2.0 * UNSKEW_64,
            ),
        ];
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let gradients = [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (d, d),
            (-d, d),
            (d, -d),
            (-d, -d),
        ];
        corners
            .iter()
            .map(|&(ci, cj, px, py)| {
                let h = mix(
                    wrap_to_u32(ci).wrapping_mul(HASH_I) ^ wrap_to_u32(cj).wrapping_mul(HASH_J)
                );
                let (gx, gy) = gradients[(h >> 29) as usize];
                let t = (1.0 - 2.0 * (px * px + py * py)).max(0.0);
                t * t * (t * t + 2.0) * (px * gx + py * gy)
            })
            .sum()
    }

    /// A gated noise pulse rests at each end of the unipolar range rather than
    /// only approaching it, so the two knees are where it arrives, and outside
    /// them it holds still however far the noise goes on.
    #[test]
    fn a_noise_gate_rests_at_both_ends_of_its_range() {
        for v in [0.0, GATE_LOW / 2.0, GATE_LOW, -GATE_LOW] {
            assert_eq!(
                gate(v),
                0.0,
                "noise of {v} lit a pulse from inside the dark zone"
            );
        }
        let above = (GATE_HIGH + 1.0) / 2.0;
        for v in [GATE_HIGH, -GATE_HIGH, above, -above, 1.0, -1.0] {
            assert_eq!(
                gate(v),
                1.0,
                "noise of {v} fell short of full from above the high knee"
            );
        }
    }

    /// The gate reads a magnitude, so noise displaced either way lights a pulse
    /// the same amount, and a pulse rises without pause between its knees.
    #[test]
    fn a_noise_gate_is_symmetric_and_rises_between_its_knees() {
        let step = (GATE_HIGH - GATE_LOW) / 64.0;
        let mut previous = 0.0;
        for i in 1..64 {
            let v = GATE_LOW + step * i as f32;
            let value = gate(v);
            assert_eq!(
                value,
                gate(-v),
                "noise of {v} and of its negation lit a pulse differently"
            );
            assert!(
                value > previous,
                "a pulse stalled at {value} between its knees, at noise of {v}"
            );
            assert!(
                (0.0..=1.0).contains(&value),
                "noise of {v} lit a pulse to {value}, outside the unipolar range"
            );
            previous = value;
        }
    }

    /// A pulse leaves and reaches rest smoothly, so brightness has no kink at
    /// either knee where a viewer would read it as a corner in the light.
    #[test]
    fn a_noise_gate_has_no_kink_at_either_knee() {
        let span = GATE_HIGH - GATE_LOW;
        let step = span / 4096.0;
        // The slope just inside a knee, against the slope of a straight ramp
        // between the knees, which is what the gate would have if it did not
        // ease in and out.
        let ramp = step / span;
        for knee in [GATE_LOW, GATE_HIGH] {
            let inside = if knee == GATE_LOW { step } else { -step };
            let slope = (gate(knee + inside) - gate(knee)).abs();
            assert!(
                slope < ramp / 100.0,
                "a pulse turned a corner at the {knee} knee: it moved {slope} \
                 over a step a straight ramp would move {ramp}"
            );
        }
    }
}
