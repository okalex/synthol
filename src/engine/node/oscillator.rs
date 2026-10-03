use std::f32::consts::TAU;

use crate::engine::MidiEvent;

#[derive(Debug)]
pub struct SineOscillator {
    sample_rate: f32,
    phase: f32,
    frequency: f32,
    amplitude: f32,
    active_note: Option<u8>,
}

impl Default for SineOscillator {
    fn default() -> Self {
        Self {
            sample_rate: 44_100.0,
            phase: 0.0,
            frequency: 0.0,
            amplitude: 0.0,
            active_note: None,
        }
    }
}

impl SineOscillator {
    pub fn reset(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.phase = 0.0;
        self.frequency = 0.0;
        self.amplitude = 0.0;
        self.active_note = None;
    }

    pub fn handle_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { note, velocity } if velocity > 0 => {
                self.active_note = Some(note);
                self.frequency = midi_note_frequency(note);
                self.amplitude = f32::from(velocity) / 127.0;
                self.phase = 0.0;
            }
            MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note } => {
                if self.active_note == Some(note) {
                    self.active_note = None;
                    self.amplitude = 0.0;
                }
            }
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        if self.active_note.is_none() {
            return 0.0;
        }

        let output = self.phase.sin() * self.amplitude;
        self.phase = (self.phase + TAU * self.frequency / self.sample_rate) % TAU;
        output
    }

    pub fn has_active_note(&self) -> bool {
        self.active_note.is_some()
    }
}

pub fn midi_note_frequency(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::midi_note_frequency;

    #[test]
    fn midi_a4_is_440_hz() {
        assert!((midi_note_frequency(69) - 440.0).abs() < 0.001);
    }

    #[test]
    fn midi_notes_are_one_octave_apart() {
        let a4 = midi_note_frequency(69);
        let a5 = midi_note_frequency(81);
        assert!((a5 - 2.0 * a4).abs() < 0.001);
    }
}
