use super::effects::{EffectChain, EffectControls, MAX_EFFECTS};
use super::midi::MidiEvent;
use super::modulation::{ModDepths, ModDestination};
use super::node::envelope::{AdsrEnvelope, AdsrSettings};
use super::node::filter::{BiquadState, Filter, FilterSettings};
use super::node::lfo::{Lfo, LfoSettings};
use super::node::oscillator::{Oscillator, Waveform};
use super::{MAX_ENVELOPES, MAX_LFOS, MAX_OSCILLATORS, OscillatorSettings};

/// The control values every voice starts from each sample, before its
/// modulation is applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct VoiceControls {
    pub effects: EffectControls,
    /// Per-oscillator pitch, level, and unison settings.
    pub oscillators: [OscillatorSettings; MAX_OSCILLATORS],
    /// How many oscillators, from the first, sound.
    pub oscillator_count: usize,
    pub depths: [ModDepths; MAX_LFOS],
    pub envelope_depths: [ModDepths; MAX_ENVELOPES],
    /// The shared LFO's value in sync mode; `None` in trigger mode, where
    /// each voice uses its own LFO.
    pub sync_lfos: [Option<f32>; MAX_LFOS],
    pub lfo_count: usize,
}

/// Per-voice audio and modulator state.
#[derive(Debug)]
pub(super) struct Voice {
    /// Every oscillator follows the voice's note; only the first
    /// `VoiceControls::oscillator_count` are rendered and mixed.
    oscillators: [Oscillator; MAX_OSCILLATORS],
    /// Each voice designs its own filter so the LFO can move it per note.
    filters: [Filter; MAX_EFFECTS],
    filter_states: [[BiquadState; 2]; MAX_EFFECTS],
    envelopes: [AdsrEnvelope; MAX_ENVELOPES],
    envelope_count: usize,
    /// This note's LFO: restarted on every note press and stopped when the
    /// voice falls silent.
    lfos: [Lfo; MAX_LFOS],
    lfo_count: usize,
    note: Option<u8>,
    /// Allocation order stamp; lower values are older and stolen first.
    started_at: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            oscillators: Default::default(),
            filters: Default::default(),
            filter_states: Default::default(),
            envelopes: Default::default(),
            envelope_count: 1,
            lfos: Default::default(),
            lfo_count: 0,
            note: None,
            started_at: 0,
        }
    }
}

impl Voice {
    pub(super) fn reset(&mut self, sample_rate: f32) -> bool {
        for oscillator in &mut self.oscillators {
            oscillator.reset(sample_rate);
        }
        for (filter, state) in self.filters.iter_mut().zip(&mut self.filter_states) {
            filter.prepare(sample_rate);
            for channel in state {
                channel.reset();
            }
        }
        for lfo in &mut self.lfos {
            lfo.reset(sample_rate);
        }
        self.note = None;
        self.started_at = 0;
        self.envelopes
            .iter_mut()
            .all(|envelope| envelope.prepare(sample_rate))
    }

    pub(super) fn set_envelope_settings(&mut self, index: usize, settings: AdsrSettings) {
        self.envelopes[index].set_settings(settings);
    }

    pub(super) fn set_effect_chain(&mut self, old: &EffectChain, new: &EffectChain) {
        for slot in 0..MAX_EFFECTS {
            if old.slots().contains(&slot) != new.slots().contains(&slot) {
                for channel in &mut self.filter_states[slot] {
                    channel.reset();
                }
            }
        }
    }

    pub(super) fn set_envelope_count(&mut self, count: usize) {
        for (index, envelope) in self.envelopes.iter_mut().enumerate() {
            if index >= count {
                envelope.reset();
            } else if index >= self.envelope_count && self.oscillators[0].has_active_note() {
                envelope.note_on();
            }
        }
        self.envelope_count = count;
        if !self.is_active() {
            self.stop();
        }
    }

