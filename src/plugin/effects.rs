use truce::prelude::*;

use super::{FilterType, SynthParams, SynthParamsParamId};
use crate::engine::{EffectChain, FilterSettings, MAX_EFFECTS};

pub struct EffectParams<'a> {
    pub kind: &'a EnumParam<FilterType>,
    pub cutoff: &'a FloatParam,
    pub q: &'a FloatParam,
    pub mix: &'a FloatParam,
    pub enabled: &'a BoolParam,
    pub order: &'a IntParam,
}

macro_rules! filter_params {
    ($ty:ident, $kind:literal, $cutoff:literal, $q:literal, $mix:literal, $enabled:literal, $order:literal) => {
        #[derive(Params)]
        pub struct $ty {
            #[param(id = 0, name = $kind, default = 0)]
            pub kind: EnumParam<FilterType>,
            #[param(id = 1, name = $cutoff, range = "log(20, 20000)", default = 20000.0, unit = "Hz", smooth = "log(20)")]
            pub cutoff: FloatParam,
            #[param(id = 2, name = $q, range = "log(0.1, 20)", default = 0.707, smooth = "exp(20)")]
            pub q: FloatParam,
            #[param(id = 3, name = $mix, range = "linear(0, 100)", default = 100.0, unit = "%", smooth = "exp(20)")]
            pub mix: FloatParam,
            #[param(id = 4, name = $enabled, default = false)]
            pub enabled: BoolParam,
            #[param(id = 5, name = $order, range = "discrete(0, 31)", default = 0)]
            pub order: IntParam,
        }
        impl $ty {
            pub fn controls(&self) -> EffectParams<'_> {
                EffectParams {
                    kind: &self.kind, cutoff: &self.cutoff, q: &self.q,
                    mix: &self.mix, enabled: &self.enabled, order: &self.order,
                }
            }
        }
    };
}

