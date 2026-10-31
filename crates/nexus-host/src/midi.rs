//! Channel-voice messages, read from MIDI 1.0 bytes or a UMP packet.
//!
//! A backend that takes raw MIDI (CLAP) passes [`Event::Midi`] and
//! [`Event::Midi2`] through untouched. One that does not (VST3) reads both
//! into a [`ChannelVoice`] — a note, a poly pressure or a controller value,
//! normalised to `0..=1` — and maps that onto its own events and parameters,
//! so a MIDI 2.0 packet keeps its 16- and 32-bit resolution (`N8`).
//!
//! [`Event::Midi`]: crate::process::Event::Midi
//! [`Event::Midi2`]: crate::process::Event::Midi2

/// A controller a channel-voice message moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    /// Control change `0..=127`.
    Cc(u8),
    /// Channel pressure (aftertouch).
    ChannelPressure,
    /// Pitch bend; `0.5` is the centre.
    PitchBend,
}

/// One channel-voice message. Channels are `0..=15`, keys `0..=127`, every
/// value `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelVoice {
    NoteOn { channel: i16, key: i16, velocity: f64 },
    NoteOff { channel: i16, key: i16, velocity: f64 },
    PolyPressure { channel: i16, key: i16, pressure: f64 },
    Controller { channel: i16, control: Control, value: f64 },
}

impl ChannelVoice {
    /// Read a MIDI 1.0 message (status + two data bytes). A note-on with
    /// velocity 0 is a note-off, as MIDI 1.0 says.
    pub fn from_midi1(data: &[u8; 3]) -> Option<Self> {
        let channel = (data[0] & 0x0F) as i16;
        let key = (data[1] & 0x7F) as i16;
        let seven = |b: u8| (b & 0x7F) as f64 / 127.0;
        Some(match data[0] & 0xF0 {
            0x90 if data[2] & 0x7F > 0 => Self::NoteOn { channel, key, velocity: seven(data[2]) },
            0x80 | 0x90 => Self::NoteOff { channel, key, velocity: seven(data[2]) },
            0xA0 => Self::PolyPressure { channel, key, pressure: seven(data[2]) },
            0xB0 => Self::Controller { channel, control: Control::Cc(key as u8), value: seven(data[2]) },
            0xD0 => Self::Controller { channel, control: Control::ChannelPressure, value: seven(data[1]) },
            0xE0 => {
                let raw = (data[1] & 0x7F) as u32 | (((data[2] & 0x7F) as u32) << 7);
                Self::Controller { channel, control: Control::PitchBend, value: raw as f64 / 16383.0 }
            }
            _ => return None,
        })
    }

