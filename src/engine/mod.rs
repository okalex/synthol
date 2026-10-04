mod graph;
mod midi;
pub mod modulation;
pub mod node;
mod voice;

pub use midi::MidiEvent;

use graph::{CompiledGraph, GraphDocument};
use modulation::ModDepths;
pub use modulation::{MOD_SLOTS, ModDestination, ModRoute};
use node::envelope::AdsrSettings;
pub use node::filter::{FilterMode, FilterSettings};
use node::lfo::Lfo;
pub use node::lfo::{LfoMode, LfoSettings};
pub use node::oscillator::Waveform;
use voice::{Voice, VoiceControls};

/// Hard upper bound on simultaneous voices.
pub const MAX_VOICES: usize = 8;
/// Hard upper bound on oscillators per voice.
pub const MAX_OSCILLATORS: usize = 4;

/// One oscillator's controls. Every voice runs each oscillator on its note
/// and mixes the active ones before the filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscillatorSettings {
    pub waveform: Waveform,
    /// Normalized phase (`0.0..1.0`, 0 to 360 degrees) new notes start from.
    pub start_phase: f32,
    /// Transposition in semitones, before LFO modulation.
    pub pitch: f32,
    /// Gain (`0.0..=1.0`), before LFO modulation.
    pub level: f32,
}

impl Default for OscillatorSettings {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            start_phase: 0.0,
            pitch: 0.0,
            level: 1.0,
        }
    }
}

/// Where each running LFO is in its cycle, for display.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LfoPositions {
    /// Normalized phase per voice slot, `None` where no LFO is running. In
    /// sync mode only slot 0 is used, for the shared LFO.
    pub phases: [Option<f32>; MAX_VOICES],
    /// The slot of the most recently started LFO among the running ones.
    pub newest: Option<usize>,
}

