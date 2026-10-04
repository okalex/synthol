use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdsrSettings {
    pub attack: Duration,
    pub decay: Duration,
    pub sustain_db: f32,
    pub release: Duration,
}

impl Default for AdsrSettings {
    fn default() -> Self {
        Self {
            attack: Duration::from_millis(10),
            decay: Duration::from_millis(500),
            sustain_db: -6.0,
            release: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug)]
pub struct AdsrEnvelope {
    settings: AdsrSettings,
    sample_rate: f32,
    attack_samples: usize,
    decay_samples: usize,
    release_samples: usize,
    sustain_level: f32,
    stage: Stage,
    stage_position: usize,
    level: f32,
    release_start_level: f32,
}

impl AdsrEnvelope {
    pub fn new(settings: AdsrSettings, sample_rate: f32) -> Self {
        let mut envelope = Self {
            settings,
            sample_rate,
            attack_samples: 0,
            decay_samples: 0,
            release_samples: 0,
            sustain_level: decibels_to_linear(settings.sustain_db),
            stage: Stage::Idle,
            stage_position: 0,
            level: 0.0,
            release_start_level: 0.0,
        };
        envelope.prepare(sample_rate);
        envelope
    }

    pub fn prepare(&mut self, sample_rate: f32) -> bool {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        self.sample_rate = sample_rate;
        self.update_timing();
        self.reset();
        true
    }

    pub fn set_settings(&mut self, settings: AdsrSettings) {
        if self.settings == settings {
            return;
        }
        self.settings = settings;
        self.update_timing();
    }

    fn update_timing(&mut self) {
        self.attack_samples = duration_samples(self.settings.attack, self.sample_rate);
        self.decay_samples = duration_samples(self.settings.decay, self.sample_rate);
        self.release_samples = duration_samples(self.settings.release, self.sample_rate);
        self.sustain_level = decibels_to_linear(self.settings.sustain_db);
    }

    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.stage_position = 0;
        self.level = 0.0;
        self.release_start_level = 0.0;
    }

    pub fn note_on(&mut self) {
        self.stage = Stage::Attack;
        self.stage_position = 0;
        self.level = 0.0;
        self.release_start_level = 0.0;
    }

    pub fn note_off(&mut self) {
        if self.stage == Stage::Idle || self.stage == Stage::Release {
            return;
        }

        self.release_start_level = self.level;
        self.stage_position = 0;
        if self.release_samples == 0 || self.release_start_level == 0.0 {
            self.stage = Stage::Idle;
            self.level = 0.0;
        } else {
            self.stage = Stage::Release;
        }
    }

