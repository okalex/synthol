mod graph;
mod midi;
pub mod node;
mod voice;

pub use midi::MidiEvent;

use graph::{CompiledGraph, GraphDocument};
use node::envelope::AdsrSettings;
pub use node::oscillator::Waveform;
use voice::Voice;

/// Hard upper bound on simultaneous voices.
pub const MAX_VOICES: usize = 8;

#[derive(Debug)]
pub struct SynthEngine {
    graph: CompiledGraph,
    voices: [Voice; MAX_VOICES],
    voice_limit: usize,
    next_voice_stamp: u64,
}

impl Default for SynthEngine {
    fn default() -> Self {
        let graph = GraphDocument::initial()
            .compile()
            .expect("the built-in oscillator graph must be valid");

        Self {
            graph,
            voices: Default::default(),
            voice_limit: MAX_VOICES,
            next_voice_stamp: 0,
        }
    }
}

impl SynthEngine {
    pub fn reset(&mut self, sample_rate: f32) -> bool {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        let mut prepared = true;
        for voice in &mut self.voices {
            prepared &= voice.reset(sample_rate);
        }
        debug_assert!(prepared);
        self.next_voice_stamp = 0;
        prepared
    }

    pub fn set_output_envelope_settings(&mut self, settings: AdsrSettings) {
        for voice in &mut self.voices {
            voice.set_envelope_settings(settings);
        }
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        for voice in &mut self.voices {
            voice.set_waveform(waveform);
        }
    }

    /// Set the normalized oscillator phase (`0.0..1.0`, 0 to 360 degrees)
    /// that new notes start from.
    pub fn set_start_phase(&mut self, phase: f32) {
        for voice in &mut self.voices {
            voice.set_start_phase(phase);
        }
    }

    /// Limit how many voices new notes may use (clamped to `1..=MAX_VOICES`).
    /// Held voices above a lowered limit are released and allowed to finish.
    pub fn set_voice_limit(&mut self, limit: usize) {
        let limit = limit.clamp(1, MAX_VOICES);
        if limit == self.voice_limit {
            return;
        }
        if limit < self.voice_limit {
            for voice in &mut self.voices[limit..] {
                voice.note_off();
            }
        }
        self.voice_limit = limit;
    }

    pub fn voice_limit(&self) -> usize {
        self.voice_limit
    }

    pub fn handle_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { note, velocity } if velocity > 0 => {
                let index = self.allocate_voice(note);
                let stamp = self.next_voice_stamp;
                self.next_voice_stamp = self.next_voice_stamp.wrapping_add(1);
                self.voices[index].note_on(note, velocity, stamp);
            }
            MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note } => {
                for voice in &mut self.voices {
                    if voice.is_held() && voice.note() == Some(note) {
                        voice.note_off();
                    }
                }
            }
        }
    }

    /// Voice policy: retrigger a voice already sounding this note, else use a
    /// free voice, else steal the oldest releasing voice, else the oldest held.
    fn allocate_voice(&self, note: u8) -> usize {
        let voices = &self.voices[..self.voice_limit];

        if let Some(index) = voices.iter().position(|voice| voice.note() == Some(note)) {
            return index;
        }
        if let Some(index) = voices.iter().position(|voice| !voice.is_active()) {
            return index;
        }

        let oldest = |held: bool| {
            voices
                .iter()
                .enumerate()
                .filter(|(_, voice)| voice.is_held() == held)
                .min_by_key(|(_, voice)| voice.started_at())
                .map(|(index, _)| index)
        };
        oldest(false).or_else(|| oldest(true)).unwrap_or(0)
    }

    pub fn next_sample(&mut self, gain: f32) -> f32 {
        let graph = &self.graph;
        let output: f32 = self
            .voices
            .iter_mut()
            .map(|voice| voice.next_sample(graph))
            .sum();
        output * gain
    }

    pub fn has_active_note(&self) -> bool {
        self.voices.iter().any(Voice::is_active)
    }

    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.is_active()).count()
    }

    #[cfg(test)]
    fn voice_for_note(&self, note: u8) -> Option<&Voice> {
        self.voices.iter().find(|voice| voice.note() == Some(note))
    }
}

#[cfg(test)]
mod tests {
    use super::SynthEngine;
    use crate::engine::midi::MidiEvent;

