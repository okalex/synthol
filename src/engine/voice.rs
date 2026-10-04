use super::graph::CompiledGraph;
use super::midi::MidiEvent;
use super::node::envelope::{AdsrEnvelope, AdsrSettings};
use super::node::oscillator::{Oscillator, Waveform};

/// Per-voice node state for the oscillator-to-envelope graph.
#[derive(Debug, Default)]
pub(super) struct Voice {
    oscillator: Oscillator,
    envelope: AdsrEnvelope,
    note: Option<u8>,
    /// Allocation order stamp; lower values are older and stolen first.
    started_at: u64,
}

impl Voice {
    pub(super) fn reset(&mut self, sample_rate: f32) -> bool {
        self.oscillator.reset(sample_rate);
        self.note = None;
        self.started_at = 0;
        self.envelope.prepare(sample_rate)
    }

    pub(super) fn set_envelope_settings(&mut self, settings: AdsrSettings) {
        self.envelope.set_settings(settings);
    }

    pub(super) fn set_waveform(&mut self, waveform: Waveform) {
        self.oscillator.set_waveform(waveform);
    }

    pub(super) fn set_start_phase(&mut self, phase: f32) {
        self.oscillator.set_start_phase(phase);
    }

    pub(super) fn note_on(&mut self, note: u8, velocity: u8, started_at: u64) {
        self.oscillator
            .handle_event(MidiEvent::NoteOn { note, velocity });
        self.envelope.note_on();
        self.note = Some(note);
        self.started_at = started_at;
    }

    pub(super) fn note_off(&mut self) {
        if let Some(note) = self.oscillator.active_note() {
            self.oscillator.handle_event(MidiEvent::NoteOff { note });
            self.envelope.note_off();
        }
    }

    pub(super) fn next_sample(&mut self, graph: &CompiledGraph) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        let mut audio = 0.0;
        let mut output = 0.0;

        for &node in graph.execution_order() {
            if node == graph.oscillator() {
                audio = self.oscillator.next_sample();
            } else if node == graph.envelope() {
                audio = self.envelope.process_sample(audio);
            } else if node == graph.output() {
                output = audio;
            }
        }

        if !self.envelope.is_active() && !self.oscillator.has_active_note() {
            self.oscillator.stop();
            self.note = None;
        }

        output
    }

    /// The note this voice is sounding, whether held or releasing.
    pub(super) fn note(&self) -> Option<u8> {
        self.note
    }

    pub(super) fn is_held(&self) -> bool {
        self.oscillator.has_active_note()
    }

    pub(super) fn is_active(&self) -> bool {
        self.oscillator.has_active_note() || self.envelope.is_active()
    }

    pub(super) fn started_at(&self) -> u64 {
        self.started_at
    }

    #[cfg(test)]
    pub(super) fn envelope_level(&self) -> f32 {
        self.envelope.level()
    }
}