    pub fn process_sample(&mut self, input: f32) -> f32 {
        self.advance();
        input * self.level
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    fn advance(&mut self) {
        loop {
            match self.stage {
                Stage::Idle => return,
                Stage::Attack if self.attack_samples == 0 => {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                    self.stage_position = 0;
                }
                Stage::Attack => {
                    self.stage_position += 1;
                    self.level = self.stage_position as f32 / self.attack_samples as f32;
                    if self.stage_position >= self.attack_samples {
                        self.level = 1.0;
                        self.stage = Stage::Decay;
                        self.stage_position = 0;
                    }
                    return;
                }
                Stage::Decay if self.decay_samples == 0 => {
                    self.level = self.sustain_level;
                    self.stage = Stage::Sustain;
                    self.stage_position = 0;
                }
                Stage::Decay => {
                    self.stage_position += 1;
                    let progress = self.stage_position as f32 / self.decay_samples as f32;
                    self.level = 1.0 + (self.sustain_level - 1.0) * progress;
                    if self.stage_position >= self.decay_samples {
                        self.level = self.sustain_level;
                        self.stage = Stage::Sustain;
                        self.stage_position = 0;
                    }
                    return;
                }
                Stage::Sustain => {
                    self.level = self.sustain_level;
                    return;
                }
                Stage::Release => {
                    self.stage_position += 1;
                    let progress = self.stage_position as f32 / self.release_samples as f32;
                    self.level = self.release_start_level * (1.0 - progress);
                    if self.stage_position >= self.release_samples {
                        self.level = 0.0;
                        self.stage = Stage::Idle;
                        self.stage_position = 0;
                    }
                    return;
                }
            }
        }
    }
}

fn duration_samples(duration: Duration, sample_rate: f32) -> usize {
    (duration.as_secs_f64() * f64::from(sample_rate)).round() as usize
}

fn decibels_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

impl Default for AdsrEnvelope {
    fn default() -> Self {
        Self::new(AdsrSettings::default(), 44_100.0)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{AdsrEnvelope, AdsrSettings};

    const SAMPLE_RATE: f32 = 44_100.0;

    #[test]
    fn default_settings_match_requested_adsr() {
        let settings = AdsrSettings::default();
        assert_eq!(settings.attack, Duration::from_millis(10));
        assert_eq!(settings.decay, Duration::from_millis(500));
        assert_eq!(settings.sustain_db, -6.0);
        assert_eq!(settings.release, Duration::from_secs(1));
    }

    #[test]
    fn attack_reaches_peak_after_configured_time() {
        let mut envelope = AdsrEnvelope::new(AdsrSettings::default(), SAMPLE_RATE);
        envelope.note_on();

        for _ in 0..440 {
            envelope.process_sample(1.0);
        }
        assert!(envelope.level() < 1.0);

        envelope.process_sample(1.0);
        assert_eq!(envelope.level(), 1.0);
    }

    #[test]
    fn decay_reaches_minus_six_db_sustain_after_configured_time() {
        let mut envelope = AdsrEnvelope::new(AdsrSettings::default(), SAMPLE_RATE);
        envelope.note_on();

        for _ in 0..(441 + 22_049) {
            envelope.process_sample(1.0);
        }
        assert!(envelope.level() > 0.5);

        envelope.process_sample(1.0);
        assert!((envelope.level() - 10.0_f32.powf(-6.0 / 20.0)).abs() < 1e-5);
    }

    #[test]
    fn release_reaches_silence_after_configured_time() {
        let mut envelope = AdsrEnvelope::new(AdsrSettings::default(), SAMPLE_RATE);
        envelope.note_on();

        for _ in 0..(441 + 22_050) {
            envelope.process_sample(1.0);
        }
        envelope.note_off();
        assert!(envelope.is_active());

        for _ in 0..44_099 {
            envelope.process_sample(1.0);
        }
        assert!(envelope.level() > 0.0);

        envelope.process_sample(1.0);
        assert_eq!(envelope.level(), 0.0);
        assert!(!envelope.is_active());
    }

    #[test]
    fn release_uses_the_current_level() {
        let mut envelope = AdsrEnvelope::new(AdsrSettings::default(), SAMPLE_RATE);
        envelope.note_on();
        for _ in 0..100 {
            envelope.process_sample(1.0);
        }
        let level_before_release = envelope.level();

        envelope.note_off();
        envelope.process_sample(1.0);

        assert!(envelope.level() < level_before_release);
        assert!(envelope.level() > 0.0);
    }

    #[test]
    fn settings_update_the_active_envelope() {
        let mut envelope = AdsrEnvelope::new(
            AdsrSettings {
                attack: Duration::ZERO,
                decay: Duration::ZERO,
                sustain_db: -6.0,
                release: Duration::from_secs(1),
            },
            SAMPLE_RATE,
        );
        envelope.note_on();
        envelope.process_sample(1.0);
        let original_sustain = envelope.level();

        envelope.set_settings(AdsrSettings {
            sustain_db: -12.0,
            ..AdsrSettings {
                attack: Duration::ZERO,
                decay: Duration::ZERO,
                sustain_db: -6.0,
                release: Duration::from_secs(1),
            }
        });
        envelope.process_sample(1.0);

        assert!(envelope.level() < original_sustain);
        assert!((envelope.level() - 10.0_f32.powf(-12.0 / 20.0)).abs() < 1e-5);
    }

    #[test]
    fn invalid_sample_rate_does_not_replace_the_prepared_rate() {
        let mut envelope = AdsrEnvelope::new(
            AdsrSettings {
                attack: Duration::from_millis(10),
                ..AdsrSettings::default()
            },
            1_000.0,
        );

        assert!(!envelope.prepare(0.0));
        assert!(!envelope.prepare(f32::NAN));
        envelope.note_on();
        for _ in 0..10 {
            envelope.process_sample(1.0);
        }
        assert_eq!(envelope.level(), 1.0);
    }

    #[test]
    fn zero_velocity_retrigger_can_be_released_immediately() {
        let mut envelope = AdsrEnvelope::new(AdsrSettings::default(), SAMPLE_RATE);
        envelope.note_on();
        envelope.note_off();
        assert!(!envelope.is_active());
    }
}
