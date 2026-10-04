use super::oscillator::{Waveform, naive_waveform_sample};

pub const MIN_LFO_HZ: f32 = 0.01;
pub const MAX_LFO_HZ: f32 = 30.0;

/// How LFOs relate to notes. The engine applies the mode; an `Lfo` itself
/// only runs between `start` and `stop`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LfoMode {
    /// Each note gets its own LFO, started from phase 0 when the note is
    /// pressed and running until the note finishes sounding.
    #[default]
    Trigger,
    /// One LFO runs continuously, shared by every note.
    Sync,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoSettings {
    pub waveform: Waveform,
    pub frequency_hz: f32,
    pub mode: LfoMode,
}

impl Default for LfoSettings {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            frequency_hz: 1.0,
            mode: LfoMode::Trigger,
        }
    }
}

/// A low-frequency oscillator producing a bipolar (-1 to 1) control signal.
/// Shapes are the oscillator's waveforms without band-limiting.
#[derive(Debug)]
pub struct Lfo {
    sample_rate: f64,
    /// Normalized phase in `0.0..1.0`. Kept in f64 because at 0.01 Hz and
    /// high sample rates the per-sample increment is below f32 resolution.
    phase: f64,
    waveform: Waveform,
    frequency_hz: f32,
    running: bool,
}

impl Default for Lfo {
    fn default() -> Self {
        let settings = LfoSettings::default();
        Self {
            sample_rate: 44_100.0,
            phase: 0.0,
            waveform: settings.waveform,
            frequency_hz: settings.frequency_hz,
            running: false,
        }
    }
}

impl Lfo {
    /// Stops the LFO and rewinds it.
    pub fn reset(&mut self, sample_rate: f32) {
        self.sample_rate = f64::from(sample_rate);
        self.phase = 0.0;
        self.running = false;
    }

    /// Applies shape and rate; the mode is handled by the engine.
    pub fn set_settings(&mut self, settings: LfoSettings) {
        self.waveform = settings.waveform;
        self.frequency_hz = if settings.frequency_hz.is_finite() {
            settings.frequency_hz.clamp(MIN_LFO_HZ, MAX_LFO_HZ)
        } else {
            MIN_LFO_HZ
        };
    }

    /// Starts (or restarts) the cycle from phase 0.
    pub fn start(&mut self) {
        self.phase = 0.0;
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// The current value, then advances one sample. Returns 0 while stopped.
    pub fn next_sample(&mut self) -> f32 {
        if !self.running {
            return 0.0;
        }
        let value = naive_waveform_sample(self.waveform, self.phase as f32);
        let increment = f64::from(self.frequency_hz) / self.sample_rate;
        self.phase = (self.phase + increment).fract();
        value
    }

    /// Normalized position in the cycle, `0.0..1.0`.
    pub fn phase(&self) -> f32 {
        self.phase as f32
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

/// Fills `samples` with one LFO cycle of `waveform`, 0 to 360 degrees
/// inclusive, using the same function the LFO runs. Intended for displays.
pub fn render_lfo_cycle(waveform: Waveform, samples: &mut [f32]) {
    let Some(segments) = samples.len().checked_sub(1).filter(|&n| n > 0) else {
        samples.fill(0.0);
        return;
    };
    for (index, sample) in samples.iter_mut().enumerate() {
        let phase = (index as f32 / segments as f32).fract();
        *sample = naive_waveform_sample(waveform, phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_lfo(waveform: Waveform, frequency_hz: f32, sample_rate: f32) -> Lfo {
        let mut lfo = Lfo::default();
        lfo.reset(sample_rate);
        lfo.set_settings(LfoSettings {
            waveform,
            frequency_hz,
            mode: LfoMode::Trigger,
        });
        lfo.start();
        lfo
    }

    #[test]
    fn completes_one_cycle_per_period() {
        let mut lfo = running_lfo(Waveform::Sine, 2.0, 1_000.0);
        let samples: Vec<f32> = (0..500).map(|_| lfo.next_sample()).collect();
        assert!(samples[0].abs() < 1e-6);
        assert!((samples[125] - 1.0).abs() < 1e-4);
        assert!((samples[375] + 1.0).abs() < 1e-4);
        assert!(lfo.phase() < 1e-6 || lfo.phase() > 1.0 - 1e-6);
    }

    #[test]
    fn shapes_are_unfiltered() {
        let mut square = running_lfo(Waveform::Square, 1.0, 100.0);
        let values: Vec<f32> = (0..100).map(|_| square.next_sample()).collect();
        assert!(values[1..50].iter().all(|&v| v == 1.0));
        assert!(values[50..].iter().all(|&v| v == -1.0));

        let mut saw = running_lfo(Waveform::Sawtooth, 1.0, 100.0);
        let values: Vec<f32> = (0..100).map(|_| saw.next_sample()).collect();
        assert!((values[49] - 0.98).abs() < 1e-4);
        assert!((values[50] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn rate_is_clamped_to_its_range() {
        for (input, expected) in [
            (0.0, MIN_LFO_HZ),
            (100.0, MAX_LFO_HZ),
            (f32::NAN, MIN_LFO_HZ),
            (5.0, 5.0),
        ] {
            let lfo = running_lfo(Waveform::Sine, input, 48_000.0);
            assert_eq!(lfo.frequency_hz, expected, "{input}");
        }
    }

    #[test]
    fn slowest_rate_still_advances_at_high_sample_rates() {
        let mut lfo = running_lfo(Waveform::Sine, MIN_LFO_HZ, 192_000.0);
        for _ in 0..192_000 {
            lfo.next_sample();
        }
        assert!((lfo.phase() - 0.01).abs() < 1e-6, "{}", lfo.phase());
    }

    #[test]
    fn runs_only_between_start_and_stop() {
        let mut lfo = Lfo::default();
        lfo.reset(1_000.0);
        lfo.set_settings(LfoSettings {
            frequency_hz: 10.0,
            ..LfoSettings::default()
        });
        assert!(!lfo.is_running());
        assert_eq!(lfo.next_sample(), 0.0);
        assert_eq!(lfo.phase(), 0.0);

        lfo.start();
        for _ in 0..30 {
            lfo.next_sample();
        }
        assert!((lfo.phase() - 0.3).abs() < 1e-6);

        lfo.start();
        assert_eq!(lfo.phase(), 0.0);

        lfo.stop();
        assert!(!lfo.is_running());
        lfo.next_sample();
        assert_eq!(lfo.phase(), 0.0);
    }

    #[test]
    fn rendered_cycle_matches_the_lfo() {
        for waveform in [
            Waveform::Sine,
            Waveform::Square,
            Waveform::Triangle,
            Waveform::Sawtooth,
        ] {
            let mut samples = [0.0; 65];
            render_lfo_cycle(waveform, &mut samples);
            let mut lfo = running_lfo(waveform, 1.0, 64.0);
            for (index, expected) in samples[..64].iter().enumerate() {
                let actual = lfo.next_sample();
                assert!((actual - expected).abs() < 1e-5, "{waveform:?} {index}");
            }
        }
    }
}
