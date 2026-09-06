use crate::layer::Layer;
use crate::mixer::Channel;
use crate::render_context::RenderContext;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;

/// A look is a beam that is essentially the contents of an entire mixer.
/// All channel settings are preserved.
#[derive(Clone, Serialize, Debug)]
pub struct Look {
    pub channels: Vec<Channel>,
}

impl Look {
    pub fn from_channels(channels: Vec<Channel>) -> Self {
        Self { channels }
    }

    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        for channel in &mut self.channels {
            channel.update_state(delta_t, audio_envelope);
        }
    }

    /// Draw all the Beams in this Look.
    ///
    /// Each subchannel contributes its own layers, so a look never merges shapes
    /// that are drawn differently into one layer.
    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        ctx: RenderContext,
        out: &mut Vec<Layer>,
    ) {
        for channel in &self.channels {
            channel.render(level, mask, ctx, out);
        }
    }
}

/// The deepest a look may nest looks inside itself when read from serial form.
///
/// A look holds channels, a channel holds a beam, and a beam may be another
/// look, so nesting is unbounded in the data and each level costs a stack
/// frame to recover. Without a ceiling, a few kilobytes can assert enough
/// levels to exhaust the stack outright, which no error can be reported from.
/// A look holding a look is already an unusual composition and a handful of
/// levels covers anything a show puts on stage, so this sits far above what
/// gets built and far below what a stack can absorb.
pub const MAX_NESTING_DEPTH: usize = 32;

thread_local! {
    /// How many levels of look nesting are currently being recovered.
    static NESTING_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Claims one level of look nesting for as long as it is held.
struct DepthGuard;

impl DepthGuard {
    /// Claim a level, or return `None` if the deepest allowed is already held.
    fn enter() -> Option<Self> {
        NESTING_DEPTH.with(|depth| {
            let next = depth.get() + 1;
            if next > MAX_NESTING_DEPTH {
                return None;
            }
            depth.set(next);
            Some(Self)
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        NESTING_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// The fields of a look, recovered with no ceiling on nesting.
#[derive(Deserialize)]
#[serde(rename = "Look")]
struct LookFields {
    channels: Vec<Channel>,
}

impl<'de> Deserialize<'de> for Look {
    /// Recover a look, refusing one nested deeper than `MAX_NESTING_DEPTH`.
    ///
    /// The ceiling is enforced on the way down rather than checked afterwards:
    /// an over-deep look never gets built, so the stack it would have consumed
    /// is never consumed.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _depth = DepthGuard::enter().ok_or_else(|| {
            de::Error::custom(format!(
                "a look nests more than {MAX_NESTING_DEPTH} looks deep"
            ))
        })?;
        let LookFields { channels } = LookFields::deserialize(deserializer)?;
        Ok(Self { channels })
    }
}
