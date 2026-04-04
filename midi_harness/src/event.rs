//! Types for representing basic MIDI events.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Specification for what type of midi event.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventType {
    NoteOn,
    NoteOff,
    ControlChange,
}

/// A specification of a midi mapping.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Mapping {
    pub event_type: EventType,
    pub channel: u8,
    pub control: u8,
}

impl fmt::Display for Mapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{}",
            match self.event_type {
                EventType::NoteOn => "NoteOn ",
                EventType::NoteOff => "NoteOff",
                EventType::ControlChange => "CntChng",
            },
            self.channel,
            self.control
        )
    }
}

/// Helper constructor for a note on mapping.
pub const fn note_on(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::NoteOn,
        channel,
        control,
    }
}

/// Helper constructor for a note off mapping.
pub const fn note_off(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::NoteOff,
        channel,
        control,
    }
}

/// Helper constructor - most controls are on channel 0.
pub const fn note_on_ch0(control: u8) -> Mapping {
    note_on(0, control)
}

/// Helper constructor - other relevant special case is channel 1.
pub const fn note_on_ch1(control: u8) -> Mapping {
    note_on(1, control)
}

/// Helper constructor for a control change mapping.
pub const fn cc(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::ControlChange,
        channel,
        control,
    }
}

/// Helper constructor - most controls are on channel 0.
pub const fn cc_ch0(control: u8) -> Mapping {
    cc(0, control)
}

/// A fully-specified midi event.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct Event {
    pub mapping: Mapping,
    pub value: u8,
}

impl Event {
    /// Interpret a byte slice as a MIDI event.
    pub fn parse(msg: &[u8]) -> Result<Self, ParseError> {
        if msg.len() < 3 {
            return Err(ParseError::TooShort(msg.len()));
        }
        let control = msg[1];
        let value = msg[2];
        let event_type = match msg[0] >> 4 {
            8 => EventType::NoteOff,
            // Most midi devices just send NoteOn with a velocity of 0 for NoteOff.
            9 if value == 0 => EventType::NoteOff,
            9 => EventType::NoteOn,
            11 => EventType::ControlChange,
            other => {
                return Err(ParseError::BadType(other));
            }
        };

        let channel = msg[0] & 15;
        Ok(Event {
            mapping: Mapping {
                event_type,
                channel,
                control,
            },
            value,
        })
    }
}

/// Helper constructor for a midi event.
pub const fn event(mapping: Mapping, value: u8) -> Event {
    Event { mapping, value }
}

#[derive(Debug)]
pub enum ParseError {
    TooShort(usize),
    BadType(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_real_note_off_with_nonzero_velocity() {
        // Status 0x82 = NoteOff on channel 2, control 50, release velocity 64.
        let msg = [0x82, 50, 64];
        let event = Event::parse(&msg).expect("should parse real NoteOff with non-zero velocity");
        assert_eq!(event.mapping.event_type, EventType::NoteOff);
        assert_eq!(event.mapping.channel, 2);
        assert_eq!(event.mapping.control, 50);
        assert_eq!(event.value, 64);
    }
}