filter_params!(
    Filter2Params,
    "Filter 2 Type",
    "Filter 2 Cutoff",
    "Filter 2 Q",
    "Filter 2 Mix",
    "Filter 2 Enabled",
    "Filter 2 Order"
);
filter_params!(
    Filter3Params,
    "Filter 3 Type",
    "Filter 3 Cutoff",
    "Filter 3 Q",
    "Filter 3 Mix",
    "Filter 3 Enabled",
    "Filter 3 Order"
);
filter_params!(
    Filter4Params,
    "Filter 4 Type",
    "Filter 4 Cutoff",
    "Filter 4 Q",
    "Filter 4 Mix",
    "Filter 4 Enabled",
    "Filter 4 Order"
);
filter_params!(
    Filter5Params,
    "Filter 5 Type",
    "Filter 5 Cutoff",
    "Filter 5 Q",
    "Filter 5 Mix",
    "Filter 5 Enabled",
    "Filter 5 Order"
);
filter_params!(
    Filter6Params,
    "Filter 6 Type",
    "Filter 6 Cutoff",
    "Filter 6 Q",
    "Filter 6 Mix",
    "Filter 6 Enabled",
    "Filter 6 Order"
);
filter_params!(
    Filter7Params,
    "Filter 7 Type",
    "Filter 7 Cutoff",
    "Filter 7 Q",
    "Filter 7 Mix",
    "Filter 7 Enabled",
    "Filter 7 Order"
);
filter_params!(
    Filter8Params,
    "Filter 8 Type",
    "Filter 8 Cutoff",
    "Filter 8 Q",
    "Filter 8 Mix",
    "Filter 8 Enabled",
    "Filter 8 Order"
);
filter_params!(
    Filter9Params,
    "Filter 9 Type",
    "Filter 9 Cutoff",
    "Filter 9 Q",
    "Filter 9 Mix",
    "Filter 9 Enabled",
    "Filter 9 Order"
);
filter_params!(
    Filter10Params,
    "Filter 10 Type",
    "Filter 10 Cutoff",
    "Filter 10 Q",
    "Filter 10 Mix",
    "Filter 10 Enabled",
    "Filter 10 Order"
);
filter_params!(
    Filter11Params,
    "Filter 11 Type",
    "Filter 11 Cutoff",
    "Filter 11 Q",
    "Filter 11 Mix",
    "Filter 11 Enabled",
    "Filter 11 Order"
);
filter_params!(
    Filter12Params,
    "Filter 12 Type",
    "Filter 12 Cutoff",
    "Filter 12 Q",
    "Filter 12 Mix",
    "Filter 12 Enabled",
    "Filter 12 Order"
);
filter_params!(
    Filter13Params,
    "Filter 13 Type",
    "Filter 13 Cutoff",
    "Filter 13 Q",
    "Filter 13 Mix",
    "Filter 13 Enabled",
    "Filter 13 Order"
);
filter_params!(
    Filter14Params,
    "Filter 14 Type",
    "Filter 14 Cutoff",
    "Filter 14 Q",
    "Filter 14 Mix",
    "Filter 14 Enabled",
    "Filter 14 Order"
);
filter_params!(
    Filter15Params,
    "Filter 15 Type",
    "Filter 15 Cutoff",
    "Filter 15 Q",
    "Filter 15 Mix",
    "Filter 15 Enabled",
    "Filter 15 Order"
);
filter_params!(
    Filter16Params,
    "Filter 16 Type",
    "Filter 16 Cutoff",
    "Filter 16 Q",
    "Filter 16 Mix",
    "Filter 16 Enabled",
    "Filter 16 Order"
);
filter_params!(
    Filter17Params,
    "Filter 17 Type",
    "Filter 17 Cutoff",
    "Filter 17 Q",
    "Filter 17 Mix",
    "Filter 17 Enabled",
    "Filter 17 Order"
);
filter_params!(
    Filter18Params,
    "Filter 18 Type",
    "Filter 18 Cutoff",
    "Filter 18 Q",
    "Filter 18 Mix",
    "Filter 18 Enabled",
    "Filter 18 Order"
);
filter_params!(
    Filter19Params,
    "Filter 19 Type",
    "Filter 19 Cutoff",
    "Filter 19 Q",
    "Filter 19 Mix",
    "Filter 19 Enabled",
    "Filter 19 Order"
);
filter_params!(
    Filter20Params,
    "Filter 20 Type",
    "Filter 20 Cutoff",
    "Filter 20 Q",
    "Filter 20 Mix",
    "Filter 20 Enabled",
    "Filter 20 Order"
);
filter_params!(
    Filter21Params,
    "Filter 21 Type",
    "Filter 21 Cutoff",
    "Filter 21 Q",
    "Filter 21 Mix",
    "Filter 21 Enabled",
    "Filter 21 Order"
);
filter_params!(
    Filter22Params,
    "Filter 22 Type",
    "Filter 22 Cutoff",
    "Filter 22 Q",
    "Filter 22 Mix",
    "Filter 22 Enabled",
    "Filter 22 Order"
);
filter_params!(
    Filter23Params,
    "Filter 23 Type",
    "Filter 23 Cutoff",
    "Filter 23 Q",
    "Filter 23 Mix",
    "Filter 23 Enabled",
    "Filter 23 Order"
);
filter_params!(
    Filter24Params,
    "Filter 24 Type",
    "Filter 24 Cutoff",
    "Filter 24 Q",
    "Filter 24 Mix",
    "Filter 24 Enabled",
    "Filter 24 Order"
);
filter_params!(
    Filter25Params,
    "Filter 25 Type",
    "Filter 25 Cutoff",
    "Filter 25 Q",
    "Filter 25 Mix",
    "Filter 25 Enabled",
    "Filter 25 Order"
);
filter_params!(
    Filter26Params,
    "Filter 26 Type",
    "Filter 26 Cutoff",
    "Filter 26 Q",
    "Filter 26 Mix",
    "Filter 26 Enabled",
    "Filter 26 Order"
);
filter_params!(
    Filter27Params,
    "Filter 27 Type",
    "Filter 27 Cutoff",
    "Filter 27 Q",
    "Filter 27 Mix",
    "Filter 27 Enabled",
    "Filter 27 Order"
);
filter_params!(
    Filter28Params,
    "Filter 28 Type",
    "Filter 28 Cutoff",
    "Filter 28 Q",
    "Filter 28 Mix",
    "Filter 28 Enabled",
    "Filter 28 Order"
);
filter_params!(
    Filter29Params,
    "Filter 29 Type",
    "Filter 29 Cutoff",
    "Filter 29 Q",
    "Filter 29 Mix",
    "Filter 29 Enabled",
    "Filter 29 Order"
);
filter_params!(
    Filter30Params,
    "Filter 30 Type",
    "Filter 30 Cutoff",
    "Filter 30 Q",
    "Filter 30 Mix",
    "Filter 30 Enabled",
    "Filter 30 Order"
);
filter_params!(
    Filter31Params,
    "Filter 31 Type",
    "Filter 31 Cutoff",
    "Filter 31 Q",
    "Filter 31 Mix",
    "Filter 31 Enabled",
    "Filter 31 Order"
);
filter_params!(
    Filter32Params,
    "Filter 32 Type",
    "Filter 32 Cutoff",
    "Filter 32 Q",
    "Filter 32 Mix",
    "Filter 32 Enabled",
    "Filter 32 Order"
);

