//! LFO modulation routing. A route's depth is a fraction of its
//! destination's control range, so a depth of 0.25 swings the target a
//! quarter of the way across its knob in each direction. The ranges match
//! the plugin's parameter ranges, which gives each destination a natural
//! unit: semitones for pitch, octaves for cutoff, and a ratio for Q.

use super::MAX_OSCILLATORS;
use super::node::filter::{MAX_CUTOFF_HZ, MAX_Q, MIN_CUTOFF_HZ, MIN_Q};

/// Number of LFO routing slots.
pub const MOD_SLOTS: usize = 4;
/// The oscillator pitch control spans this many semitones either way.
pub const MAX_PITCH_SEMITONES: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModDestination {
    /// An oscillator's pitch in semitones, by oscillator index.
    OscPitch(usize),
    /// An oscillator's level as linear gain, 0 to 1, by oscillator index.
    OscLevel(usize),
    /// Filter cutoff in Hz, on a log scale.
    FilterCutoff,
    /// Filter Q, on a log scale.
    FilterQ,
}

impl ModDestination {
    /// Every destination: each oscillator's pitch and level in turn, then
    /// the filter. `index` gives a destination's position here.
    pub const ALL: [Self; 2 * MAX_OSCILLATORS + 2] = {
        let mut all = [Self::FilterCutoff; 2 * MAX_OSCILLATORS + 2];
        let mut oscillator = 0;
        while oscillator < MAX_OSCILLATORS {
            all[2 * oscillator] = Self::OscPitch(oscillator);
            all[2 * oscillator + 1] = Self::OscLevel(oscillator);
            oscillator += 1;
        }
        all[2 * MAX_OSCILLATORS + 1] = Self::FilterQ;
        all
    };

    /// Position in `ALL`. Oscillator indices must be below `MAX_OSCILLATORS`.
    pub fn index(self) -> usize {
        match self {
            Self::OscPitch(oscillator) => 2 * oscillator,
            Self::OscLevel(oscillator) => 2 * oscillator + 1,
            Self::FilterCutoff => 2 * MAX_OSCILLATORS,
            Self::FilterQ => 2 * MAX_OSCILLATORS + 1,
        }
    }

    /// The oscillator this destination belongs to, if any.
    pub fn oscillator(self) -> Option<usize> {
        match self {
            Self::OscPitch(oscillator) | Self::OscLevel(oscillator) => Some(oscillator),
            Self::FilterCutoff | Self::FilterQ => None,
        }
    }

    /// Whether this names a destination in `ALL`.
    pub fn is_valid(self) -> bool {
        self.oscillator()
            .is_none_or(|oscillator| oscillator < MAX_OSCILLATORS)
    }

    /// Where `value` (in the destination's units) sits across its control
    /// range, from 0 to 1.
    pub fn normalize(self, value: f32) -> f32 {
        let normalized = match self {
            Self::OscPitch(_) => (value + MAX_PITCH_SEMITONES) / (2.0 * MAX_PITCH_SEMITONES),
            Self::OscLevel(_) => value,
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
            Self::OscPitch(_) => (2.0 * normalized - 1.0) * MAX_PITCH_SEMITONES,
            Self::OscLevel(_) => normalized,
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
                && destination.is_valid()
            {
                depths.0[destination.index()] += route.amount.clamp(-1.0, 1.0);
            }
        }
        depths
    }

    pub fn depth(&self, destination: ModDestination) -> f32 {
        if destination.is_valid() {
            self.0[destination.index()]
        } else {
            0.0
        }
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
        let pitch = ModDestination::OscPitch(0).modulate(0.0, 1.0 / 48.0);
        assert!((pitch - 1.0).abs() < 1e-5);
        let octaves = (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2();
        let cutoff = ModDestination::FilterCutoff.modulate(1_000.0, 1.0 / octaves);
        assert!((cutoff - 2_000.0).abs() < 0.5);
        let level = ModDestination::OscLevel(0).modulate(0.5, -0.25);
        assert!((level - 0.25).abs() < 1e-6);
    }

    #[test]
    fn modulation_is_clamped_to_the_control_range() {
        assert_eq!(ModDestination::OscLevel(0).modulate(0.9, 0.5), 1.0);
        assert_eq!(ModDestination::OscPitch(0).modulate(-20.0, -0.5), -24.0);
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
                destination: Some(ModDestination::OscPitch(0)),
                amount: f32::NAN,
            },
        ]);
        assert_eq!(depths.depth(ModDestination::FilterCutoff), -0.25);
        assert_eq!(depths.depth(ModDestination::OscPitch(0)), 0.0);
        assert_eq!(depths.depth(ModDestination::OscLevel(0)), 0.0);
    }

    #[test]
    fn every_oscillator_has_its_own_destinations() {
        for (index, destination) in ModDestination::ALL.iter().enumerate() {
            assert_eq!(destination.index(), index);
        }
        assert_eq!(ModDestination::ALL[2], ModDestination::OscPitch(1));
        assert_eq!(ModDestination::OscLevel(3).oscillator(), Some(3));
        assert_eq!(ModDestination::FilterQ.oscillator(), None);

        let depths = ModDepths::from_routes(&[ModRoute {
            destination: Some(ModDestination::OscPitch(2)),
            amount: 0.5,
        }]);
        assert_eq!(depths.depth(ModDestination::OscPitch(2)), 0.5);
        assert_eq!(depths.depth(ModDestination::OscPitch(0)), 0.0);
    }
}
