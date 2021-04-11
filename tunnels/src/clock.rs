use crate::numbers::UnipolarFloat;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// how many globally-available clocks?
const N_CLOCKS: usize = 8;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ClockIdx(usize);

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank([Clock; N_CLOCKS]);

impl ClockBank {
    pub fn curr_angle(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].curr_angle()
    }

    pub fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].submaster_level
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clock {
    curr_angle: UnipolarFloat,
    /// in unit angle per second
    rate: f64,
    /// did the clock tick on its most recent update?
    ticked: bool,
    /// is this clock running in "one-shot" mode?
    /// the clock runs for one cycle when triggered then waits for another
    /// trigger event
    one_shot: bool,
    /// should this clock reset and tick on its next update?
    reset_on_update: bool,
    /// submaster level for this clock
    pub submaster_level: UnipolarFloat,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            curr_angle: UnipolarFloat(0.0),
            rate: 0.0,
            ticked: true,
            one_shot: false,
            reset_on_update: false,
            submaster_level: UnipolarFloat(1.0),
        }
    }
}

impl Clock {
    pub fn update_state(&mut self, delta_t: Duration) {
        if self.reset_on_update {
            self.ticked = true;
            self.curr_angle = UnipolarFloat(0.0);
            self.reset_on_update = false;
        } else {
            // delta_t has units of us, need to divide by 1000000
            let new_angle = self.curr_angle.0 + (self.rate * delta_t.as_secs_f64());

            // if we're running in one-shot mode, clamp the angle at 1.0
            if self.one_shot && new_angle >= 1.0 {
                self.curr_angle = UnipolarFloat(1.0);
                self.ticked = false;
            } else {
                // if the phase just escaped our range, we ticked this frame
                self.ticked = new_angle >= 1.0 || new_angle < 0.0;
                self.curr_angle = UnipolarFloat(new_angle % 1.0);
            }
        }
    }

    pub fn curr_angle(&self) -> UnipolarFloat {
        self.curr_angle
    }
}
