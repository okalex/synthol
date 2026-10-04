mod graph;
mod midi;
pub mod node;

pub use midi::MidiEvent;

use graph::{CompiledGraph, GraphDocument};
use node::envelope::{AdsrEnvelope, AdsrSettings};
use node::oscillator::SineOscillator;

#[derive(Debug)]
pub struct SynthEngine {
    graph: CompiledGraph,
    oscillator: SineOscillator,
    output_envelope: AdsrEnvelope,
}

impl Default for SynthEngine {
    fn default() -> Self {
        let graph = GraphDocument::initial()
            .compile()
            .expect("the built-in oscillator graph must be valid");

        Self {
            graph,
            oscillator: SineOscillator::default(),
            output_envelope: AdsrEnvelope::default(),
        }
    }
}

impl SynthEngine {
    pub fn reset(&mut self, sample_rate: f32) -> bool {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        self.oscillator.reset(sample_rate);
        let prepared = self.output_envelope.prepare(sample_rate);
        debug_assert!(prepared);
        prepared
    }

    pub fn set_output_envelope_settings(&mut self, settings: AdsrSettings) {
        self.output_envelope.set_settings(settings);
    }

    pub fn handle_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { velocity, .. } if velocity > 0 => {
                self.oscillator.handle_event(event);
                self.output_envelope.note_on();
            }
            MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note } => {
                if self.oscillator.active_note() == Some(note) {
                    self.oscillator.handle_event(event);
                    self.output_envelope.note_off();
                }
            }
        }
    }

    pub fn next_sample(&mut self, gain: f32) -> f32 {
        let mut audio = 0.0;
        let mut output = 0.0;

        for &node in self.graph.execution_order() {
            if node == self.graph.oscillator() {
                audio = self.oscillator.next_sample();
            } else if node == self.graph.envelope() {
                audio = self.output_envelope.process_sample(audio);
            } else if node == self.graph.output() {
                output = audio;
            }
        }

        if !self.output_envelope.is_active() && !self.oscillator.has_active_note() {
            self.oscillator.stop();
        }

        output * gain
    }

    pub fn has_active_note(&self) -> bool {
        self.oscillator.has_active_note() || self.output_envelope.is_active()
    }

    #[cfg(test)]
    pub(super) fn output_envelope_level(&self) -> f32 {
        self.output_envelope.level()
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
    fn later_note_on_takes_over() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
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

        assert!(!engine.oscillator.has_active_note());
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
        let envelope_after_one_sample = engine.output_envelope_level();
        engine.next_sample(0.5);
        assert_eq!(
            engine.output_envelope_level(),
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
}