#[derive(Debug)]
pub struct SynthEngine {
    graph: CompiledGraph,
    filter: FilterSettings,
    oscillators: [OscillatorSettings; MAX_OSCILLATORS],
    /// How many oscillators, from the first, sound.
    oscillator_count: usize,
    depths: ModDepths,
    /// The free-running LFO shared by every voice in sync mode. In trigger
    /// mode each voice runs its own instead.
    sync_lfo: Lfo,
    lfo_mode: LfoMode,
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
            filter: FilterSettings::default(),
            oscillators: [OscillatorSettings::default(); MAX_OSCILLATORS],
            oscillator_count: 1,
            depths: ModDepths::default(),
            sync_lfo: Lfo::default(),
            lfo_mode: LfoMode::default(),
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
        self.sync_lfo.reset(sample_rate);
        if self.lfo_mode == LfoMode::Sync {
            self.sync_lfo.start();
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

    /// Set the filter every voice's oscillator is routed through, before LFO
    /// modulation. Each voice recomputes its coefficients only when its
    /// (modulated) settings change, so this is cheap to call per sample.
    pub fn set_filter_settings(&mut self, settings: FilterSettings) {
        self.filter = settings;
    }

    /// Sound the first `count` oscillators (clamped to
    /// `1..=MAX_OSCILLATORS`).
    pub fn set_oscillator_count(&mut self, count: usize) {
        self.oscillator_count = count.clamp(1, MAX_OSCILLATORS);
    }

    pub fn oscillator_count(&self) -> usize {
        self.oscillator_count
    }

    /// Apply every control of oscillator `index`; out-of-range indices are
    /// ignored.
    pub fn set_oscillator(&mut self, index: usize, settings: OscillatorSettings) {
        self.set_waveform(index, settings.waveform);
        self.set_start_phase(index, settings.start_phase);
        self.set_oscillator_pitch(index, settings.pitch);
        self.set_oscillator_level(index, settings.level);
    }

    /// Transpose oscillator `index` by `semitones`, before LFO modulation.
    pub fn set_oscillator_pitch(&mut self, index: usize, semitones: f32) {
        if let Some(oscillator) = self.oscillators.get_mut(index) {
            oscillator.pitch = semitones;
        }
    }

    /// Set oscillator `index`'s gain (`0.0..=1.0`), before LFO modulation.
    pub fn set_oscillator_level(&mut self, index: usize, level: f32) {
        if let Some(oscillator) = self.oscillators.get_mut(index) {
            oscillator.level = level;
        }
    }

    /// Route the LFO to destinations; routes sharing a destination add up.
    pub fn set_modulation(&mut self, routes: &[ModRoute]) {
        self.depths = ModDepths::from_routes(routes);
    }

    pub fn set_lfo_settings(&mut self, settings: LfoSettings) {
        self.lfo_mode = settings.mode;
        self.sync_lfo.set_settings(settings);
        match settings.mode {
            LfoMode::Sync if !self.sync_lfo.is_running() => self.sync_lfo.start(),
            LfoMode::Sync => {}
            LfoMode::Trigger => self.sync_lfo.stop(),
        }
        for voice in &mut self.voices {
            voice.set_lfo_settings(settings);
        }
    }

    /// Positions of the LFOs that apply in the current mode. Voice LFOs keep
    /// running in sync mode (they're cheap), so switching back to trigger
    /// mode shows where each sounding note's LFO is.
    pub fn lfo_positions(&self) -> LfoPositions {
        let mut positions = LfoPositions::default();
        match self.lfo_mode {
            LfoMode::Sync => {
                if self.sync_lfo.is_running() {
                    positions.phases[0] = Some(self.sync_lfo.phase());
                    positions.newest = Some(0);
                }
            }
            LfoMode::Trigger => {
                let mut newest_start = None;
                for (index, voice) in self.voices.iter().enumerate() {
                    let Some(phase) = voice.lfo_phase() else {
                        continue;
                    };
                    positions.phases[index] = Some(phase);
                    if newest_start.is_none_or(|start| voice.started_at() > start) {
                        newest_start = Some(voice.started_at());
                        positions.newest = Some(index);
                    }
                }
            }
        }
        positions
    }

    pub fn set_waveform(&mut self, index: usize, waveform: Waveform) {
        let Some(oscillator) = self.oscillators.get_mut(index) else {
            return;
        };
        oscillator.waveform = waveform;
        for voice in &mut self.voices {
            voice.set_waveform(index, waveform);
        }
    }

    /// Set the normalized phase (`0.0..1.0`, 0 to 360 degrees) that new
    /// notes start oscillator `index` from.
    pub fn set_start_phase(&mut self, index: usize, phase: f32) {
        let Some(oscillator) = self.oscillators.get_mut(index) else {
            return;
        };
        oscillator.start_phase = phase;
        for voice in &mut self.voices {
            voice.set_start_phase(index, phase);
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
        // Advance the shared LFO every sample, even with no notes, so it
        // free-runs.
        let sync_lfo = self.sync_lfo.next_sample();
        let controls = VoiceControls {
            filter: self.filter,
            oscillators: self.oscillators,
            oscillator_count: self.oscillator_count,
            depths: self.depths,
            sync_lfo: (self.lfo_mode == LfoMode::Sync).then_some(sync_lfo),
        };
        let graph = &self.graph;
        let output: f32 = self
            .voices
            .iter_mut()
            .map(|voice| voice.next_sample(graph, &controls))
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

    fn render_note(settings: super::FilterSettings) -> Vec<f32> {
        let mut engine = SynthEngine::default();
        engine.reset(48_000.0);
        engine.set_waveform(0, super::Waveform::Sawtooth);
        engine.set_filter_settings(settings);
        note_on(&mut engine, 57);
        (0..4_800).map(|_| engine.next_sample(1.0)).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn oscillators_are_routed_through_the_filter() {
        use super::{FilterMode, FilterSettings};

        let open = render_note(FilterSettings::default());
        let closed = render_note(FilterSettings {
            mode: FilterMode::LowPass,
            cutoff_hz: 100.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
        });
        let high_passed = render_note(FilterSettings {
            mode: FilterMode::HighPass,
            cutoff_hz: 10_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
        });
        assert!(rms(&closed) < rms(&open) * 0.25);
        assert!(rms(&high_passed) < rms(&open) * 0.25);
    }

    #[test]
    fn filter_memory_is_cleared_when_a_voice_finishes() {
        use super::{FilterMode, FilterSettings};
        use crate::engine::AdsrSettings;
        use std::time::Duration;

        let resonant_engine = || {
            let mut engine = SynthEngine::default();
            engine.reset(48_000.0);
            engine.set_filter_settings(FilterSettings {
                mode: FilterMode::LowPass,
                cutoff_hz: 300.0,
                q: 10.0,
            });
            engine.set_output_envelope_settings(AdsrSettings {
                attack: Duration::ZERO,
                decay: Duration::ZERO,
                sustain_db: 0.0,
                release: Duration::ZERO,
            });
            engine
        };
        let first_samples = |engine: &mut SynthEngine| {
            note_on(engine, 57);
            (0..64).map(|_| engine.next_sample(1.0)).collect::<Vec<_>>()
        };

        let mut engine = resonant_engine();
        note_on(&mut engine, 57);
        for _ in 0..1_000 {
            engine.next_sample(1.0);
        }
        engine.handle_event(MidiEvent::NoteOff { note: 57 });
        for _ in 0..10 {
            engine.next_sample(1.0);
        }
        assert!(!engine.has_active_note());

        assert_eq!(
            first_samples(&mut engine),
            first_samples(&mut resonant_engine())
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

    fn lfo_settings(mode: super::LfoMode) -> super::LfoSettings {
        super::LfoSettings {
            waveform: super::Waveform::Sine,
            frequency_hz: 2.0,
            mode,
        }
    }

    fn run(engine: &mut SynthEngine, samples: usize) {
        for _ in 0..samples {
            engine.next_sample(1.0);
        }
    }

    fn running_phases(engine: &SynthEngine) -> Vec<f32> {
        engine
            .lfo_positions()
            .phases
            .iter()
            .flatten()
            .copied()
            .collect()
    }

    #[test]
    fn trigger_mode_gives_each_note_its_own_lfo() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        assert_eq!(engine.lfo_positions(), super::LfoPositions::default());

        note_on(&mut engine, 24);
        run(&mut engine, 4_410);
        note_on(&mut engine, 38);
        let positions = engine.lfo_positions();
        let first = positions.phases[0].unwrap();
        let second = positions.phases[1].unwrap();
        // 0.1 s at 2 Hz: the first note's LFO keeps going; the second starts.
        assert!((first - 0.2).abs() < 1e-4, "{first}");
        assert_eq!(second, 0.0);
        assert_eq!(positions.newest, Some(1));

        run(&mut engine, 4_410);
        let positions = engine.lfo_positions();
        assert!((positions.phases[0].unwrap() - 0.4).abs() < 1e-4);
        assert!((positions.phases[1].unwrap() - 0.2).abs() < 1e-4);
    }

    #[test]
    fn retriggered_and_stolen_voices_restart_their_lfo() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_voice_limit(1);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        note_on(&mut engine, 60);
        run(&mut engine, 1_000);
        note_on(&mut engine, 64);
        assert_eq!(running_phases(&engine), [0.0]);
    }

    #[test]
    fn trigger_lfos_stop_when_their_note_finishes() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        note_on(&mut engine, 60);
        note_on(&mut engine, 64);
        run(&mut engine, 1_000);
        engine.handle_event(MidiEvent::NoteOff { note: 60 });
        // Releasing notes keep their LFO until the release ends.
        run(&mut engine, 100);
        assert_eq!(running_phases(&engine).len(), 2);

        run(&mut engine, 2 * 44_100);
        let positions = engine.lfo_positions();
        assert_eq!(positions.phases.iter().flatten().count(), 1);
        assert_eq!(positions.newest, Some(1));

        engine.handle_event(MidiEvent::NoteOff { note: 64 });
        run(&mut engine, 2 * 44_100);
        assert_eq!(engine.lfo_positions(), super::LfoPositions::default());
    }

    #[test]
    fn a_note_released_before_it_sounds_drops_its_lfo() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        note_on(&mut engine, 60);
        engine.handle_event(MidiEvent::NoteOff { note: 60 });
        assert_eq!(engine.lfo_positions(), super::LfoPositions::default());
    }

    #[test]
    fn sync_lfo_runs_without_notes_and_does_not_restart() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Sync));
        run(&mut engine, 4_410);
        let positions = engine.lfo_positions();
        let phase = positions.phases[0].unwrap();
        assert!((phase - 0.2).abs() < 1e-4, "{phase}");
        assert_eq!(positions.newest, Some(0));