/// Explicit nested bases reserve six IDs for each additional filter.
pub fn effect_ids(slot: usize) -> [u32; 6] {
    assert!(slot < MAX_EFFECTS);
    if slot == 0 {
        [
            SynthParamsParamId::FilterType.into(),
            SynthParamsParamId::FilterCutoff.into(),
            SynthParamsParamId::FilterQ.into(),
            SynthParamsParamId::FilterMix.into(),
            SynthParamsParamId::FilterEnabled.into(),
            SynthParamsParamId::FilterOrder.into(),
        ]
    } else {
        std::array::from_fn(|index| 10_000 + 6 * (slot - 1) as u32 + index as u32)
    }
}

impl SynthParams {
    pub fn effect_chain(&self) -> EffectChain {
        let extra = self.extra_effects();
        let mut slots = [0; MAX_EFFECTS];
        let mut len = 0;
        for slot in 0..MAX_EFFECTS {
            let enabled = if slot == 0 {
                self.filter_enabled.value()
            } else {
                extra[slot - 1].enabled.value()
            };
            if enabled {
                slots[len] = slot;
                len += 1;
            }
        }
        slots[..len].sort_unstable_by_key(|&slot| {
            let order = if slot == 0 {
                self.filter_order.value_usize()
            } else {
                extra[slot - 1].order.value_usize()
            };
            (order, slot)
        });
        EffectChain::new(&slots[..len])
    }

