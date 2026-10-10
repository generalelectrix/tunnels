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
    /// the layer is built and never has to reach the points. A rotation is one
    /// of those: a figure turns as a whole, and there is no second thing for a
    /// second reading of the knob to turn. A marquee is another: on a figure
    /// it is a second rotation, applied inside the aspect stretch rather than
    /// outside it, and a whole figure turns with that one too.
    ///
    /// Thickness is not one of those, though it reads like one. An outline has
    /// a width at every point of the contour it follows, so a periodicity of
    /// two makes it thick at two places around the figure and thin between
    /// them — which is a beam rather than a line of even weight.
    pub fn varies_across_figure(self) -> bool {
        match self {
            Self::Size
            | Self::AspectRatio
            | Self::Spin
            | Self::Thickness
            | Self::PositionX
            | Self::PositionY
            | Self::Color
            | Self::ColorSpread
            | Self::ColorSaturation => true,
            Self::Rotation | Self::MarqueeRotation => false,
        }
    }

    /// Whether a value that is the same everywhere on a figure moves the
    /// figure as a whole rather than each of its points.
    ///
    /// Position is the one that does. Moving every point by one value is
    /// moving the figure, and the placement does that without displacing a
    /// point; a value that varies across the figure moves each point by its
    /// own share instead.
    pub fn moves_the_figure_when_uniform(self) -> bool {
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
