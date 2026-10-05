use super::node::filter::FilterSettings;

pub const MAX_EFFECTS: usize = 32;

/// Stable slot identities in signal-flow order. Settings and modulation stay
/// attached to a slot rather than its position in the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectChain {
    slots: [usize; MAX_EFFECTS],
    len: usize,
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new(&[0])
    }
}

impl EffectChain {
    pub fn new(slots: &[usize]) -> Self {
        assert!(slots.len() <= MAX_EFFECTS);
        let mut chain = Self {
            slots: [0; MAX_EFFECTS],
            len: slots.len(),
        };
        for (position, &slot) in slots.iter().enumerate() {
            assert!(slot < MAX_EFFECTS && !slots[..position].contains(&slot));
            chain.slots[position] = slot;
        }
        chain
    }

    pub fn slots(&self) -> &[usize] {
        &self.slots[..self.len]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EffectControls {
    pub chain: EffectChain,
    pub filters: [FilterSettings; MAX_EFFECTS],
}

impl Default for EffectControls {
    fn default() -> Self {
        Self {
            chain: EffectChain::default(),
            filters: [FilterSettings::default(); MAX_EFFECTS],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::filter::{BiquadCoefficients, BiquadState};
    use crate::engine::{FilterMode, MidiEvent, ModDestination, ModRoute, SynthEngine};

    fn engine(chain: &[usize]) -> SynthEngine {
        let mut engine = SynthEngine::default();
        engine.reset(48_000.0);
        engine.set_envelope_count(0);
        engine.set_effect_chain(EffectChain::new(chain));
        engine.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        engine
    }

    #[test]
    fn effects_chain_matches_serial_processing_and_preserves_state_on_reorder() {
        let mut dry = engine(&[]);
        let mut synth = engine(&[0, 31]);
        let mut states = [BiquadState::default(), BiquadState::default()];
        for sample in 0..1_200 {
            let settings = [
                FilterSettings {
                    cutoff_hz: 700.0 + sample as f32,
                    q: 1.3,
                    ..FilterSettings::default()
                },
                FilterSettings {
                    mode: FilterMode::HighPass,
                    cutoff_hz: 250.0,
                    mix: 0.7,
                    ..FilterSettings::default()
                },
            ];
            synth.set_effect_filter(0, settings[0]);
            synth.set_effect_filter(31, settings[1]);
            let order = if sample < 600 { [0, 1] } else { [1, 0] };
            if sample == 600 {
                synth.set_effect_chain(EffectChain::new(&[31, 0]));
            }
            let mut expected = dry.next_sample(1.0);
            for index in order {
                expected = states[index].process_sample(
                    &BiquadCoefficients::new(settings[index], 48_000.0),
                    expected,
                );
            }
            assert_eq!(synth.next_sample(1.0), expected, "sample {sample}");
        }
    }

    #[test]
    fn effects_chain_supports_empty_and_all_32_slots() {
        let mut dry = engine(&[]);
        let slots: Vec<_> = (0..MAX_EFFECTS).rev().collect();
        let mut bypass = engine(&slots);
        for slot in 0..MAX_EFFECTS {
            bypass.set_effect_filter(
                slot,
                FilterSettings {
                    mix: 0.0,
                    ..FilterSettings::default()
                },
            );
        }
        for _ in 0..1_000 {
            assert_eq!(bypass.next_sample(1.0), dry.next_sample(1.0));
        }
    }

    #[test]
    fn effects_chain_modulates_only_the_target_filter_identity() {
        let mut dry = engine(&[]);
        let mut synth = engine(&[31, 0]);
        let settings = FilterSettings {
            cutoff_hz: 1_000.0,
            ..FilterSettings::default()
        };
        synth.set_effect_filter(
            0,
            FilterSettings {
                mix: 0.0,
                ..settings
            },
        );
        synth.set_effect_filter(31, settings);
        synth.set_envelope_count(1);
        synth.set_envelope(
            0,
            crate::engine::node::envelope::AdsrSettings {
                attack: std::time::Duration::ZERO,
                decay: std::time::Duration::ZERO,
                sustain_db: 0.0,
                release: std::time::Duration::ZERO,
            },
        );
        synth.set_envelope_modulation(
            0,
            &[ModRoute {
                destination: Some(ModDestination::EffectCutoff(31)),
                amount: 0.1,
            }],
        );
        synth.handle_event(MidiEvent::NoteOn {
            note: 69,
            velocity: 127,
        });
        let coefficients = BiquadCoefficients::new(
            FilterSettings {
                cutoff_hz: ModDestination::EffectCutoff(31).modulate(settings.cutoff_hz, 0.1),
                ..settings
            },
            48_000.0,
        );
        let mut state = BiquadState::default();
        for _ in 0..100 {
            let expected = state.process_sample(&coefficients, dry.next_sample(1.0));
            assert_eq!(synth.next_sample(1.0), expected);
        }
    }

    #[test]
    #[should_panic]
    fn effects_chain_rejects_duplicate_identities() {
        EffectChain::new(&[1, 1]);
    }
}
