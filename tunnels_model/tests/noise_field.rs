//! The noise field's statistical properties, on fixed probes.
//!
//! Exact outputs are pinned by one digest, so a change that keeps the
//! properties can still be seen, and a platform that computes the field
//! differently fails it.

use tunnels_model::noise::{GATE_HIGH, GATE_LOW, NoiseBase};

/// The field at a large origin plus a small offset.
fn field(origin: [f64; 2], offset: [f64; 2]) -> f64 {
    let base = NoiseBase::new(origin[0], origin[1]);
    f64::from(base.sample(offset[0] as f32, offset[1] as f32))
}

/// splitmix64: a fixed, portable stream of probe coordinates.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), with 53 bits.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [0, 1), with 24 bits, so exact in `f32`.
    fn unit_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [lo, hi).
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// A probe point: an origin far out in the field, and an offset near it.
    fn scattered(&mut self) -> ([f64; 2], [f64; 2]) {
        let origin = [self.range(0.0, 1e6), self.range(0.0, 4096.0)];
        let offset = [self.range(0.0, 64.0), self.range(0.0, 64.0)];
        (origin, offset)
    }
}

/// Pearson's correlation of two equally long series.
fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    sab / (saa * sbb).sqrt()
}

/// About a seventh of magnitudes lie below a pulse's low knee and about a
/// twentieth above its high one, and every value lies in [−1, 1], including at
/// the highest peak a climb from the largest value seen can find.
#[test]
fn magnitudes_split_at_the_gate_knees_and_stay_in_range() {
    const POINTS: usize = 200_000;
    let mut rng = Rng(1);
    let (mut below, mut above) = (0usize, 0usize);
    let (mut peak, mut peak_at) = (0.0f64, ([0.0; 2], [0.0; 2]));
    for _ in 0..POINTS {
        let (origin, offset) = rng.scattered();
        let v = field(origin, offset).abs();
        below += usize::from(v < f64::from(GATE_LOW));
        above += usize::from(v > f64::from(GATE_HIGH));
        if v > peak {
            (peak, peak_at) = (v, (origin, offset));
        }
    }
    let below = below as f64 / POINTS as f64;
    let above = above as f64 / POINTS as f64;
    assert!(
        (below - 1.0 / 7.0).abs() < 0.01,
        "{below} of magnitudes lay below {GATE_LOW}, not about a seventh"
    );
    assert!(
        (above - 1.0 / 20.0).abs() < 0.005,
        "{above} of magnitudes lay above {GATE_HIGH}, not about a twentieth"
    );

    let (origin, mut at) = peak_at;
    let mut step = 1e-2;
    const DIRECTIONS: [[f64; 2]; 8] = [
        [1.0, 0.0],
        [-1.0, 0.0],
        [0.0, 1.0],
        [0.0, -1.0],
        [0.7, 0.7],
        [-0.7, 0.7],
        [0.7, -0.7],
        [-0.7, -0.7],
    ];
    while step > 1e-7 {
        let mut moved = false;
        for d in DIRECTIONS {
            let next = [at[0] + d[0] * step, at[1] + d[1] * step];
            let v = field(origin, next).abs();
            if v > peak {
                (peak, at, moved) = (v, next, true);
            }
        }
        if !moved {
            step /= 2.0;
        }
    }
    assert!(peak <= 1.0, "the field climbed to a magnitude of {peak}");
}

/// Neighbouring samples along fine lines differ by no more than a bounded
/// slope allows, near and far from the origin.
#[test]
fn the_field_is_continuous() {
    for h in [1e-3, 1e-5] {
        let mut rng = Rng(2);
        let mut slope = 0.0f64;
        for _ in 0..64 {
            let (origin, start) = rng.scattered();
            let angle = rng.range(0.0, std::f64::consts::TAU);
            let (dx, dy) = (angle.cos() * h, angle.sin() * h);
            let mut previous = field(origin, start);
            for k in 1..=4096 {
                let v = field(
                    origin,
                    [start[0] + dx * f64::from(k), start[1] + dy * f64::from(k)],
                );
                slope = slope.max((v - previous).abs() / h);
                previous = v;
            }
        }
        assert!(
            slope < 5.0,
            "the field stepped at a slope of {slope} over steps of {h}"
        );
    }
}

