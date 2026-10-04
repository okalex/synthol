use super::graph::CompiledGraph;
use super::midi::MidiEvent;
use super::modulation::{ModDepths, ModDestination};
use super::node::envelope::{AdsrEnvelope, AdsrSettings};
use super::node::filter::{BiquadState, Filter, FilterSettings};
use super::node::lfo::{Lfo, LfoSettings};
use super::node::oscillator::{Oscillator, Waveform};
use super::{MAX_LFOS, MAX_OSCILLATORS, OscillatorSettings};

/// The control values every voice starts from each sample, before its LFO
/// modulation is applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct VoiceControls {
    pub filter: FilterSettings,
    /// Per-oscillator settings; only pitch and level are read here.
    pub oscillators: [OscillatorSettings; MAX_OSCILLATORS],
    /// How many oscillators, from the first, sound.
    pub oscillator_count: usize,
    pub depths: [ModDepths; MAX_LFOS],
    /// The shared LFO's value in sync mode; `None` in trigger mode, where
    /// each voice uses its own LFO.
    pub sync_lfos: [Option<f32>; MAX_LFOS],
    pub lfo_count: usize,
}

/// Per-voice node state for the oscillator-to-filter-to-envelope graph.
#[derive(Debug, Default)]
pub(super) struct Voice {
    /// Every oscillator follows the voice's note; only the first
    /// `VoiceControls::oscillator_count` are rendered and mixed.
    oscillators: [Oscillator; MAX_OSCILLATORS],
    /// Each voice designs its own filter so the LFO can move it per note.
    filter: Filter,
    filter_state: BiquadState,
    envelope: AdsrEnvelope,
    /// This note's LFO: restarted on every note press and stopped when the
    /// voice falls silent.
    lfos: [Lfo; MAX_LFOS],
    lfo_count: usize,
    note: Option<u8>,
    /// Allocation order stamp; lower values are older and stolen first.
    started_at: u64,
}

impl Voice {
    pub(super) fn reset(&mut self, sample_rate: f32) -> bool {
        for oscillator in &mut self.oscillators {
            oscillator.reset(sample_rate);
        }
        self.filter.prepare(sample_rate);
        self.filter_state.reset();
        for lfo in &mut self.lfos {
            lfo.reset(sample_rate);
        }
        self.note = None;
        self.started_at = 0;
        self.envelope.prepare(sample_rate)
    }

    pub(super) fn set_envelope_settings(&mut self, settings: AdsrSettings) {
        self.envelope.set_settings(settings);
    }

    pub(super) fn set_lfo_settings(&mut self, index: usize, settings: LfoSettings) {
        self.lfos[index].set_settings(settings);
    }

    pub(super) fn set_lfo_count(&mut self, count: usize) {
        let active = self.is_active();
        for (index, lfo) in self.lfos.iter_mut().enumerate() {
            if index >= count {
                lfo.stop();
            } else if index >= self.lfo_count && active {
                lfo.start();
            }
        }
        self.lfo_count = count;
    }

    pub(super) fn set_waveform(&mut self, oscillator: usize, waveform: Waveform) {
        if let Some(oscillator) = self.oscillators.get_mut(oscillator) {
            oscillator.set_waveform(waveform);
        }
    }

    pub(super) fn set_start_phase(&mut self, oscillator: usize, phase: f32) {
        if let Some(oscillator) = self.oscillators.get_mut(oscillator) {
            oscillator.set_start_phase(phase);
        }
    }

    pub(super) fn note_on(&mut self, note: u8, velocity: u8, started_at: u64) {
        // A voice can go silent without rendering another sample (e.g. a
        // zero-length release), so clear stale filter memory here too. A
        // retriggered, still-sounding voice keeps it to avoid a click.
        if !self.is_active() {
            self.filter_state.reset();
        }
        for oscillator in &mut self.oscillators {
            oscillator.handle_event(MidiEvent::NoteOn { note, velocity });
        }
        self.envelope.note_on();
        for lfo in &mut self.lfos[..self.lfo_count] {
            lfo.start();
        }
        self.note = Some(note);
        self.started_at = started_at;
    }

