//! Targeting animations to tunnel parameters.
//! Due to some quirks in the way I implemented the UI, it makes the most sense
//! to extract this as a separate module since it needs its own control layer
//! due to animation targets being scoped to an animation but owned by the tunnel.

use serde::{Deserialize, Serialize};

/// Tunnel parameters that can be targeted by animations.
#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum AnimationTarget {
    Rotation,
    Thickness,
    Size,
    AspectRatio,
    Color,
    ColorSpread,
    ColorSaturation,
    MarqueeRotation,
    PositionX,
    PositionY,
}

impl Default for AnimationTarget {
    fn default() -> Self {
        Self::Size
    }
}