    /// Read a UMP packet: a MIDI 1.0 channel-voice message (message type 2),
    /// or a MIDI 2.0 one (type 4) at its full resolution. The group is not
    /// part of the result. Anything else (utility, system, SysEx, per-note
    /// controllers, program change) is `None`.
    pub fn from_ump(words: &[u32; 4]) -> Option<Self> {
        let w0 = words[0];
        let status = ((w0 >> 16) & 0xFF) as u8;
        let (index, data) = (((w0 >> 8) & 0xFF) as u8, (w0 & 0xFF) as u8);
        match w0 >> 28 {
            0x2 => Self::from_midi1(&[status, index, data]),
            0x4 => {
                let channel = (status & 0x0F) as i16;
                let key = (index & 0x7F) as i16;
                let sixteen = (words[1] >> 16) as f64 / 65535.0;
                let thirty_two = words[1] as f64 / u32::MAX as f64;
                Some(match status & 0xF0 {
                    // MIDI 2.0: velocity 0 is a note-on like any other.
                    0x90 => Self::NoteOn { channel, key, velocity: sixteen },
                    0x80 => Self::NoteOff { channel, key, velocity: sixteen },
                    0xA0 => Self::PolyPressure { channel, key, pressure: thirty_two },
                    0xB0 => Self::Controller { channel, control: Control::Cc(index & 0x7F), value: thirty_two },
                    0xD0 => Self::Controller { channel, control: Control::ChannelPressure, value: thirty_two },
                    0xE0 => Self::Controller { channel, control: Control::PitchBend, value: thirty_two },
                    _ => return None,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MIDI 2.0 channel-voice packet: group 0, `status`, `index` byte,
    /// `extra` byte, and the data word.
    fn m2(status: u8, index: u8, extra: u8, data: u32) -> [u32; 4] {
        [0x4000_0000 | (status as u32) << 16 | (index as u32) << 8 | extra as u32, data, 0, 0]
    }

    #[test]
    fn midi1_bytes_read_as_before() {
        assert_eq!(
            ChannelVoice::from_midi1(&[0x93, 60, 127]),
            Some(ChannelVoice::NoteOn { channel: 3, key: 60, velocity: 1.0 })
        );
        assert_eq!(
            ChannelVoice::from_midi1(&[0x90, 60, 0]),
            Some(ChannelVoice::NoteOff { channel: 0, key: 60, velocity: 0.0 })
        );
        assert_eq!(
            ChannelVoice::from_midi1(&[0xE1, 0x7F, 0x7F]),
            Some(ChannelVoice::Controller { channel: 1, control: Control::PitchBend, value: 1.0 })
        );
        assert_eq!(
            ChannelVoice::from_midi1(&[0xD2, 127, 0]),
            Some(ChannelVoice::Controller { channel: 2, control: Control::ChannelPressure, value: 1.0 })
        );
        assert_eq!(ChannelVoice::from_midi1(&[0xC0, 5, 0]), None, "program change");
    }

    #[test]
    fn a_ump_midi1_packet_reads_like_its_bytes() {
        let packet = [0x2093_3C40, 0, 0, 0]; // group 0, note on ch 3, key 60, vel 64
        assert_eq!(ChannelVoice::from_ump(&packet), ChannelVoice::from_midi1(&[0x93, 0x3C, 0x40]));
    }

    #[test]
    fn midi2_keeps_its_resolution() {
        assert_eq!(
            ChannelVoice::from_ump(&m2(0x95, 64, 0, 0x8000_0000)),
            Some(ChannelVoice::NoteOn { channel: 5, key: 64, velocity: 0x8000 as f64 / 65535.0 })
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0x90, 64, 0, 0)),
            Some(ChannelVoice::NoteOn { channel: 0, key: 64, velocity: 0.0 }),
            "velocity 0 is still a note-on in MIDI 2.0"
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0x81, 64, 0, 0xFFFF_0000)),
            Some(ChannelVoice::NoteOff { channel: 1, key: 64, velocity: 1.0 })
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0xB2, 74, 0, u32::MAX)),
            Some(ChannelVoice::Controller { channel: 2, control: Control::Cc(74), value: 1.0 })
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0xE0, 0, 0, 0x8000_0000)),
            Some(ChannelVoice::Controller {
                channel: 0,
                control: Control::PitchBend,
                value: 0x8000_0000u32 as f64 / u32::MAX as f64,
            })
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0xA0, 60, 0, u32::MAX)),
            Some(ChannelVoice::PolyPressure { channel: 0, key: 60, pressure: 1.0 })
        );
        assert_eq!(
            ChannelVoice::from_ump(&m2(0xD0, 0, 0, 0)),
            Some(ChannelVoice::Controller { channel: 0, control: Control::ChannelPressure, value: 0.0 })
        );
    }

    #[test]
    fn other_ump_messages_are_none() {
        assert_eq!(ChannelVoice::from_ump(&[0x0000_0000, 0, 0, 0]), None, "utility");
        assert_eq!(ChannelVoice::from_ump(&[0x10F8_0000, 0, 0, 0]), None, "system");
        assert_eq!(ChannelVoice::from_ump(&m2(0xC0, 0, 0, 0)), None, "program change");
        assert_eq!(ChannelVoice::from_ump(&m2(0x00, 60, 0, 0)), None, "per-note controller");
    }
}