    pub fn read_effect_filters(&self) -> [FilterSettings; MAX_EFFECTS] {
        let extra = self.extra_effects();
        std::array::from_fn(|slot| {
            if slot == 0 {
                FilterSettings {
                    mode: self.filter_type.value().into(),
                    cutoff_hz: self.filter_cutoff.read(),
                    q: self.filter_q.read(),
                    mix: self.filter_mix.read() / 100.0,
                }
            } else {
                let params = &extra[slot - 1];
                FilterSettings {
                    mode: params.kind.value().into(),
                    cutoff_hz: params.cutoff.read(),
                    q: params.q.read(),
                    mix: params.mix.read() / 100.0,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ModDestination;
    use crate::patch::Patch;
    use crate::plugin::{ModDestinationType, mod_destination_from_index, mod_destination_index};

    #[test]
    fn effects_chain_parameters_have_unique_ids_names_and_static_metadata() {
        let params = SynthParams::default();
        let metadata = |infos: Vec<truce::params::ParamInfo>| {
            infos
                .into_iter()
                .map(|info| (info.id, info.name))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            metadata(params.param_infos()),
            metadata(SynthParams::param_infos_static())
        );
        let infos = params.param_infos();
        for slot in 0..MAX_EFFECTS {
            for id in effect_ids(slot) {
                assert_eq!(infos.iter().filter(|info| info.id == id).count(), 1);
            }
            if slot > 0 {
                let info = infos
                    .iter()
                    .find(|info| info.id == effect_ids(slot)[1])
                    .unwrap();
                assert_eq!(info.name, format!("Filter {} Cutoff", slot + 1));
            }
        }
        assert_eq!(params.effect_chain().slots(), &[0]);
        for slot in 0..MAX_EFFECTS {
            for destination in ModDestination::filter_destinations(slot) {
                assert_eq!(
                    mod_destination_from_index(mod_destination_index(Some(destination))),
                    Some(destination.with_effect(0))
                );
            }
        }
        assert_eq!(ModDestinationType::variant_count(), 16);
    }

    #[test]
    fn effects_chain_routes_keep_original_automation_and_target_filter_identities() {
        let params = SynthParams::default();
        for ids in crate::plugin::LFO_PARAMS {
            params.set_plain(ids.destinations[0].into(), 3.0);
            params.set_plain(ids.filters[0].into(), 31.0);
            assert_eq!(
                params.get_normalized(ids.destinations[0].into()),
                Some(3.0 / 15.0)
            );
        }
        for ids in crate::plugin::ENV_PARAMS {
            params.set_plain(ids.destinations[0].into(), 4.0);
            params.set_plain(ids.filters[0].into(), 31.0);
            assert_eq!(
                params.get_normalized(ids.destinations[0].into()),
                Some(4.0 / 15.0)
            );
        }
        for routes in params.mod_routes() {
            assert_eq!(
                routes[0].destination,
                Some(ModDestination::EffectCutoff(31))
            );
        }
        for routes in params.envelope_routes() {
            assert_eq!(routes[0].destination, Some(ModDestination::EffectQ(31)));
        }
        let restored = SynthParams::default();
        let (ids, values) = params.collect_values();
        restored.restore_values(&ids.into_iter().zip(values).collect::<Vec<_>>());
        assert_eq!(restored.mod_routes(), params.mod_routes());
        assert_eq!(restored.envelope_routes(), params.envelope_routes());
    }

    #[test]
    fn effects_chain_round_trips_host_values_and_patch_json_including_empty() {
        let params = SynthParams::default();
        for slot in 0..MAX_EFFECTS {
            let ids = effect_ids(slot);
            params.set_plain(ids[0], (slot % 3) as f64);
            params.set_plain(ids[1], 100.0 + slot as f64 * 100.0);
            params.set_plain(ids[2], 0.5 + slot as f64 * 0.1);
            params.set_plain(ids[3], slot as f64 * 3.0);
            params.set_plain(ids[4], 1.0);
            params.set_plain(ids[5], (MAX_EFFECTS - 1 - slot) as f64);
        }
        let restored = SynthParams::default();
        let (ids, values) = params.collect_values();
        restored.restore_values(&ids.into_iter().zip(values).collect::<Vec<_>>());
        assert_eq!(
            restored.effect_chain().slots(),
            &(0..MAX_EFFECTS).rev().collect::<Vec<_>>()
        );
        for empty in [false, true] {
            if empty {
                for slot in 0..MAX_EFFECTS {
                    params.set_plain(effect_ids(slot)[4], 0.0);
                }
            }
            let patch = Patch::capture(&params);
            let json = patch.to_json("Chain", &params).unwrap();
            let patch = Patch::from_json(&json).unwrap();
            for (id, value) in patch.normalized_values(&restored) {
                restored.set_normalized(id, value);
            }
            assert_eq!(restored.effect_chain(), params.effect_chain());
            for slot in 0..MAX_EFFECTS {
                for id in effect_ids(slot) {
                    let expected = params.get_plain(id).unwrap();
                    assert!((restored.get_plain(id).unwrap() - expected).abs() < 0.001);
                }
            }
        }
        for (id, value) in Patch::empty().normalized_values(&restored) {
            restored.set_normalized(id, value);
        }
        assert_eq!(restored.effect_chain().slots(), &[0]);
    }

    #[test]
    fn effects_chain_plugin_processes_the_last_of_32_filters() {
        use std::time::Duration;
        use truce_test::driver;

        let render = |enabled: bool, last_mix: f64| {
            let mut driver = driver!(crate::Plugin)
                .duration(Duration::from_millis(100))
                .set_param(SynthParamsParamId::EnvCount, 0.0);
            for slot in 0..MAX_EFFECTS {
                let ids = effect_ids(slot);
                driver = driver
                    .set_param(ids[4], f64::from(enabled))
                    .set_param(ids[3], if slot == 31 { last_mix } else { 0.0 });
            }
            let cutoff = params_cutoff_normalized(200.0);
            driver
                .set_param(effect_ids(31)[1], cutoff)
                .script(|script| script.note_on(69, 1.0))
                .run()
                .output[0]
                .clone()
        };
        let dry = render(false, 0.0);
        assert_eq!(render(true, 0.0), dry);
        let filtered = render(true, 1.0);
        let energy = |samples: &[f32]| {
            samples[1_000..]
                .iter()
                .map(|sample| sample * sample)
                .sum::<f32>()
        };
        assert!(energy(&filtered) < energy(&dry) * 0.1);
        assert!(filtered.iter().all(|sample| sample.is_finite()));
    }

    fn params_cutoff_normalized(hz: f64) -> f64 {
        SynthParams::default()
            .filter_cutoff
            .info
            .range
            .normalize(hz)
    }
}
