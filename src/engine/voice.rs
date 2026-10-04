use super::graph::CompiledGraph;
use super::midi::MidiEvent;
use super::modulation::{ModDepths, ModDestination};
use super::node::envelope::{AdsrEnvelope, AdsrSettings};
use super::node::filter::{BiquadState, Filter, FilterSettings};
use super::node::lfo::{Lfo, LfoSettings};
use super::node::oscillator::{Oscillator, Waveform};

/// The control values every voice starts from each sample, before its LFO
/// modulation is applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct VoiceControls {
    pub filter: FilterSettings,
    /// Oscillator transposition in semitones.
    pub pitch: f32,
    /// Oscillator gain, 0 to 1.
    pub level: f32,
    pub depths: ModDepths,
    /// The shared LFO's value in sync mode; `None` in trigger mode, where
    /// each voice uses its own LFO.
    pub sync_lfo: Option<f32>,
}

/// Per-voice node state for the oscillator-to-filter-to-envelope graph.
#[derive(Debug, Default)]
pub(super) struct Voice {
    oscillator: Oscillator,
    /// Each voice designs its own filter so the LFO can move it per note.
    filter: Filter,
    filter_state: BiquadState,
    envelope: AdsrEnvelope,
    /// This note's LFO: restarted on every note press and stopped when the
    /// voice falls silent.
    lfo: Lfo,
    note: Option<u8>,
    /// Allocation order stamp; lower values are older and stolen first.
    started_at: u64,
}

impl Voice {
    pub(super) fn reset(&mut self, sample_rate: f32) -> bool {
        self.oscillator.reset(sample_rate);
        self.filter.prepare(sample_rate);
        self.filter_state.reset();
        self.lfo.reset(sample_rate);
        self.note = None;
        self.started_at = 0;
        self.envelope.prepare(sample_rate)
    }

    pub(super) fn set_envelope_settings(&mut self, settings: AdsrSettings) {
        self.envelope.set_settings(settings);
    }

    pub(super) fn set_lfo_settings(&mut self, settings: LfoSettings) {
        self.lfo.set_settings(settings);
    }

    pub(super) fn set_waveform(&mut self, waveform: Waveform) {
        self.oscillator.set_waveform(waveform);
    }

    pub(super) fn set_start_phase(&mut self, phase: f32) {
        self.oscillator.set_start_phase(phase);
    }

    pub(super) fn note_on(&mut self, note: u8, velocity: u8, started_at: u64) {
        // A voice can go silent without rendering another sample (e.g. a
        // zero-length release), so clear stale filter memory here too. A
        // retriggered, still-sounding voice keeps it to avoid a click.
        if !self.is_active() {
            self.filter_state.reset();
        }
        self.oscillator
            .handle_event(MidiEvent::NoteOn { note, velocity });
        self.envelope.note_on();
        self.lfo.start();
        self.note = Some(note);
        self.started_at = started_at;
    }

    pub(super) fn note_off(&mut self) {
        if let Some(note) = self.oscillator.active_note() {
            self.oscillator.handle_event(MidiEvent::NoteOff { note });
            self.envelope.note_off();
        }
        // An envelope released at level 0 goes idle at once, and an inactive
        // voice never reaches the cleanup in `next_sample`.
        if !self.is_active() {
            self.lfo.stop();
        }
    }

    /// Renders one sample with `controls` modulated by the LFO.
    pub(super) fn next_sample(&mut self, graph: &CompiledGraph, controls: &VoiceControls) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        // The voice LFO keeps running in sync mode so its display stays
        // current if the mode switches back.
        let own_lfo = self.lfo.next_sample();
        self.apply_controls(controls, controls.sync_lfo.unwrap_or(own_lfo));

        let mut audio = 0.0;
        let mut output = 0.0;

        for &node in graph.execution_order() {
            if node == graph.oscillator() {
                audio = self.oscillator.next_sample();
            } else if node == graph.filter() {
                audio = self
                    .filter_state
                    .process_sample(self.filter.coefficients(), audio);
            } else if node == graph.envelope() {
                audio = self.envelope.process_sample(audio);
            } else if node == graph.output() {
                output = audio;
            }
        }

        if !self.envelope.is_active() && !self.oscillator.has_active_note() {
            self.oscillator.stop();
            self.filter_state.reset();
            self.lfo.stop();
            self.note = None;
        }

        output
    }

    fn apply_controls(&mut self, controls: &VoiceControls, lfo: f32) {
        let modulate = |destination: ModDestination, value: f32| {
            destination.modulate(value, controls.depths.depth(destination) * lfo)
        };
        self.oscillator
            .set_pitch(modulate(ModDestination::OscPitch, controls.pitch));
        self.oscillator
            .set_level(modulate(ModDestination::OscLevel, controls.level));
        self.filter.set_settings(FilterSettings {
            cutoff_hz: modulate(ModDestination::FilterCutoff, controls.filter.cutoff_hz),
            q: modulate(ModDestination::FilterQ, controls.filter.q),
            ..controls.filter
        });
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

    /// This note's LFO position, or `None` if the voice is silent.
    pub(super) fn lfo_phase(&self) -> Option<f32> {
        (self.is_active() && self.lfo.is_running()).then(|| self.lfo.phase())
    }

    pub(super) fn started_at(&self) -> u64 {
        self.started_at
    }

    #[cfg(test)]
    pub(super) fn envelope_level(&self) -> f32 {
        self.envelope.level()
    }
}
