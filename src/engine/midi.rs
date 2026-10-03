#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
}