    pub(super) fn note_off(&mut self) {
        if let Some(note) = self.oscillators[0].active_note() {
            for oscillator in &mut self.oscillators {
                oscillator.handle_event(MidiEvent::NoteOff { note });
            }
            self.envelope.note_off();
        }
        // An envelope released at level 0 goes idle at once, and an inactive
        // voice never reaches the cleanup in `next_sample`.
        if !self.is_active() {
            for lfo in &mut self.lfos {
                lfo.stop();
            }
        }
    }

    /// Renders one sample with `controls` modulated by the LFO.
    pub(super) fn next_sample(&mut self, graph: &CompiledGraph, controls: &VoiceControls) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        // The voice LFO keeps running in sync mode so its display stays
        // current if the mode switches back.
        let values = std::array::from_fn(|index| {
            let own_lfo = self.lfos[index].next_sample();
            controls.sync_lfos[index].unwrap_or(own_lfo)
        });
        self.apply_controls(controls, values);

        let oscillator_count = controls.oscillator_count.clamp(1, MAX_OSCILLATORS);
        let mut audio = 0.0;
        let mut output = 0.0;

        for &node in graph.execution_order() {
            if node == graph.oscillator() {
                audio = self.oscillators[..oscillator_count]
                    .iter_mut()
                    .map(Oscillator::next_sample)
                    .sum();
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

        if !self.envelope.is_active() && !self.is_held() {
            for oscillator in &mut self.oscillators {
                oscillator.stop();
            }
            self.filter_state.reset();
            for lfo in &mut self.lfos {
                lfo.stop();
            }
            self.note = None;
        }

        output
    }

    fn apply_controls(&mut self, controls: &VoiceControls, values: [f32; MAX_LFOS]) {
        let modulate = |destination: ModDestination, value: f32| {
            let offset = controls.depths[..controls.lfo_count]
                .iter()
                .zip(values)
                .map(|(depths, lfo)| depths.depth(destination) * lfo)
                .sum();
            destination.modulate(value, offset)
        };
        let count = controls.oscillator_count.clamp(1, MAX_OSCILLATORS);
        for (index, (oscillator, settings)) in self
            .oscillators
            .iter_mut()
            .zip(&controls.oscillators)
            .enumerate()
            .take(count)
        {
            oscillator.set_pitch(modulate(ModDestination::OscPitch(index), settings.pitch));
            oscillator.set_level(modulate(ModDestination::OscLevel(index), settings.level));
        }
        self.filter.set_settings(FilterSettings {
            cutoff_hz: modulate(ModDestination::FilterCutoff, controls.filter.cutoff_hz),
            q: modulate(ModDestination::FilterQ, controls.filter.q),
            mix: modulate(ModDestination::FilterMix, controls.filter.mix),
            ..controls.filter
        });
    }

    /// The note this voice is sounding, whether held or releasing.
    pub(super) fn note(&self) -> Option<u8> {
        self.note
    }

    /// Every oscillator gets the same note events, so the first speaks for
    /// all of them.
    pub(super) fn is_held(&self) -> bool {
        self.oscillators[0].has_active_note()
    }

    pub(super) fn is_active(&self) -> bool {
        self.is_held() || self.envelope.is_active()
    }

    /// This note's LFO position, or `None` if the voice is silent.
    pub(super) fn lfo_phase(&self, index: usize) -> Option<f32> {
        (self.is_active() && self.lfos[index].is_running()).then(|| self.lfos[index].phase())
    }

    pub(super) fn started_at(&self) -> u64 {
        self.started_at
    }

    #[cfg(test)]
    pub(super) fn envelope_level(&self) -> f32 {
        self.envelope.level()
    }
}
