mod graph;
mod midi;
pub mod node;

pub use midi::MidiEvent;

use graph::{CompiledGraph, GraphDocument};
use node::oscillator::SineOscillator;

#[derive(Debug)]
pub struct SynthEngine {
    graph: CompiledGraph,
    oscillator: SineOscillator,
}

impl Default for SynthEngine {
    fn default() -> Self {
        let graph = GraphDocument::initial()
            .compile()
            .expect("the built-in oscillator graph must be valid");

        Self {
            graph,
            oscillator: SineOscillator::default(),
        }
    }
}

impl SynthEngine {
    pub fn reset(&mut self, sample_rate: f32) {
        self.oscillator.reset(sample_rate);
    }

    pub fn handle_event(&mut self, event: MidiEvent) {
        self.oscillator.handle_event(event);
    }

    pub fn next_sample(&mut self, gain: f32) -> f32 {
        let mut audio = 0.0;
        let mut output = 0.0;

        for &node in self.graph.execution_order() {
            if node == self.graph.oscillator() {
                audio = self.oscillator.next_sample();
            } else if node == self.graph.output() {
                output = audio;
            }
        }

        output * gain
    }

    pub fn has_active_note(&self) -> bool {
        self.oscillator.has_active_note()
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
}