    pub(super) fn envelope_level_at(&self, index: usize) -> Option<f32> {
        (index < self.envelope_count).then(|| self.envelopes[index].level())
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
            for state in &mut self.filter_states {
                for channel in state {
                    channel.reset();
                }
            }
        }
        for oscillator in &mut self.oscillators {
            oscillator.handle_event(MidiEvent::NoteOn { note, velocity });
        }
        for envelope in &mut self.envelopes[..self.envelope_count] {
            envelope.note_on();
        }
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
            for envelope in &mut self.envelopes[..self.envelope_count] {
                envelope.note_off();
            }
        }
        // An envelope released at level 0 goes idle at once, and an inactive
        // voice never reaches the cleanup in `next_sample`.
        if !self.is_active() {
            for lfo in &mut self.lfos {
                lfo.stop();
            }
        }
    }

    /// Renders one sample with `controls` modulated by LFOs and ADSRs.
    pub(super) fn next_sample(&mut self, controls: &VoiceControls) -> [f32; 2] {
        if !self.is_active() {
            return [0.0; 2];
        }

        // The voice LFO keeps running in sync mode so its display stays
        // current if the mode switches back.
        let values = std::array::from_fn(|index| {
            let own_lfo = self.lfos[index].next_sample();
            controls.sync_lfos[index].unwrap_or(own_lfo)
        });
        let envelope_values =
            std::array::from_fn(|index| self.envelopes[index].process_sample(1.0));
        self.apply_controls(controls, values, envelope_values);

        let oscillator_count = controls.oscillator_count.clamp(1, MAX_OSCILLATORS);
        let mut audio = [0.0; 2];
        for (oscillator, settings) in self.oscillators[..oscillator_count]
            .iter_mut()
            .zip(&controls.oscillators)
        {
            let sample = oscillator.next_stereo_sample(settings.unison);
            for channel in 0..2 {
                audio[channel] += sample[channel];
            }
        }
        for &slot in controls.effects.chain.slots() {
            for (state, sample) in self.filter_states[slot].iter_mut().zip(&mut audio) {
                *sample = state.process_sample(self.filters[slot].coefficients(), *sample);
            }
        }

        if !self.is_active() {
            self.stop();
        }

        audio
    }

    fn stop(&mut self) {
        for oscillator in &mut self.oscillators {
            oscillator.stop();
        }
        for state in &mut self.filter_states {
            for channel in state {
                channel.reset();
            }
        }
        for lfo in &mut self.lfos {
            lfo.stop();
        }
        self.note = None;
    }

    fn apply_controls(
        &mut self,
        controls: &VoiceControls,
        values: [f32; MAX_LFOS],
        envelope_values: [f32; MAX_ENVELOPES],
    ) {
        let modulate = |destination: ModDestination, value: f32| {
            let offset = controls.depths[..controls.lfo_count]
                .iter()
                .zip(values)
                .map(|(depths, lfo)| depths.depth(destination) * lfo)
                .sum();
            let value = destination.modulate(value, offset);
            controls.envelope_depths[..self.envelope_count]
                .iter()
                .zip(envelope_values)
                .fold(value, |value, (depths, envelope)| {
                    destination.modulate_envelope(value, envelope, depths.depth(destination))
                })
        };
        let count = controls.oscillator_count.clamp(1, MAX_OSCILLATORS);
        let held = self.is_held();
        for (index, (oscillator, settings)) in self
            .oscillators
            .iter_mut()
            .zip(&controls.oscillators)
            .enumerate()
            .take(count)
        {
            oscillator.set_pitch(modulate(ModDestination::OscPitch(index), settings.pitch));
            oscillator.set_shape(modulate(ModDestination::OscShape(index), settings.shape));
            oscillator.set_pan(settings.pan);
            let destination = ModDestination::OscLevel(index);
            let has_level_envelope = controls.envelope_depths[..self.envelope_count]
                .iter()
                .any(|depths| depths.depth(destination) != 0.0);
            let level = if held || has_level_envelope {
                modulate(destination, settings.level)
            } else {
                0.0
            };
            oscillator.set_level(level);
        }
        for &slot in controls.effects.chain.slots() {
            let settings = controls.effects.filters[slot];
            let [cutoff, q, mix] = ModDestination::filter_destinations(slot);
            self.filters[slot].set_settings(FilterSettings {
                cutoff_hz: modulate(cutoff, settings.cutoff_hz),
                q: modulate(q, settings.q),
                mix: modulate(mix, settings.mix),
                ..settings
            });
        }
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
        self.is_held()
            || self.envelopes[..self.envelope_count]
                .iter()
                .any(AdsrEnvelope::is_active)
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
        self.envelopes[0].level()
    }
}
