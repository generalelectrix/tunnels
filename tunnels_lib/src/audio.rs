//! Audio analysis as data: one frame of role envelopes and spectrum levels
//! per audio buffer, and the role selected to follow.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::number::UnipolarFloat;

/// An `f32` in the range [0, 1].
///
/// The range is upheld by clamping at construction. There is no arithmetic on
/// the type; convert it to a [`UnipolarFloat`] to compute with it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UnipolarF32(f32);

impl UnipolarF32 {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    /// Clamp the provided value to the unit range.
    ///
    /// NaN is rejected: it reads as zero, and fails a debug assertion.
    pub const fn new(v: f32) -> Self {
        if v.is_nan() {
            debug_assert!(false, "NaN passed to UnipolarF32");
        }
        Self::clamped(v)
    }

    /// Clamp a value to the unit range, reading NaN as zero.
    const fn clamped(v: f32) -> Self {
        if v.is_nan() {
            Self::ZERO
        } else {
            Self(v.clamp(0.0, 1.0))
        }
    }

    /// The inner value.
    pub const fn val(self) -> f32 {
        self.0
    }
}

impl From<UnipolarF32> for f32 {
    fn from(v: UnipolarF32) -> Self {
        v.0
    }
}

impl From<UnipolarF32> for f64 {
    fn from(v: UnipolarF32) -> Self {
        v.0 as f64
    }
}

/// Exact: every `f32` in [0, 1] is an `f64` in [0, 1].
impl From<UnipolarF32> for UnipolarFloat {
    fn from(v: UnipolarF32) -> Self {
        UnipolarFloat::new(v.0 as f64)
    }
}

/// Serialized as a plain `f32`.
impl Serialize for UnipolarF32 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// Deserialized from a plain `f32`, clamped to the unit range, with NaN read
/// as zero.
impl<'de> Deserialize<'de> for UnipolarF32 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f32::deserialize(deserializer).map(Self::clamped)
    }
}

/// An envelope the show can follow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Hits in the low end: kick drums, and any sharp low onset.
    Kick,
    /// The low end's level, kick included.
    #[default]
    Bass,
    /// The middle's level: voices, keys, guitars, the body of a snare.
    Mid,
    /// Hits in the high end: hats, shakers, snare wires, cymbal strikes.
    Hats,
    /// The high end's level: cymbals, hats, sibilance and air.
    Shimmer,
}

/// Number of roles.
pub const NUM_ROLES: usize = 5;

impl Role {
    /// Every role, in output order.
    pub const ALL: [Self; NUM_ROLES] =
        [Self::Kick, Self::Bass, Self::Mid, Self::Hats, Self::Shimmer];

    /// The role at an output index, if there is one.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The role's position in the output order.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The role's name as an operator reads it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Bass => "Bass",
            Self::Mid => "Mid",
            Self::Hats => "Hats",
            Self::Shimmer => "Shimmer",
        }
    }
}

/// Number of spectrum bands, in ascending frequency order.
pub const SPECTRUM_BANDS: usize = 26;

/// One audio buffer's analysis: every role's envelope and every spectrum
/// band's level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioFrame {
    roles: [UnipolarF32; NUM_ROLES],
    spectrum: [UnipolarF32; SPECTRUM_BANDS],
}

impl AudioFrame {
    /// A frame of role envelopes, in [`Role::ALL`] order, and band levels, in
    /// ascending frequency order.
    pub fn new(roles: [UnipolarF32; NUM_ROLES], spectrum: [UnipolarF32; SPECTRUM_BANDS]) -> Self {
        Self { roles, spectrum }
    }

    /// A role's envelope.
    pub fn role(&self, role: Role) -> UnipolarFloat {
        self.roles[role.index()].into()
    }

    /// Every role's envelope, in [`Role::ALL`] order.
    pub fn roles(&self) -> &[UnipolarF32; NUM_ROLES] {
        &self.roles
    }

    /// Every band's level, in ascending frequency order.
    pub fn spectrum(&self) -> [UnipolarFloat; SPECTRUM_BANDS] {
        self.spectrum.map(UnipolarFloat::from)
    }
}

/// The latest audio analysis and the role the show follows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioState {
    pub frame: AudioFrame,
    pub active_role: Role,
}

impl AudioState {
    /// The followed role's envelope.
    pub fn envelope(&self) -> UnipolarFloat {
        self.frame.role(self.active_role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction clamps to the unit range and reads NaN as zero; the
    /// value converts out exactly; the wire form is a plain `f32`, re-clamped
    /// when read back.
    #[test]
    fn unipolar_f32_holds_its_range() {
        assert_eq!(UnipolarF32::new(-0.5), UnipolarF32::ZERO);
        assert_eq!(UnipolarF32::new(1.0 + f32::EPSILON), UnipolarF32::ONE);
        assert_eq!(UnipolarF32::new(f32::INFINITY), UnipolarF32::ONE);
        let nan = std::panic::catch_unwind(|| UnipolarF32::new(f32::NAN));
        if cfg!(debug_assertions) {
            let message = nan.expect_err("NaN fails the debug assertion");
            assert_eq!(
                message.downcast_ref::<&str>(),
                Some(&"NaN passed to UnipolarF32")
            );
        } else {
            assert_eq!(nan.expect("NaN reads as zero"), UnipolarF32::ZERO);
        }

        // An f32 with no short decimal form, so a rounding on the way to
        // f64 would show.
        let v = UnipolarF32::new(0.1);
        assert_eq!(UnipolarFloat::from(v).val(), 0.1_f32 as f64);
        assert_eq!(f64::from(v), 0.1_f32 as f64);
        assert_eq!(f32::from(v), 0.1_f32);

        let bytes = postcard::to_allocvec(&v).expect("serialize");
        assert_eq!(bytes, 0.1_f32.to_le_bytes());
        for (wire, read) in [
            (1.5_f32, UnipolarF32::ONE),
            (-2.0, UnipolarF32::ZERO),
            (f32::NAN, UnipolarF32::ZERO),
        ] {
            let decoded: UnipolarF32 =
                postcard::from_bytes(&wire.to_le_bytes()).expect("deserialize");
            assert_eq!(decoded, read, "{wire} on the wire");
        }
    }

    /// A frame is 31 plain `f32`s on the wire; the envelope is the active
    /// role's.
    #[test]
    fn audio_state_reads_the_active_role() {
        let frame = AudioFrame::new(
            std::array::from_fn(|r| UnipolarF32::new(r as f32 / 10.0)),
            std::array::from_fn(|b| UnipolarF32::new(b as f32 / 100.0)),
        );
        assert_eq!(
            postcard::to_allocvec(&frame).expect("serialize").len(),
            4 * (NUM_ROLES + SPECTRUM_BANDS)
        );
        for role in Role::ALL {
            let state = AudioState {
                frame,
                active_role: role,
            };
            assert_eq!(state.envelope(), frame.role(role));
            assert_eq!(
                state.envelope(),
                UnipolarFloat::new((role.index() as f32 / 10.0) as f64)
            );
        }
        assert_eq!(
            frame.spectrum()[25],
            UnipolarFloat::new((25.0_f32 / 100.0) as f64)
        );
    }
}