/// Features are the size this kernel gives them, alike in every direction: the
/// field's correlation with itself falls to one half at the same distance
/// whichever way it is displaced.
#[test]
fn features_are_the_kernels_size_in_every_direction() {
    const HALF_LENGTH: f64 = 0.27;
    const DISTANCES: [f64; 13] = [
        0.20, 0.21, 0.22, 0.23, 0.24, 0.25, 0.26, 0.27, 0.28, 0.29, 0.30, 0.31, 0.32,
    ];
    let mut rng = Rng(3);
    let probes: Vec<_> = (0..40_000).map(|_| rng.scattered()).collect();
    let here: Vec<f64> = probes.iter().map(|&(o, p)| field(o, p)).collect();
    let lengths: Vec<f64> = (0..12)
        .map(|k| {
            let angle = f64::from(15 * k).to_radians();
            let correlations = DISTANCES.map(|d| {
                let there: Vec<f64> = probes
                    .iter()
                    .map(|&(o, p)| field(o, [p[0] + d * angle.cos(), p[1] + d * angle.sin()]))
                    .collect();
                correlation(&here, &there)
            });
            let crossing = correlations
                .windows(2)
                .position(|r| r[0] >= 0.5 && r[1] < 0.5)
                .unwrap_or_else(|| {
                    panic!("correlations at {angle} never crossed one half: {correlations:?}")
                });
            let (r0, r1) = (correlations[crossing], correlations[crossing + 1]);
            let (d0, d1) = (DISTANCES[crossing], DISTANCES[crossing + 1]);
            d0 + (d1 - d0) * (r0 - 0.5) / (r0 - r1)
        })
        .collect();
    for (k, length) in lengths.iter().enumerate() {
        assert!(
            (length / HALF_LENGTH - 1.0).abs() < 0.05,
            "the half-length at {}° was {length}, not about {HALF_LENGTH}",
            15 * k
        );
    }
    let longest = lengths.iter().copied().fold(f64::MIN, f64::max);
    let shortest = lengths.iter().copied().fold(f64::MAX, f64::min);
    assert!(
        longest / shortest <= 1.03,
        "half-lengths ranged over {shortest}..{longest} across directions"
    );
}

/// The field is uncorrelated with itself at large displacements: powers of two,
/// shifts at which simplex noise driven by a 256-entry table comes close to
/// repeating, and large shifts in y.
#[test]
fn the_field_does_not_repeat() {
    const SHIFTS: [[f64; 2]; 20] = [
        [146_176.0, 0.0],
        [146_175.9, -0.075],
        [253_184.1, 0.129],
        [292_351.9, -0.149],
        [399_360.1, 0.055],
        [545_536.0, -0.020],
        [691_711.9, -0.095],
        [798_720.1, 0.109],
        [944_896.0, 0.035],
        [256.0, 0.0],
        [65_536.0, 0.0],
        [1_048_576.0, 0.0],
        [16_777_216.0, 0.0],
        [0.0, 256.0],
        [0.0, 4_096.0],
        [0.0, 65_536.0],
        [187.4, 187.4],
        [3_000_000.0, 1_000.0],
        [123_456_789.0, 0.0],
        [-1e6, 4096.0],
    ];
    for shift in SHIFTS {
        let mut rng = Rng(4);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..20_000 {
            let origin = [rng.range(0.0, 1e6), rng.range(0.0, 128.0)];
            let offset = [rng.range(0.0, 8.0), rng.range(0.0, 8.0)];
            a.push(field(origin, offset));
            b.push(field([origin[0] + shift[0], origin[1] + shift[1]], offset));
        }
        let r = correlation(&a, &b);
        assert!(
            r.abs() <= 0.05,
            "the field correlated {r} with itself shifted by {shift:?}"
        );
    }
}

/// FNV-1a over the field's bits at fixed probes, near and far from the origin.
///
/// Uses no library function whose result could differ between machines.
#[test]
fn outputs_match_the_pinned_digest() {
    const PINNED: u64 = 0xf9d7_c22d_2a77_173e;
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    let mut rng = Rng(0x5e_edd1_6e57);
    for [x, y] in [
        [0.0, 0.0],
        [0.37, 12.0],
        [1_000.25, -3.5],
        [123_456.789, 4_095.0],
        [1e6 + 0.5, 0.0],
        [-7.5e5, 2e3],
        [2_147_483_000.0, 5.0],
        [-3e9, 2e9],
    ] {
        let base = NoiseBase::new(x, y);
        for _ in 0..4096 {
            let dx = rng.unit_f32() * 64.0 - 32.0;
            let dy = rng.unit_f32() * 64.0 - 32.0;
            for byte in base.sample(dx, dy).to_bits().to_le_bytes() {
                digest ^= u64::from(byte);
                digest = digest.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    assert_eq!(digest, PINNED, "digest {digest:#018x}");
}
