//! Targeting animations to tunnel parameters.
//! Due to some quirks in the way I implemented the UI, it makes the most sense
//! to extract this as a separate module since it needs its own control layer
//! due to animation targets being scoped to an animation but owned by the tunnel.

use serde::{Deserialize, Serialize};
use strum::VariantArray;

/// Tunnel parameters that can be targeted by animations.
#[derive(
    Copy, Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum AnimationTarget {
    Rotation,
    Thickness,
    #[default]
    Size,
    AspectRatio,
    Color,
    ColorSpread,
    ColorSaturation,
    MarqueeRotation,
    PositionX,
    PositionY,
    Spin,
}

impl AnimationTarget {
    /// Whether this target moves a colour rather than a geometry.
    ///
    /// The two are answered in different places and at different rates, so
    /// which kind a target is decides where it gets resolved.
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Self::Color | Self::ColorSpread | Self::ColorSaturation
        )
    }

    /// Whether this target varies from point to point across a figure.
    ///
    /// A figure is one shape rather than a run of them, so a target that means
    /// the same thing everywhere on it is resolved into a single number before
    /// the layer is built and never has to reach the points. A marquee is the
    /// one that is simply dead: it slides segments along a path, and a figure
    /// has no segments.
    pub fn varies_across_figure(self) -> bool {
        match self {
            Self::Size
            | Self::AspectRatio
            | Self::Spin
            | Self::PositionX
            | Self::PositionY
            | Self::Color
            | Self::ColorSpread
            | Self::ColorSaturation => true,
            Self::Rotation | Self::Thickness | Self::MarqueeRotation => false,
        }
    }

    /// Whether what reaches a figure's points is this target's deviation from
    /// the placement rather than its whole value.
    ///
    /// Position is the one it is. A figure is placed before it is drawn, in the
    /// units a position is measured in rather than in the figure's own, so the
    /// whole value is spent putting it there and what is left for the points is
    /// how far each of them departs from that — which is nothing at all where
    /// the value is the same everywhere.
    pub fn deviates_from_the_placement(self) -> bool {
        match self {
            Self::PositionX | Self::PositionY => true,
            Self::Rotation
            | Self::Thickness
            | Self::Size
            | Self::AspectRatio
            | Self::Spin
            | Self::MarqueeRotation
            | Self::Color
            | Self::ColorSpread
            | Self::ColorSaturation => false,
        }
    }
}