    #[test]
    fn starts_silent_and_plays_a_note() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);

        assert_eq!(engine.next_sample(1.0), 0.0);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        assert!(engine.next_sample(1.0).abs() < f32::EPSILON);
        assert!(engine.has_active_note());
    }

    #[test]
    fn only_the_active_note_off_stops_the_oscillator() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        engine.handle_event(MidiEvent::NoteOff { note: 60 });
        assert!(engine.has_active_note());
        engine.handle_event(MidiEvent::NoteOff { note: 69 });
        assert!(!engine.has_active_note());
    }

    #[test]
    fn later_note_on_takes_over_in_mono_mode() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_voice_limit(1);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        engine.handle_event(MidiEvent::NoteOn {
            note: 72,
            velocity: 64,
        });
        engine.handle_event(MidiEvent::NoteOff { note: 69 });
        assert!(engine.has_active_note());
        assert_eq!(engine.active_voice_count(), 1);
        assert!(engine.voice_for_note(69).is_none());
    }

    #[test]
    fn note_off_keeps_engine_active_during_release() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        for _ in 0..100 {
            engine.next_sample(1.0);
        }
        engine.handle_event(MidiEvent::NoteOff { note: 69 });

        assert!(!engine.voice_for_note(69).unwrap().is_held());
        assert!(engine.has_active_note());

        for _ in 0..44_100 {
            engine.next_sample(1.0);
        }
        assert!(!engine.has_active_note());
    }

    #[test]
    fn oscillator_continues_through_envelope_release() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        for _ in 0..1_000 {
            engine.next_sample(1.0);
        }
        engine.handle_event(MidiEvent::NoteOff { note: 69 });

        let peak = (0..128)
            .map(|_| engine.next_sample(1.0).abs())
            .fold(0.0_f32, f32::max);
        assert!(peak > 0.0);
    }

    #[test]
    fn output_envelope_precedes_output_gain() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });

        engine.next_sample(1.0);
        let envelope_after_one_sample = engine.voice_for_note(69).unwrap().envelope_level();
        engine.next_sample(0.5);
        assert_eq!(
            engine.voice_for_note(69).unwrap().envelope_level(),
            envelope_after_one_sample * 2.0
        );
    }

    #[test]
    fn invalid_sample_rate_does_not_reset_the_engine() {
        let mut engine = SynthEngine::default();
        assert!(engine.reset(48_000.0));
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });

        assert!(!engine.reset(0.0));
        assert!(engine.has_active_note());
        assert!(engine.reset(44_100.0));
        assert!(!engine.has_active_note());
    }

    fn note_on(engine: &mut SynthEngine, note: u8) {
        engine.handle_event(MidiEvent::NoteOn {
            note,
            velocity: 127,
        });
    }

    #[test]
    fn plays_a_chord_on_separate_voices() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        for note in [60, 64, 67] {
            note_on(&mut engine, note);
        }
        assert_eq!(engine.active_voice_count(), 3);

        engine.handle_event(MidiEvent::NoteOff { note: 64 });
        assert!(engine.voice_for_note(60).unwrap().is_held());
        assert!(!engine.voice_for_note(64).unwrap().is_held());
        assert!(engine.voice_for_note(67).unwrap().is_held());
    }

    #[test]
    fn chord_output_is_the_sum_of_its_voices() {
        let render = |notes: &[u8]| {
            let mut engine = SynthEngine::default();
            engine.reset(44_100.0);
            for &note in notes {
                note_on(&mut engine, note);
            }
            (0..2_000)
                .map(|_| engine.next_sample(1.0))
                .collect::<Vec<_>>()
        };
        let a = render(&[60]);
        let b = render(&[67]);
        let chord = render(&[60, 67]);
        for ((a, b), chord) in a.iter().zip(&b).zip(&chord) {
            assert!((a + b - chord).abs() < 1e-5);
        }
    }

    #[test]
    fn repeated_note_retriggers_its_voice() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        note_on(&mut engine, 60);
        engine.handle_event(MidiEvent::NoteOff { note: 60 });
        note_on(&mut engine, 60);
        assert_eq!(engine.active_voice_count(), 1);
        assert!(engine.voice_for_note(60).unwrap().is_held());
    }

    #[test]
    fn steals_the_oldest_held_voice_when_full() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_voice_limit(3);
        for note in [60, 62, 64, 65] {
            note_on(&mut engine, note);
        }
        assert_eq!(engine.active_voice_count(), 3);
        assert!(engine.voice_for_note(60).is_none());
        for note in [62, 64, 65] {
            assert!(engine.voice_for_note(note).unwrap().is_held());
        }
    }

    #[test]
    fn prefers_stealing_a_releasing_voice() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_voice_limit(2);
        note_on(&mut engine, 60);
        note_on(&mut engine, 62);
        engine.handle_event(MidiEvent::NoteOff { note: 62 });
        note_on(&mut engine, 64);

        assert!(engine.voice_for_note(60).unwrap().is_held());
        assert!(engine.voice_for_note(62).is_none());
        assert!(engine.voice_for_note(64).unwrap().is_held());
    }

    #[test]
    fn supports_up_to_max_voices() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_voice_limit(100);
        assert_eq!(engine.voice_limit(), super::MAX_VOICES);
        for note in 60..60 + super::MAX_VOICES as u8 + 2 {
            note_on(&mut engine, note);
        }
        assert_eq!(engine.active_voice_count(), super::MAX_VOICES);
    }

    #[test]
    fn lowering_the_voice_limit_releases_excess_voices() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        for note in [60, 64, 67] {
            note_on(&mut engine, note);
        }
        engine.set_voice_limit(1);

        assert!(engine.voice_for_note(60).unwrap().is_held());
        assert!(!engine.voice_for_note(64).unwrap().is_held());
        assert!(!engine.voice_for_note(67).unwrap().is_held());

        note_on(&mut engine, 72);
        assert!(engine.voice_for_note(60).is_none());
        assert!(engine.voice_for_note(72).unwrap().is_held());
    }
}
