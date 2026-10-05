use std::f32::consts::TAU;

use crate::engine::MidiEvent;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Waveform {
    #[default]
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

const MAX_INCREMENT: f32 = 0.49;
pub const MAX_UNISON_VOICES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnisonSettings {
    pub voices: usize,
    /// Symmetric pitch spread in cents, from 0 to 50 (100 cents per semitone).
    pub detune: f32,
    /// Stereo separation, from 0 to 1.
    pub width: f32,
}

impl Default for UnisonSettings {
    fn default() -> Self {
        Self {
            voices: 1,
            detune: 0.0,
            width: 0.0,
        }
    }
}

#[derive(Debug)]
pub struct Oscillator {
    sample_rate: f32,
    /// Normalized phase in `0.0..1.0`.
    phase: f32,
    unison_phases: [f32; MAX_UNISON_VOICES - 1],
    unison_count: usize,
    /// Normalized phase each note starts from, in `0.0..1.0`.
    start_phase: f32,
    frequency: f32,
    amplitude: f32,
    /// Transposition in semitones and the frequency ratio it gives.
    pitch_semitones: f32,
    pitch_ratio: f32,
    /// Output gain, 0 to 1.
    level: f32,
    waveform: Waveform,
    active_note: Option<u8>,
}

impl Default for Oscillator {
    fn default() -> Self {
        Self {
            sample_rate: 44_100.0,
            phase: 0.0,
            unison_phases: [0.0; MAX_UNISON_VOICES - 1],
            unison_count: 1,
            start_phase: 0.0,
            frequency: 0.0,
            amplitude: 0.0,
            pitch_semitones: 0.0,
            pitch_ratio: 1.0,
            level: 1.0,
            waveform: Waveform::default(),
            active_note: None,
        }
    }
}

impl Oscillator {
    pub fn reset(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.phase = 0.0;
        self.unison_phases.fill(0.0);
        self.unison_count = 1;
        self.frequency = 0.0;
        self.amplitude = 0.0;
        self.active_note = None;
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    /// Set the normalized phase (`0.0..1.0`, i.e. 0 to 360 degrees) that new
    /// notes start from. Sounding notes keep their current phase.
    pub fn set_start_phase(&mut self, phase: f32) {
        self.start_phase = wrap_phase(phase);
    }

    /// Transpose by `semitones`; applies to sounding notes immediately.
    pub fn set_pitch(&mut self, semitones: f32) {
        let semitones = if semitones.is_finite() {
            semitones
        } else {
            0.0
        };
        if semitones != self.pitch_semitones {
            self.pitch_semitones = semitones;
            self.pitch_ratio = (semitones / 12.0).exp2();
        }
    }

    /// Set the output gain (`0.0..=1.0`).
    pub fn set_level(&mut self, level: f32) {
        self.level = if level.is_finite() {
            level.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    pub fn handle_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { note, velocity } if velocity > 0 => {
                self.active_note = Some(note);
                self.frequency = midi_note_frequency(note);
                self.amplitude = f32::from(velocity) / 127.0;
                self.phase = self.start_phase;
                self.unison_phases.fill(self.start_phase);
            }
            MidiEvent::NoteOn { note, .. } | MidiEvent::NoteOff { note } => {
                if self.active_note == Some(note) {
                    self.active_note = None;
                }
            }
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        self.next_stereo_sample(UnisonSettings::default())[0]
    }

    pub fn next_stereo_sample(&mut self, settings: UnisonSettings) -> [f32; 2] {
        if self.amplitude == 0.0 {
            return [0.0; 2];
        }

        let count = settings.voices.clamp(1, MAX_UNISON_VOICES);
        let detune = settings.detune.clamp(0.0, 50.0);
        let width = settings.width.clamp(0.0, 1.0);
        if count > self.unison_count {
            self.unison_phases[self.unison_count - 1..count - 1].fill(self.phase);
        }
        self.unison_count = count;
        let base_increment = self.frequency * self.pitch_ratio / self.sample_rate;
        let gain = self.amplitude * self.level / count as f32;
        let mut output = [0.0; 2];
        let start_phase = self.phase;
        for index in 0..count {
            let position = if count == 1 {
                0.0
            } else {
                2.0 * index as f32 / (count - 1) as f32 - 1.0
            };
            let pan = if 2 * index + 1 == count {
                0.0
            } else if index < count / 2 {
                -width
            } else {
                width
            };
            let phase = if index == 0 {
                &mut self.phase
            } else {
                &mut self.unison_phases[index - 1]
            };
            if detune == 0.0 {
                *phase = start_phase;
            }
            // Clamp each detuned voice below Nyquist, including PolyBLEP shapes.
            let increment =
                (base_increment * (position * detune / 1200.0).exp2()).min(MAX_INCREMENT);
            let sample = waveform_sample(self.waveform, *phase, increment) * gain;
            *phase = (*phase + increment).fract();
            output[0] += sample * (1.0 - pan);
            output[1] += sample * (1.0 + pan);
        }
        output
    }

    pub fn has_active_note(&self) -> bool {
        self.active_note.is_some()
    }

    pub fn stop(&mut self) {
        self.active_note = None;
        self.amplitude = 0.0;
    }

    pub fn active_note(&self) -> Option<u8> {
        self.active_note
    }
}

/// Fills `samples` with one cycle of `waveform` as a note starting at
/// normalized `start_phase` plays it: 0 to 360 degrees inclusive, relative to
/// the start. Uses the same band-limited shape the oscillator plays as if it
/// produced `samples.len() - 1` samples per cycle. Intended for displays, so
/// shapes are computed rather than drawn by hand.
pub fn render_cycle(waveform: Waveform, start_phase: f32, samples: &mut [f32]) {
    let Some(segments) = samples.len().checked_sub(1).filter(|&n| n > 0) else {
        samples.fill(0.0);
        return;
    };
    let start_phase = wrap_phase(start_phase);
    let increment = 1.0 / segments as f32;
    for (index, sample) in samples.iter_mut().enumerate() {
        let phase = (start_phase + index as f32 * increment).fract();
        *sample = waveform_sample(waveform, phase, increment);
    }
}

fn wrap_phase(phase: f32) -> f32 {
    if phase.is_finite() {
        phase.rem_euclid(1.0) % 1.0
    } else {
        0.0
    }
}

/// One sample of `waveform` at normalized `phase` without band-limiting, for
/// sub-audio sources such as LFOs where hard edges are intended.
#[cfg(test)]
pub(crate) fn naive_waveform_sample(waveform: Waveform, phase: f32) -> f32 {
    waveform_sample(waveform, phase, 0.0)
}

/// One sample of `waveform` at normalized `phase`. Every shape is zero and
/// rising at phase 0, so notes with the default start phase begin without a
/// jump.
fn waveform_sample(waveform: Waveform, phase: f32, increment: f32) -> f32 {
    match waveform {
        Waveform::Sine => (TAU * phase).sin(),
        Waveform::Square => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            naive + poly_blep(phase, increment) - poly_blep((phase + 0.5).fract(), increment)
        }
        Waveform::Triangle => 4.0 * ((phase + 0.75).fract() - 0.5).abs() - 1.0,
        Waveform::Sawtooth => {
            // Rising ramp offset by half a cycle so it starts at zero; the
            // falling edge sits at phase 0.5.
            let shifted = (phase + 0.5).fract();
            2.0 * shifted - 1.0 - poly_blep(shifted, increment)
        }
    }
}

/// Polynomial band-limited step correction around a rising discontinuity at
/// phase 0, reducing the aliasing of hard edges.
fn poly_blep(phase: f32, increment: f32) -> f32 {
    if increment <= 0.0 {
        0.0
    } else if phase < increment {
        let t = phase / increment;
        t + t - t * t - 1.0
    } else if phase > 1.0 - increment {
        let t = (phase - 1.0) / increment;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

pub fn midi_note_frequency(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_UNISON_VOICES, Oscillator, TAU, UnisonSettings, Waveform, midi_note_frequency,
        render_cycle, waveform_sample,
    };
    use crate::engine::MidiEvent;

    const WAVEFORMS: [Waveform; 4] = [
        Waveform::Sine,
        Waveform::Square,
        Waveform::Triangle,
        Waveform::Sawtooth,
    ];

    #[test]
    fn rendered_cycle_spans_zero_to_360_degrees() {
        for waveform in WAVEFORMS {
            let mut samples = [0.0; 257];
            render_cycle(waveform, 0.0, &mut samples);
            assert!(
                samples[0].abs() < 1e-5,
                "{waveform:?} starts at {}",
                samples[0]
            );
            assert!(
                samples[256].abs() < 1e-5,
                "{waveform:?} ends at {}",
                samples[256]
            );
            assert!(
                samples.iter().all(|s| s.abs() <= 1.0 + 1e-5),
                "{waveform:?}"
            );
        }
    }

    #[test]
    fn rendered_cycle_matches_the_oscillator() {
        // 128 samples per cycle keeps phase accumulation exact.
        let mut samples = [0.0; 129];
        render_cycle(Waveform::Sawtooth, 0.0, &mut samples);

        let mut oscillator = Oscillator::default();
        oscillator.reset(128.0);
        oscillator.set_waveform(Waveform::Sawtooth);
        oscillator.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        oscillator.frequency = 1.0;
        for (index, expected) in samples[..128].iter().enumerate() {
            let actual = oscillator.next_sample();
            assert!((actual - expected).abs() < 1e-5, "sample {index}");
        }
    }

    #[test]
    fn rendered_cycle_differs_per_waveform() {
        let render = |waveform| {
            let mut samples = [0.0; 65];
            render_cycle(waveform, 0.0, &mut samples);
            samples
        };
        for (index, a) in WAVEFORMS.iter().enumerate() {
            for b in &WAVEFORMS[index + 1..] {
                assert_ne!(render(*a), render(*b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn start_phase_sets_where_notes_begin() {
        let mut oscillator = Oscillator::default();
        oscillator.reset(48_000.0);
        oscillator.set_start_phase(0.25);
        oscillator.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        assert!((oscillator.next_sample() - 1.0).abs() < 1e-5);

        oscillator.set_start_phase(0.75);
        oscillator.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        assert!((oscillator.next_sample() + 1.0).abs() < 1e-5);
    }

    #[test]
    fn start_phase_wraps_into_one_cycle() {
        let mut oscillator = Oscillator::default();
        for (input, expected) in [(1.0, 0.0), (1.25, 0.25), (-0.25, 0.75), (f32::NAN, 0.0)] {
            oscillator.set_start_phase(input);
            assert!((oscillator.start_phase - expected).abs() < 1e-6, "{input}");
        }
    }

    #[test]
    fn rendered_cycle_starts_at_the_start_phase() {
        // 128 samples per cycle keeps phase accumulation exact.
        let mut samples = [0.0; 129];
        render_cycle(Waveform::Square, 0.375, &mut samples);

        let mut oscillator = Oscillator::default();
        oscillator.reset(128.0);
        oscillator.set_waveform(Waveform::Square);
        oscillator.set_start_phase(0.375);
        oscillator.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        oscillator.frequency = 1.0;
        for (index, expected) in samples[..128].iter().enumerate() {
            let actual = oscillator.next_sample();
            assert!((actual - expected).abs() < 1e-5, "sample {index}");
        }
        assert!((samples[0] - samples[128]).abs() < 1e-5);
    }

    #[test]
    fn rendered_cycle_handles_tiny_buffers() {
        let mut empty: [f32; 0] = [];
        render_cycle(Waveform::Sine, 0.0, &mut empty);
        let mut single = [1.0];
        render_cycle(Waveform::Square, 0.0, &mut single);
        assert_eq!(single, [0.0]);
    }

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

    #[test]
    fn waveforms_hit_expected_points() {
        let cases = [
            (Waveform::Sine, [0.0, 1.0, 0.0, -1.0]),
            (Waveform::Square, [0.0, 1.0, 0.0, -1.0]),
            (Waveform::Triangle, [0.0, 1.0, 0.0, -1.0]),
            (Waveform::Sawtooth, [0.0, 0.5, 0.0, -0.5]),
        ];
        for (waveform, expected) in cases {
            for (index, expected) in expected.into_iter().enumerate() {
                let phase = index as f32 * 0.25;
                let actual = waveform_sample(waveform, phase, 0.01);
                // PolyBLEP smooths the square and sawtooth edges to zero.
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "{waveform:?} at phase {phase}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn square_is_flat_between_edges() {
        for phase in [0.1, 0.2, 0.3, 0.4] {
            assert_eq!(waveform_sample(Waveform::Square, phase, 0.01), 1.0);
            assert_eq!(waveform_sample(Waveform::Square, phase + 0.5, 0.01), -1.0);
        }
    }

    #[test]
    fn triangle_is_linear_between_peaks() {
        let a = waveform_sample(Waveform::Triangle, 0.3, 0.01);
        let b = waveform_sample(Waveform::Triangle, 0.4, 0.01);
        let c = waveform_sample(Waveform::Triangle, 0.5, 0.01);
        assert!(((a - b) - (b - c)).abs() < 1e-5);
    }

    #[test]
    fn sawtooth_ramps_up_between_edges() {
        let a = waveform_sample(Waveform::Sawtooth, 0.6, 0.01);
        let b = waveform_sample(Waveform::Sawtooth, 0.7, 0.01);
        let c = waveform_sample(Waveform::Sawtooth, 0.8, 0.01);
        assert!(a < b && b < c);
        assert!(((b - a) - (c - b)).abs() < 1e-5);
    }

    #[test]
    fn every_waveform_stays_bounded_and_has_no_dc() {
        for waveform in [
            Waveform::Sine,
            Waveform::Square,
            Waveform::Triangle,
            Waveform::Sawtooth,
        ] {
            let mut oscillator = Oscillator::default();
            oscillator.reset(48_000.0);
            oscillator.set_waveform(waveform);
            // 480 Hz divides 48 kHz evenly, so 1000 samples are 10 whole cycles.
            oscillator.handle_event(MidiEvent::NoteOn {
                note: 71,
                velocity: 127,
            });
            oscillator.frequency = 480.0;

            let samples: Vec<f32> = (0..1_000).map(|_| oscillator.next_sample()).collect();
            let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
            let mean = samples.iter().sum::<f32>() / samples.len() as f32;
            assert!(peak <= 1.0 + 1e-5, "{waveform:?} peak {peak}");
            assert!(peak > 0.9, "{waveform:?} peak {peak}");
            assert!(mean.abs() < 1e-3, "{waveform:?} DC {mean}");
        }
    }

    #[test]
    fn waveform_changes_the_output() {
        let render = |waveform| {
            let mut oscillator = Oscillator::default();
            oscillator.reset(44_100.0);
            oscillator.set_waveform(waveform);
            oscillator.handle_event(MidiEvent::NoteOn {
                note: 69,
                velocity: 127,
            });
            (0..64)
                .map(|_| oscillator.next_sample())
                .collect::<Vec<_>>()
        };
        assert_ne!(render(Waveform::Sine), render(Waveform::Square));
        assert_ne!(render(Waveform::Sine), render(Waveform::Triangle));
        assert_ne!(render(Waveform::Square), render(Waveform::Triangle));
        assert_ne!(render(Waveform::Sawtooth), render(Waveform::Sine));
        assert_ne!(render(Waveform::Sawtooth), render(Waveform::Square));
        assert_ne!(render(Waveform::Sawtooth), render(Waveform::Triangle));
    }

    fn sine_note(pitch: f32, level: f32) -> Vec<f32> {
        let mut oscillator = Oscillator::default();
        oscillator.reset(48_000.0);
        oscillator.set_waveform(Waveform::Sine);
        oscillator.set_pitch(pitch);
        oscillator.set_level(level);
        oscillator.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        (0..4_800).map(|_| oscillator.next_sample()).collect()
    }

    fn rising_zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count()
    }

    #[test]
    fn pitch_transposes_by_semitones() {
        // 0.1 s of A4 (440 Hz) has 44 cycles; an octave up has 88, a fifth
        // down (293.66 Hz) has 29.
        assert_eq!(rising_zero_crossings(&sine_note(0.0, 1.0)), 43);
        assert_eq!(rising_zero_crossings(&sine_note(12.0, 1.0)), 87);
        assert_eq!(rising_zero_crossings(&sine_note(-7.0, 1.0)), 29);
    }

    #[test]
    fn level_scales_the_output() {
        let full = sine_note(0.0, 1.0);
        let quarter = sine_note(0.0, 0.25);
        for (full, quarter) in full.iter().zip(&quarter) {
            assert!((full * 0.25 - quarter).abs() < 1e-6);
        }
        assert!(sine_note(0.0, 0.0).iter().all(|&s| s == 0.0));
    }
    #[test]
    fn unison_detunes_symmetrically_and_pans_pairs_with_a_centered_odd_voice() {
        for count in 1..=MAX_UNISON_VOICES {
            for width in [0.0, 0.5, 1.0] {
                let settings = UnisonSettings {
                    voices: count,
                    detune: 50.0,
                    width,
                };
                let mut oscillator = Oscillator::default();
                oscillator.reset(48_000.0);
                oscillator.set_start_phase(0.125);
                oscillator.handle_event(MidiEvent::NoteOn {
                    note: 69,
                    velocity: 127,
                });
                let mut phases = vec![0.125; count];
                for _ in 0..2000 {
                    let mut expected = [0.0; 2];
                    for (index, phase) in phases.iter_mut().enumerate() {
                        let offset = if count == 1 {
                            0.0
                        } else {
                            (2 * index) as f32 / (count - 1) as f32 - 1.0
                        };
                        let pan = if 2 * index + 1 == count {
                            0.0
                        } else {
                            if index < count / 2 { -width } else { width }
                        };
                        let sample = (TAU * *phase).sin() / count as f32;
                        expected[0] += sample * (1.0 - pan);
                        expected[1] += sample * (1.0 + pan);
                        *phase = (*phase + 440.0 / 48_000.0 * (offset / 24.0).exp2()).fract();
                    }
                    let actual = oscillator.next_stereo_sample(settings);
                    for channel in 0..2 {
                        assert!(
                            (actual[channel] - expected[channel]).abs() < 1e-5,
                            "{count} voices, width {width}, channel {channel}"
                        );
                    }
                    if width == 0.0 {
                        assert_eq!(actual[0], actual[1]);
                    }
                }
            }
        }
    }

    #[test]
    fn zero_detune_stays_centered_and_preserves_single_oscillator_level() {
        for count in 1..=MAX_UNISON_VOICES {
            let mut reference = Oscillator::default();
            let mut unison = Oscillator::default();
            for oscillator in [&mut reference, &mut unison] {
                oscillator.reset(48_000.0);
                oscillator.handle_event(MidiEvent::NoteOn {
                    note: 69,
                    velocity: 127,
                });
            }
            for _ in 0..1000 {
                let expected = reference.next_sample();
                let actual = unison.next_stereo_sample(UnisonSettings {
                    voices: count,
                    detune: 0.0,
                    width: 1.0,
                });
                assert!((actual[0] - actual[1]).abs() < 1e-6);
                assert!((actual[0] - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn maximum_unison_is_finite_and_bounded_at_high_pitches_for_every_waveform() {
        for waveform in WAVEFORMS {
            let mut oscillator = Oscillator::default();
            oscillator.reset(44_100.0);
            oscillator.set_waveform(waveform);
            oscillator.set_pitch(24.0);
            oscillator.handle_event(MidiEvent::NoteOn {
                note: 127,
                velocity: 127,
            });
            for _ in 0..1000 {
                let sample = oscillator.next_stereo_sample(UnisonSettings {
                    voices: MAX_UNISON_VOICES,
                    detune: 50.0,
                    width: 1.0,
                });
                assert!(sample.iter().all(|s| s.is_finite() && s.abs() <= 1.00001));
            }
        }
    }

    #[test]
    fn unison_retrigger_reset_and_live_count_changes_keep_valid_phases() {
        let mut oscillator = Oscillator::default();
        oscillator.reset(48_000.0);
        oscillator.set_start_phase(0.25);
        let event = MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        };
        let settings = UnisonSettings {
            voices: 20,
            detune: 50.0,
            width: 1.0,
        };
        oscillator.handle_event(event);
        for _ in 0..500 {
            oscillator.next_stereo_sample(settings);
        }
        oscillator.handle_event(event);
        let first = oscillator.next_stereo_sample(settings);
        assert!((first[0] - 1.0).abs() < 1e-6);
        assert!((first[1] - 1.0).abs() < 1e-6);
        for count in [0, 1, 20, 2, usize::MAX, 3] {
            let sample = oscillator.next_stereo_sample(UnisonSettings {
                voices: count,
                ..settings
            });
            assert!(sample.iter().all(|s| s.is_finite() && s.abs() <= 1.00001));
        }
        oscillator.next_stereo_sample(UnisonSettings {
            detune: 0.0,
            ..settings
        });
        assert!(
            oscillator
                .unison_phases
                .iter()
                .all(|&phase| phase == oscillator.phase)
        );
        oscillator.reset(48_000.0);
        assert_eq!(oscillator.next_stereo_sample(settings), [0.0; 2]);
    }
}
