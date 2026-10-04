//! Two-pole (12 dB per octave) biquad filter using the RBJ "Audio EQ
//! Cookbook" low-pass, high-pass, and constant 0 dB peak band-pass designs.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

pub const MIN_CUTOFF_HZ: f32 = 20.0;
pub const MAX_CUTOFF_HZ: f32 = 20_000.0;
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 20.0;
/// Keeps the cutoff strictly below Nyquist, where the bilinear design breaks
/// down.
const MAX_CUTOFF_FRACTION_OF_SAMPLE_RATE: f32 = 0.49;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterMode {
    #[default]
    LowPass,
    HighPass,
    BandPass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterSettings {
    pub mode: FilterMode,
    pub cutoff_hz: f32,
    pub q: f32,
}

impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            mode: FilterMode::LowPass,
            cutoff_hz: MAX_CUTOFF_HZ,
            q: FRAC_1_SQRT_2,
        }
    }
}

/// Normalized (`a0 == 1`) biquad coefficients.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Default for BiquadCoefficients {
    /// A pass-through filter.
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl BiquadCoefficients {
    /// Designs the filter for `settings` at `sample_rate`. Cutoff and Q are
    /// clamped to their supported ranges and the cutoff is kept below
    /// Nyquist.
    pub fn new(settings: FilterSettings, sample_rate: f32) -> Self {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Self::default();
        }
        let defaults = FilterSettings::default();
        let cutoff = finite_or(settings.cutoff_hz, defaults.cutoff_hz)
            .clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
            .min(sample_rate * MAX_CUTOFF_FRACTION_OF_SAMPLE_RATE);
        let q = finite_or(settings.q, defaults.q).clamp(MIN_Q, MAX_Q);

        let omega = TAU * cutoff / sample_rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * q);

        let (b0, b1, b2) = match settings.mode {
            FilterMode::LowPass => {
                let b = (1.0 - cos) / 2.0;
                (b, 1.0 - cos, b)
            }
            FilterMode::HighPass => {
                let b = (1.0 + cos) / 2.0;
                (b, -(1.0 + cos), b)
            }
            FilterMode::BandPass => (alpha, 0.0, -alpha),
        };
        let a0 = 1.0 + alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Linear magnitude of the filter's response at `frequency`, evaluated
    /// from the same coefficients the filter runs with.
    pub fn magnitude(&self, frequency: f32, sample_rate: f32) -> f32 {
        let omega = TAU * frequency / sample_rate;
        let (sin1, cos1) = omega.sin_cos();
        let (sin2, cos2) = (2.0 * omega).sin_cos();
        // H(z) evaluated at z = e^{j omega}; z^-n = cos(n omega) - j sin(n omega).
        let numerator_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
        let numerator_im = -(self.b1 * sin1 + self.b2 * sin2);
        let denominator_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
        let denominator_im = -(self.a1 * sin1 + self.a2 * sin2);
        (numerator_re.hypot(numerator_im)) / denominator_re.hypot(denominator_im)
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Filter design shared by every voice: the current settings and the
/// coefficients prepared for them. Coefficients are recomputed only when the
/// settings or sample rate change.
#[derive(Debug)]
pub struct Filter {
    sample_rate: f32,
    settings: FilterSettings,
    coefficients: BiquadCoefficients,
}

impl Default for Filter {
    fn default() -> Self {
        let sample_rate = 44_100.0;
        let settings = FilterSettings::default();
        Self {
            sample_rate,
            settings,
            coefficients: BiquadCoefficients::new(settings, sample_rate),
        }
    }
}

impl Filter {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.coefficients = BiquadCoefficients::new(self.settings, sample_rate);
    }

    pub fn set_settings(&mut self, settings: FilterSettings) {
        if settings != self.settings {
            self.settings = settings;
            self.coefficients = BiquadCoefficients::new(settings, self.sample_rate);
        }
    }

    pub fn coefficients(&self) -> &BiquadCoefficients {
        &self.coefficients
    }
}

/// Per-voice filter memory (transposed direct form II).
#[derive(Clone, Copy, Debug, Default)]
pub struct BiquadState {
    s1: f32,
    s2: f32,
}

impl BiquadState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn process_sample(&mut self, coefficients: &BiquadCoefficients, input: f32) -> f32 {
        let c = coefficients;
        let output = c.b0 * input + self.s1;
        self.s1 = c.b1 * input - c.a1 * output + self.s2;
        self.s2 = c.b2 * input - c.a2 * output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn settings(mode: FilterMode, cutoff_hz: f32, q: f32) -> FilterSettings {
        FilterSettings { mode, cutoff_hz, q }
    }

    fn db(magnitude: f32) -> f32 {
        20.0 * magnitude.log10()
    }

    fn response_db(settings: FilterSettings, frequency: f32) -> f32 {
        db(BiquadCoefficients::new(settings, SAMPLE_RATE).magnitude(frequency, SAMPLE_RATE))
    }

    /// Steady-state RMS gain of a sine at `frequency` through the filter.
    fn measured_gain(settings: FilterSettings, frequency: f32) -> f32 {
        let coefficients = BiquadCoefficients::new(settings, SAMPLE_RATE);
        let mut state = BiquadState::default();
        let samples = SAMPLE_RATE as usize;
        let mut input_energy = 0.0;
        let mut output_energy = 0.0;
        for index in 0..samples {
            let input = (TAU * frequency * index as f32 / SAMPLE_RATE).sin();
            let output = state.process_sample(&coefficients, input);
            if index >= samples / 2 {
                input_energy += input * input;
                output_energy += output * output;
            }
        }
        (output_energy / input_energy).sqrt()
    }