        note_on(&mut engine, 60);
        note_on(&mut engine, 64);
        assert_eq!(running_phases(&engine), [phase]);
    }

    #[test]
    fn switching_modes_shows_the_matching_lfos() {
        let mut engine = SynthEngine::default();
        engine.reset(44_100.0);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        note_on(&mut engine, 60);
        run(&mut engine, 4_410);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Sync));
        assert_eq!(running_phases(&engine), [0.0]);
        run(&mut engine, 4_410);
        engine.set_lfo_settings(lfo_settings(super::LfoMode::Trigger));
        let phase = engine.lfo_positions().phases[0].unwrap();
        assert!((phase - 0.4).abs() < 1e-4, "{phase}");
    }

    #[test]
    fn an_unrouted_lfo_does_not_change_the_audio_output() {
        let render = |settings: Option<super::LfoSettings>| {
            let mut engine = SynthEngine::default();
            engine.reset(44_100.0);
            if let Some(settings) = settings {
                engine.set_lfo_settings(settings);
            }
            note_on(&mut engine, 60);
            note_on(&mut engine, 67);
            (0..2_000)
                .map(|_| engine.next_sample(1.0))
                .collect::<Vec<_>>()
        };
        let reference = render(None);
        for mode in [super::LfoMode::Trigger, super::LfoMode::Sync] {
            assert_eq!(render(Some(lfo_settings(mode))), reference);
        }
    }

    use super::{ModDestination, ModRoute};

    /// An engine with an instant, full-level envelope and a 1 Hz square LFO,
    /// which reads +1 for the first half of its cycle and -1 for the second.
    fn modulated_engine(mode: super::LfoMode, routes: &[ModRoute]) -> SynthEngine {
        use crate::engine::AdsrSettings;
        use std::time::Duration;

        let mut engine = SynthEngine::default();
        engine.reset(48_000.0);
        engine.set_output_envelope_settings(AdsrSettings {
            attack: Duration::ZERO,
            decay: Duration::ZERO,
            sustain_db: 0.0,
            release: Duration::ZERO,
        });
        engine.set_lfo_settings(super::LfoSettings {
            waveform: super::Waveform::Square,
            frequency_hz: 1.0,
            mode,
        });
        engine.set_modulation(routes);
        engine
    }

    fn route(destination: ModDestination, amount: f32) -> ModRoute {
        ModRoute {
            destination: Some(destination),
            amount,
        }
    }

    fn render(engine: &mut SynthEngine, samples: usize) -> Vec<f32> {
        (0..samples).map(|_| engine.next_sample(1.0)).collect()
    }

    fn rising_zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count()
    }

    #[test]
    fn empty_and_zero_routes_leave_the_output_untouched() {
        let render_with = |routes: &[ModRoute]| {
            let mut engine = modulated_engine(super::LfoMode::Trigger, routes);
            engine.set_filter_settings(super::FilterSettings {
                cutoff_hz: 2_000.0,
                q: 4.0,
                ..Default::default()
            });
            engine.set_oscillator_pitch(0, 3.0);
            engine.set_oscillator_level(0, 0.6);
            note_on(&mut engine, 60);
            render(&mut engine, 2_000)
        };
        let reference = render_with(&[]);
        let zero = ModDestination::ALL.map(|destination| route(destination, 0.0));
        assert_eq!(render_with(&zero), reference);
        assert_eq!(render_with(&[ModRoute::default(); 4]), reference);
    }

    #[test]
    fn pitch_and_level_controls_shape_the_note() {
        let mut engine = modulated_engine(super::LfoMode::Trigger, &[]);
        note_on(&mut engine, 69);
        let reference = render(&mut engine, 4_800);

        let mut engine = modulated_engine(super::LfoMode::Trigger, &[]);
        engine.set_oscillator_pitch(0, 12.0);
        engine.set_oscillator_level(0, 0.5);
        note_on(&mut engine, 69);
        let shifted = render(&mut engine, 4_800);

        // 0.1 s of 440 Hz against 880 Hz, at half the amplitude.
        assert_eq!(rising_zero_crossings(&reference), 43);
        assert_eq!(rising_zero_crossings(&shifted), 87);
        assert!((rms(&shifted) - rms(&reference) * 0.5).abs() < 0.01);
    }

    #[test]
    fn lfo_modulates_pitch_in_semitones() {
        // An octave of depth: the first half-second plays A5, the second A3.
        let mut engine = modulated_engine(
            super::LfoMode::Trigger,
            &[route(ModDestination::OscPitch(0), 12.0 / 48.0)],
        );
        note_on(&mut engine, 69);
        let samples = render(&mut engine, 48_000);
        let up = rising_zero_crossings(&samples[..4_800]);
        let down = rising_zero_crossings(&samples[24_000..28_800]);
        assert!((87..=88).contains(&up), "{up}");
        assert!((21..=22).contains(&down), "{down}");
    }

    #[test]
    fn lfo_modulates_level_and_clamps_it() {
        // Level 0.5 swung by half the range: silent for the first half-cycle
        // and full for the second.
        let mut engine = modulated_engine(
            super::LfoMode::Trigger,
            &[route(ModDestination::OscLevel(0), -0.75)],
        );
        engine.set_oscillator_level(0, 0.5);
        note_on(&mut engine, 69);
        let samples = render(&mut engine, 48_000);
        assert!(samples[..23_000].iter().all(|&sample| sample == 0.0));
        let peak = samples[25_000..]
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!(peak > 0.99, "{peak}");
    }

    #[test]
    fn lfo_modulates_the_filter_cutoff_per_voice() {
        let closed = super::FilterSettings {
            cutoff_hz: 100.0,
            ..Default::default()
        };
        let mut steady = modulated_engine(super::LfoMode::Trigger, &[]);
        steady.set_filter_settings(closed);
        note_on(&mut steady, 69);
        let steady = render(&mut steady, 4_800);

        // Five octaves up while the LFO is high.
        let octaves = (20_000.0_f32 / 20.0).log2();
        let mut opened = modulated_engine(
            super::LfoMode::Trigger,
            &[route(ModDestination::FilterCutoff, 5.0 / octaves)],
        );
        opened.set_filter_settings(closed);
        note_on(&mut opened, 69);
        let opened = render(&mut opened, 4_800);
        assert!(rms(&opened) > rms(&steady) * 4.0);
    }

    #[test]
    fn trigger_mode_restarts_modulation_per_note_and_sync_does_not() {
        let level_route = [route(ModDestination::OscLevel(0), -1.0)];
        // Trigger: each note starts at the top of its own LFO cycle, muted
        // for half a second, no matter when it's played.
        let mut engine = modulated_engine(super::LfoMode::Trigger, &level_route);
        note_on(&mut engine, 60);
        render(&mut engine, 30_000);
        note_on(&mut engine, 67);
        let samples = render(&mut engine, 12_000);
        assert!(samples.iter().any(|&sample| sample != 0.0));
        // Note 60 is in its loud half; note 67 should still be silent, so the
        // output is note 60 alone.
        let mut alone = modulated_engine(super::LfoMode::Trigger, &level_route);
        note_on(&mut alone, 60);
        render(&mut alone, 30_000);
        assert_eq!(render(&mut alone, 12_000), samples);

        // Sync: the shared LFO is already in its loud half, so a new note
        // sounds immediately.
        let mut engine = modulated_engine(super::LfoMode::Sync, &level_route);
        render(&mut engine, 30_000);
        note_on(&mut engine, 67);
        let samples = render(&mut engine, 4_800);
        assert!(rms(&samples) > 0.5, "{}", rms(&samples));
    }

    fn silent_oscillator_settings() -> super::OscillatorSettings {
        super::OscillatorSettings {
            level: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn only_the_active_oscillators_sound_and_they_mix() {
        let mut single = modulated_engine(super::LfoMode::Trigger, &[]);
        note_on(&mut single, 69);
        let single = render(&mut single, 4_800);

        // A second oscillator past the count is ignored.
        let mut hidden = modulated_engine(super::LfoMode::Trigger, &[]);
        hidden.set_oscillator(
            1,
            super::OscillatorSettings {
                waveform: super::Waveform::Square,
                pitch: 7.0,
                ..Default::default()
            },
        );
        note_on(&mut hidden, 69);
        assert_eq!(render(&mut hidden, 4_800), single);

        // Two identical oscillators double the amplitude.
        let mut doubled = modulated_engine(super::LfoMode::Trigger, &[]);
        doubled.set_oscillator_count(2);
        note_on(&mut doubled, 69);
        let doubled = render(&mut doubled, 4_800);
        for (a, b) in doubled.iter().zip(&single) {
            assert!((a - 2.0 * b).abs() < 1e-5);
        }
        assert_eq!(doubled.len(), single.len());
    }

    #[test]
    fn each_oscillator_has_independent_controls() {
        // Oscillator 1 silenced, oscillator 2 an octave up at half level:
        // the output is oscillator 2 alone.
        let mut engine = modulated_engine(super::LfoMode::Trigger, &[]);
        engine.set_oscillator_count(2);
        engine.set_oscillator(0, silent_oscillator_settings());
        engine.set_oscillator(
            1,
            super::OscillatorSettings {
                pitch: 12.0,
                level: 0.5,
                ..Default::default()
            },
        );
        note_on(&mut engine, 69);
        let samples = render(&mut engine, 4_800);
        assert_eq!(rising_zero_crossings(&samples), 87);
        let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "{peak}");

        // The waveform and start phase follow oscillator 2's own settings.
        let mut square = modulated_engine(super::LfoMode::Trigger, &[]);
        square.set_oscillator_count(2);
        square.set_oscillator(0, silent_oscillator_settings());
        square.set_oscillator(
            1,
            super::OscillatorSettings {
                waveform: super::Waveform::Square,
                start_phase: 0.25,
                ..Default::default()
            },
        );
        note_on(&mut square, 69);
        let square = render(&mut square, 4_800);
        // Oscillator 1 with the same settings sounds identical.
        let mut reference = modulated_engine(super::LfoMode::Trigger, &[]);
        reference.set_waveform(0, super::Waveform::Square);
        reference.set_start_phase(0, 0.25);
        note_on(&mut reference, 69);
        assert_eq!(square, render(&mut reference, 4_800));
        assert!(rms(&square) > 0.9, "{}", rms(&square));
    }

    #[test]
    fn lfo_routes_target_a_single_oscillator() {
        // Muting oscillator 2's level for the first half-cycle leaves
        // oscillator 1 playing alone.
        let mut engine = modulated_engine(
            super::LfoMode::Trigger,
            &[route(ModDestination::OscLevel(1), -1.0)],
        );
        engine.set_oscillator_count(2);
        note_on(&mut engine, 69);
        let modulated = render(&mut engine, 4_800);

        let mut alone = modulated_engine(super::LfoMode::Trigger, &[]);
        note_on(&mut alone, 69);
        assert_eq!(modulated, render(&mut alone, 4_800));

        // A route to an inactive oscillator changes nothing.
        let mut unused = modulated_engine(
            super::LfoMode::Trigger,
            &[route(ModDestination::OscPitch(3), 0.5)],
        );
        unused.set_oscillator_count(2);
        note_on(&mut unused, 69);
        let mut reference = modulated_engine(super::LfoMode::Trigger, &[]);
        reference.set_oscillator_count(2);
        note_on(&mut reference, 69);
        assert_eq!(render(&mut unused, 4_800), render(&mut reference, 4_800));
    }

    #[test]
    fn oscillator_count_and_index_are_bounded() {
        let mut engine = SynthEngine::default();
        assert_eq!(engine.oscillator_count(), 1);
        engine.set_oscillator_count(0);
        assert_eq!(engine.oscillator_count(), 1);
        engine.set_oscillator_count(99);
        assert_eq!(engine.oscillator_count(), super::MAX_OSCILLATORS);
        // Out-of-range oscillators are ignored rather than panicking.
        engine.set_oscillator(super::MAX_OSCILLATORS, Default::default());
        engine.set_modulation(&[route(ModDestination::OscPitch(99), 1.0)]);
        engine.reset(48_000.0);
        note_on(&mut engine, 69);
        render(&mut engine, 10);
    }
}
