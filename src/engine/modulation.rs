//! LFO modulation routing. A route's depth is a fraction of its
//! destination's control range, so a depth of 0.25 swings the target a
//! quarter of the way across its knob in each direction. The ranges match
//! the plugin's parameter ranges, which gives each destination a natural
//! unit: semitones for pitch, octaves for cutoff, and a ratio for Q.

use super::node::filter::{MAX_CUTOFF_HZ, MAX_Q, MIN_CUTOFF_HZ, MIN_Q};

/// Number of LFO routing slots.
pub const MOD_SLOTS: usize = 4;
/// The oscillator pitch control spans this many semitones either way.
pub const MAX_PITCH_SEMITONES: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModDestination {
    /// Oscillator pitch in semitones.
    OscPitch,
    /// Oscillator level as linear gain, 0 to 1.
    OscLevel,
    /// Filter cutoff in Hz, on a log scale.
    FilterCutoff,
    /// Filter Q, on a log scale.
    FilterQ,
}

impl ModDestination {
    pub const ALL: [Self; 4] = [
        Self::OscPitch,
        Self::OscLevel,
        Self::FilterCutoff,
        Self::FilterQ,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::OscPitch => 0,
            Self::OscLevel => 1,
            Self::FilterCutoff => 2,
            Self::FilterQ => 3,
        }
    }

    /// Where `value` (in the destination's units) sits across its control
    /// range, from 0 to 1.
    pub fn normalize(self, value: f32) -> f32 {
        let normalized = match self {
            Self::OscPitch => (value + MAX_PITCH_SEMITONES) / (2.0 * MAX_PITCH_SEMITONES),
            Self::OscLevel => value,
            Self::FilterCutoff => {
                (value / MIN_CUTOFF_HZ).ln() / (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).ln()
            }
            Self::FilterQ => (value / MIN_Q).ln() / (MAX_Q / MIN_Q).ln(),
        };
        if normalized.is_finite() {
            normalized.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The value (in the destination's units) at `normalized` across the
    /// control range; inverse of `normalize`.
    pub fn denormalize(self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match self {
            Self::OscPitch => (2.0 * normalized - 1.0) * MAX_PITCH_SEMITONES,
            Self::OscLevel => normalized,
            Self::FilterCutoff => MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(normalized),
            Self::FilterQ => MIN_Q * (MAX_Q / MIN_Q).powf(normalized),
        }
    }

    /// `value` moved by `offset` (a fraction of the control range), held
    /// within the range. A zero offset returns `value` untouched.
    pub fn modulate(self, value: f32, offset: f32) -> f32 {
        if offset == 0.0 {
            return value;
        }
        self.denormalize(self.normalize(value) + offset)
    }
}

/// One routing slot: the LFO drives `destination` by `amount`, a signed
/// fraction of the destination's control range (-1 to 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModRoute {
    pub destination: Option<ModDestination>,
    pub amount: f32,
}

/// The total depth applied to each destination; routes sharing a
/// destination add up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModDepths([f32; ModDestination::ALL.len()]);

impl ModDepths {
    pub fn from_routes(routes: &[ModRoute]) -> Self {
        let mut depths = Self::default();
        for route in routes {
            if let Some(destination) = route.destination
                && route.amount.is_finite()
            {
                depths.0[destination.index()] += route.amount.clamp(-1.0, 1.0);
            }
        }
        depths
    }

    pub fn depth(&self, destination: ModDestination) -> f32 {
        self.0[destination.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_denormalize_round_trip() {
        for destination in ModDestination::ALL {
            for normalized in [0.0, 0.2, 0.5, 0.9, 1.0] {
                let value = destination.denormalize(normalized);
                let back = destination.normalize(value);
                assert!(
                    (back - normalized).abs() < 1e-5,
                    "{destination:?}: {normalized} -> {value} -> {back}"
                );
            }
        }
    }

    #[test]
    fn depth_units_follow_each_control_range() {
        // A full-range depth spans 48 semitones of pitch, ~10 octaves of
        // cutoff (20 Hz to 20 kHz), and all of the level.
        let pitch = ModDestination::OscPitch.modulate(0.0, 1.0 / 48.0);
        assert!((pitch - 1.0).abs() < 1e-5);
        let octaves = (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2();
        let cutoff = ModDestination::FilterCutoff.modulate(1_000.0, 1.0 / octaves);
        assert!((cutoff - 2_000.0).abs() < 0.5);
        let level = ModDestination::OscLevel.modulate(0.5, -0.25);
        assert!((level - 0.25).abs() < 1e-6);
    }

    #[test]
    fn modulation_is_clamped_to_the_control_range() {
        assert_eq!(ModDestination::OscLevel.modulate(0.9, 0.5), 1.0);
        assert_eq!(ModDestination::OscPitch.modulate(-20.0, -0.5), -24.0);
        let cutoff = ModDestination::FilterCutoff.modulate(20_000.0, 1.0);
        assert!((cutoff - MAX_CUTOFF_HZ).abs() < 1.0);
        // An unmodulated value passes through even outside the range.
        assert_eq!(ModDestination::FilterQ.modulate(0.707, 0.0), 0.707);
    }

    #[test]
    fn routes_to_the_same_destination_add_up() {
        let depths = ModDepths::from_routes(&[
            ModRoute {
                destination: Some(ModDestination::FilterCutoff),
                amount: 0.25,
            },
            ModRoute {
                destination: None,
                amount: 1.0,
            },
            ModRoute {
                destination: Some(ModDestination::FilterCutoff),
                amount: -0.5,
            },
            ModRoute {
                destination: Some(ModDestination::OscPitch),
                amount: f32::NAN,
            },
        ]);
        assert_eq!(depths.depth(ModDestination::FilterCutoff), -0.25);
        assert_eq!(depths.depth(ModDestination::OscPitch), 0.0);
        assert_eq!(depths.depth(ModDestination::OscLevel), 0.0);
    }
}