    #[test]
    fn butterworth_q_is_3_db_down_at_cutoff() {
        for mode in [FilterMode::LowPass, FilterMode::HighPass] {
            let gain = response_db(settings(mode, 1_000.0, FRAC_1_SQRT_2), 1_000.0);
            assert!((gain + 3.01).abs() < 0.05, "{mode:?} at cutoff: {gain} dB");
        }
    }

    #[test]
    fn low_pass_passes_lows_and_rolls_off_12_db_per_octave() {
        let s = settings(FilterMode::LowPass, 200.0, FRAC_1_SQRT_2);
        assert!(response_db(s, 20.0).abs() < 0.01);
        // Well below Nyquist, where bilinear warping doesn't steepen the slope.
        let slope = response_db(s, 1_600.0) - response_db(s, 3_200.0);
        assert!((slope - 12.0).abs() < 0.3, "slope {slope} dB/octave");
    }

    #[test]
    fn high_pass_passes_highs_and_rolls_off_12_db_per_octave() {
        let s = settings(FilterMode::HighPass, 4_000.0, FRAC_1_SQRT_2);
        assert!(response_db(s, 18_000.0).abs() < 0.2);
        let slope = response_db(s, 500.0) - response_db(s, 250.0);
        assert!((slope - 12.0).abs() < 0.2, "slope {slope} dB/octave");
    }

    #[test]
    fn band_pass_peaks_at_unity_and_rolls_off_both_sides() {
        let s = settings(FilterMode::BandPass, 1_000.0, 2.0);
        assert!(response_db(s, 1_000.0).abs() < 0.01);
        assert!(response_db(s, 100.0) < -20.0);
        assert!(response_db(s, 10_000.0) < -20.0);
        let low_slope = response_db(s, 100.0) - response_db(s, 50.0);
        assert!((low_slope - 6.0).abs() < 0.2, "low slope {low_slope}");
    }

    #[test]
    fn higher_q_narrows_band_pass_and_resonates_low_pass() {
        let wide = settings(FilterMode::BandPass, 1_000.0, 0.5);
        let narrow = settings(FilterMode::BandPass, 1_000.0, 8.0);
        assert!(response_db(narrow, 1_500.0) < response_db(wide, 1_500.0) - 6.0);

        let resonant = settings(FilterMode::LowPass, 1_000.0, 10.0);
        assert!((response_db(resonant, 1_000.0) - 20.0).abs() < 0.1);
    }

    #[test]
    fn processing_matches_the_reported_response() {
        for s in [
            settings(FilterMode::LowPass, 800.0, 2.0),
            settings(FilterMode::HighPass, 800.0, 0.7),
            settings(FilterMode::BandPass, 800.0, 4.0),
        ] {
            for frequency in [200.0, 800.0, 3_000.0] {
                let expected = BiquadCoefficients::new(s, SAMPLE_RATE).magnitude(frequency, SAMPLE_RATE);
                let actual = measured_gain(s, frequency);
                assert!(
                    (actual - expected).abs() < 0.01 * expected.max(0.01),
                    "{s:?} at {frequency} Hz: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn out_of_range_settings_are_clamped() {
        let clamped = BiquadCoefficients::new(settings(FilterMode::LowPass, 1.0, 0.0), SAMPLE_RATE);
        let minimum =
            BiquadCoefficients::new(settings(FilterMode::LowPass, MIN_CUTOFF_HZ, MIN_Q), SAMPLE_RATE);
        assert_eq!(clamped, minimum);

        let invalid = BiquadCoefficients::new(
            settings(FilterMode::LowPass, f32::NAN, f32::INFINITY),
            SAMPLE_RATE,
        );
        assert_eq!(
            invalid,
            BiquadCoefficients::new(FilterSettings::default(), SAMPLE_RATE)
        );
    }

    #[test]
    fn cutoff_stays_below_nyquist_at_low_sample_rates() {
        let coefficients =
            BiquadCoefficients::new(settings(FilterMode::LowPass, 20_000.0, 0.7), 22_050.0);
        let mut state = BiquadState::default();
        for index in 0..10_000 {
            let input = if index % 2 == 0 { 1.0 } else { -1.0 };
            assert!(state.process_sample(&coefficients, input).is_finite());
        }
        assert_eq!(BiquadCoefficients::new(FilterSettings::default(), 0.0), BiquadCoefficients::default());
    }

    #[test]
    fn filter_recomputes_coefficients_when_settings_change() {
        let mut filter = Filter::default();
        filter.prepare(SAMPLE_RATE);
        let initial = *filter.coefficients();
        filter.set_settings(settings(FilterMode::HighPass, 300.0, 1.0));
        assert_ne!(*filter.coefficients(), initial);
        filter.set_settings(FilterSettings::default());
        assert_eq!(*filter.coefficients(), initial);
    }

    #[test]
    fn reset_clears_filter_memory() {
        let coefficients = BiquadCoefficients::new(settings(FilterMode::LowPass, 200.0, 4.0), SAMPLE_RATE);
        let mut state = BiquadState::default();
        state.process_sample(&coefficients, 1.0);
        state.reset();
        assert_eq!(state.process_sample(&coefficients, 0.0), 0.0);
    }
}
