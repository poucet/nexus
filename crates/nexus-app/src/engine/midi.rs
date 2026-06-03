//! MIDI input: connect to a device and feed raw messages to the audio thread
//! over a lock-free ring buffer.

use anyhow::Context;
use midir::{Ignore, MidiInput, MidiInputConnection};
use rtrb::{Consumer, RingBuffer};

/// A 3-byte MIDI channel-voice message (note on/off, CC, …). Longer messages
/// (sysex) are dropped for now.
pub type MidiMessage = [u8; 3];

/// The consumer side of the MIDI ring buffer, drained on the audio thread.
pub type MidiConsumer = Consumer<MidiMessage>;

/// A live MIDI connection plus the device name. The connection must be kept
/// alive for messages to keep flowing.
pub struct MidiConnection {
    pub name: String,
    _connection: MidiInputConnection<()>,
}

/// Connect to the most-recently-attached MIDI input, returning the consumer the
/// audio thread reads from plus the live connection. If no device is present,
/// returns an empty consumer and `None` (the host simply receives no MIDI).
pub fn connect_default() -> anyhow::Result<(MidiConsumer, Option<MidiConnection>)> {
    let (mut producer, consumer) = RingBuffer::<MidiMessage>::new(512);

    let mut input = MidiInput::new("Nexus").context("create MIDI input")?;
    input.ignore(Ignore::SysexAndTime);

    let ports = input.ports();
    let Some(port) = ports.last() else {
        return Ok((consumer, None)); // no MIDI device — producer is dropped
    };
    let name = input.port_name(port).unwrap_or_else(|_| "MIDI input".to_string());

    let connection = input
        .connect(
            port,
            "nexus-midi-in",
            move |_timestamp, message, ()| {
                let mut buf = [0u8; 3];
                let n = message.len().min(3);
                buf[..n].copy_from_slice(&message[..n]);
                let _ = producer.push(buf);
            },
            (),
        )
        .map_err(|e| anyhow::anyhow!("MIDI connect failed: {e}"))?;

    Ok((consumer, Some(MidiConnection { name, _connection: connection })))
}
