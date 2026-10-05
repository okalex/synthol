use std::sync::Arc;
use std::time::Duration;

use truce::prelude::*;

use crate::editor;
use crate::engine::node::envelope::AdsrSettings;
use crate::engine::{
    FilterMode, FilterSettings, LfoMode, LfoPositions, LfoSettings, MAX_ENVELOPES, MAX_LFOS,
    MAX_OSCILLATORS, MAX_VOICES, MOD_SLOTS, MidiEvent, ModDestination, ModRoute, SynthEngine,
    Waveform,
};

#[derive(ParamEnum)]
pub enum OscillatorType {
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

impl From<OscillatorType> for Waveform {
    fn from(oscillator: OscillatorType) -> Self {
        match oscillator {
            OscillatorType::Sine => Self::Sine,
            OscillatorType::Square => Self::Square,
            OscillatorType::Triangle => Self::Triangle,
            OscillatorType::Sawtooth => Self::Sawtooth,
        }
    }
}

#[derive(ParamEnum)]
pub enum FilterType {
    #[name = "Low Pass"]
    LowPass,
    #[name = "High Pass"]
    HighPass,
    #[name = "Band Pass"]
    BandPass,
}

impl From<FilterType> for FilterMode {
    fn from(filter: FilterType) -> Self {
        match filter {
            FilterType::LowPass => Self::LowPass,
            FilterType::HighPass => Self::HighPass,
            FilterType::BandPass => Self::BandPass,
        }
    }
}

#[derive(ParamEnum)]
pub enum LfoShapeType {
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

impl From<LfoShapeType> for Waveform {
    fn from(shape: LfoShapeType) -> Self {
        match shape {
            LfoShapeType::Sine => Self::Sine,
            LfoShapeType::Square => Self::Square,
            LfoShapeType::Triangle => Self::Triangle,
            LfoShapeType::Sawtooth => Self::Sawtooth,
        }
    }
}

#[derive(ParamEnum)]
pub enum LfoModeType {
    Trigger,
    Sync,
}

impl From<LfoModeType> for LfoMode {
    fn from(mode: LfoModeType) -> Self {
        match mode {
            LfoModeType::Trigger => Self::Trigger,
            LfoModeType::Sync => Self::Sync,
        }
    }
}

/// Where a modulator routing slot sends its source. Oscillators after the first
/// come last so sessions saved before they existed keep their routes.
#[derive(ParamEnum)]
pub enum ModDestinationType {
    None,
    #[name = "Osc 1 Pitch"]
    Osc1Pitch,
    #[name = "Osc 1 Level"]
    Osc1Level,
    #[name = "Filter Cutoff"]
    FilterCutoff,
    #[name = "Filter Q"]
    FilterQ,
    #[name = "Osc 2 Pitch"]
    Osc2Pitch,
    #[name = "Osc 2 Level"]
    Osc2Level,
    #[name = "Osc 3 Pitch"]
    Osc3Pitch,
    #[name = "Osc 3 Level"]
    Osc3Level,
    #[name = "Osc 4 Pitch"]
    Osc4Pitch,
    #[name = "Osc 4 Level"]
    Osc4Level,
    #[name = "Filter Mix"]
    FilterMix,
}

impl From<ModDestinationType> for Option<ModDestination> {
    fn from(destination: ModDestinationType) -> Self {
        Some(match destination {
            ModDestinationType::None => return None,
            ModDestinationType::Osc1Pitch => ModDestination::OscPitch(0),
            ModDestinationType::Osc1Level => ModDestination::OscLevel(0),
            ModDestinationType::FilterCutoff => ModDestination::FilterCutoff,
            ModDestinationType::FilterQ => ModDestination::FilterQ,
            ModDestinationType::Osc2Pitch => ModDestination::OscPitch(1),
            ModDestinationType::Osc2Level => ModDestination::OscLevel(1),
            ModDestinationType::Osc3Pitch => ModDestination::OscPitch(2),
            ModDestinationType::Osc3Level => ModDestination::OscLevel(2),
            ModDestinationType::Osc4Pitch => ModDestination::OscPitch(3),
            ModDestinationType::Osc4Level => ModDestination::OscLevel(3),
            ModDestinationType::FilterMix => ModDestination::FilterMix,
        })
    }
}

impl From<Option<ModDestination>> for ModDestinationType {
    fn from(destination: Option<ModDestination>) -> Self {
        match destination {
            None => Self::None,
            Some(ModDestination::OscPitch(0)) => Self::Osc1Pitch,
            Some(ModDestination::OscLevel(0)) => Self::Osc1Level,
            Some(ModDestination::FilterCutoff) => Self::FilterCutoff,
            Some(ModDestination::FilterQ) => Self::FilterQ,
            Some(ModDestination::FilterMix) => Self::FilterMix,
            Some(ModDestination::OscPitch(1)) => Self::Osc2Pitch,
            Some(ModDestination::OscLevel(1)) => Self::Osc2Level,
            Some(ModDestination::OscPitch(2)) => Self::Osc3Pitch,
            Some(ModDestination::OscLevel(2)) => Self::Osc3Level,
            Some(ModDestination::OscPitch(3)) => Self::Osc4Pitch,
            Some(ModDestination::OscLevel(3)) => Self::Osc4Level,
            Some(ModDestination::OscPitch(_) | ModDestination::OscLevel(_)) => Self::None,
        }
    }
}

#[derive(Params)]
pub struct SynthParams {
    #[param(
        name = "Volume",
        range = "linear(-60, 0)",
        default = 0.0,
        unit = "dB",
        smooth = "exp(5)"
    )]
    pub volume: FloatParam,
    #[param(
        name = "Attack",
        range = "skewed(0, 10000, 0.2)",
        default = 10.0,
        unit = "ms"
    )]
    pub attack: FloatParam,
    #[param(
        name = "Decay",
        range = "skewed(0, 10000, 0.2)",
        default = 500.0,
        unit = "ms"
    )]
    pub decay: FloatParam,
    #[param(name = "Sustain", range = "linear(-60, 0)", default = -6.0, unit = "dB")]
    pub sustain: FloatParam,
    #[param(
        name = "Release",
        range = "skewed(0, 10000, 0.2)",
        default = 1000.0,
        unit = "ms"
    )]
    pub release: FloatParam,
    #[param(name = "Voices", range = "discrete(1, 8)", default = 8)]
    pub voices: IntParam,
    // Keep the original oscillator and LFO field names: Truce derives
    // stable parameter IDs from them. See `OSCILLATOR_PARAMS` and `LFO_PARAMS`.
    #[param(name = "Osc 1 Type", default = 0)]
    pub osc_1_type: EnumParam<OscillatorType>,
    #[param(
        name = "Osc 1 Phase",
        range = "linear(0, 360)",
        default = 0.0,
        unit = "°"
    )]
    pub osc_1_phase: FloatParam,
    // The pitch and level ranges must match `engine::modulation`.
    #[param(
        name = "Osc 1 Pitch",
        range = "linear(-24, 24)",
        default = 0.0,
        unit = "st",
        smooth = "linear(10)",
        format = "format_pitch"
    )]
    pub osc_1_pitch: FloatParam,
    #[param(
        name = "Osc 1 Level",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "exp(5)",
        format = "format_percent"
    )]
    pub osc_1_level: FloatParam,
    #[param(name = "Filter Type", default = 0)]
    pub filter_type: EnumParam<FilterType>,
    #[param(
        name = "Filter Cutoff",
        range = "log(20, 20000)",
        default = 20000.0,
        unit = "Hz",
        smooth = "log(20)"
    )]
    pub filter_cutoff: FloatParam,
    #[param(
        name = "Filter Q",
        range = "log(0.1, 20)",
        default = 0.707,
        smooth = "exp(20)"
    )]
    pub filter_q: FloatParam,
    #[param(name = "LFO Shape", default = 0)]
    pub lfo_shape: EnumParam<LfoShapeType>,
    #[param(
        name = "LFO Rate",
        range = "log(0.01, 30)",
        default = 1.0,
        unit = "Hz",
        format = "format_lfo_rate"
    )]
    pub lfo_rate: FloatParam,
    #[param(name = "LFO Mode", default = 0)]
    pub lfo_mode: EnumParam<LfoModeType>,
    // LFO routing slots; see `MOD_DESTINATION_PARAMS`. Each amount is a
    // percentage of its destination's control range.
    #[param(name = "LFO Route 1 Destination", default = 0)]
    pub mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub mod_1_amount: FloatParam,
    #[param(name = "LFO Route 2 Destination", default = 0)]
    pub mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub mod_2_amount: FloatParam,
    #[param(name = "LFO Route 3 Destination", default = 0)]
    pub mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub mod_3_amount: FloatParam,
    #[param(name = "LFO Route 4 Destination", default = 0)]
    pub mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub mod_4_amount: FloatParam,
    /// How many oscillators, from the first, sound. The editor's add and
    /// remove buttons change it.
    #[param(name = "Oscillators", range = "discrete(1, 4)", default = 1)]
    pub osc_count: IntParam,
    #[param(name = "Osc 2 Type", default = 0)]
    pub osc_2_type: EnumParam<OscillatorType>,
    #[param(
        name = "Osc 2 Phase",
        range = "linear(0, 360)",
        default = 0.0,
        unit = "°"
    )]
    pub osc_2_phase: FloatParam,
    #[param(
        name = "Osc 2 Pitch",
        range = "linear(-24, 24)",
        default = 0.0,
        unit = "st",
        smooth = "linear(10)",
        format = "format_pitch"
    )]
    pub osc_2_pitch: FloatParam,
    #[param(
        name = "Osc 2 Level",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "exp(5)",
        format = "format_percent"
    )]
    pub osc_2_level: FloatParam,
    #[param(name = "Osc 3 Type", default = 0)]
    pub osc_3_type: EnumParam<OscillatorType>,
    #[param(
        name = "Osc 3 Phase",
        range = "linear(0, 360)",
        default = 0.0,
        unit = "°"
    )]
    pub osc_3_phase: FloatParam,
    #[param(
        name = "Osc 3 Pitch",
        range = "linear(-24, 24)",
        default = 0.0,
        unit = "st",
        smooth = "linear(10)",
        format = "format_pitch"
    )]
    pub osc_3_pitch: FloatParam,
    #[param(
        name = "Osc 3 Level",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "exp(5)",
        format = "format_percent"
    )]
    pub osc_3_level: FloatParam,
    #[param(name = "Osc 4 Type", default = 0)]
    pub osc_4_type: EnumParam<OscillatorType>,
    #[param(
        name = "Osc 4 Phase",
        range = "linear(0, 360)",
        default = 0.0,
        unit = "°"
    )]
    pub osc_4_phase: FloatParam,
    #[param(
        name = "Osc 4 Pitch",
        range = "linear(-24, 24)",
        default = 0.0,
        unit = "st",
        smooth = "linear(10)",
        format = "format_pitch"
    )]
    pub osc_4_pitch: FloatParam,
    #[param(
        name = "Osc 4 Level",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "exp(5)",
        format = "format_percent"
    )]
    pub osc_4_level: FloatParam,
    // LFO positions published for the editor, one slot per voice; see
    // `encode_lfo_position`. Meters can't be arrays, hence the repetition.
    #[meter]
    pub lfo_position_0: MeterSlot,
    #[meter]
    pub lfo_position_1: MeterSlot,
    #[meter]
    pub lfo_position_2: MeterSlot,
    #[meter]
    pub lfo_position_3: MeterSlot,
    #[meter]
    pub lfo_position_4: MeterSlot,
    #[meter]
    pub lfo_position_5: MeterSlot,
    #[meter]
    pub lfo_position_6: MeterSlot,
    #[meter]
    pub lfo_position_7: MeterSlot,
    /// The slot of the most recently started LFO; see `encode_lfo_newest`.
    #[meter]
    pub lfo_newest: MeterSlot,
    #[param(name = "LFOs", range = "discrete(0, 4)", default = 0)]
    pub lfo_count: IntParam,
    #[param(name = "LFO 2 Shape", default = 0)]
    pub lfo_2_shape: EnumParam<LfoShapeType>,
    #[param(
        name = "LFO 2 Rate",
        range = "log(0.01, 30)",
        default = 1.0,
        unit = "Hz",
        format = "format_lfo_rate"
    )]
    pub lfo_2_rate: FloatParam,
    #[param(name = "LFO 2 Mode", default = 0)]
    pub lfo_2_mode: EnumParam<LfoModeType>,
    #[param(name = "LFO 2 Route 1 Destination", default = 0)]
    pub lfo_2_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 2 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_2_mod_1_amount: FloatParam,
    #[param(name = "LFO 2 Route 2 Destination", default = 0)]
    pub lfo_2_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 2 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_2_mod_2_amount: FloatParam,
    #[param(name = "LFO 2 Route 3 Destination", default = 0)]
    pub lfo_2_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 2 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_2_mod_3_amount: FloatParam,
    #[param(name = "LFO 2 Route 4 Destination", default = 0)]
    pub lfo_2_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 2 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_2_mod_4_amount: FloatParam,
    #[param(name = "LFO 3 Shape", default = 0)]
    pub lfo_3_shape: EnumParam<LfoShapeType>,
    #[param(
        name = "LFO 3 Rate",
        range = "log(0.01, 30)",
        default = 1.0,
        unit = "Hz",
        format = "format_lfo_rate"
    )]
    pub lfo_3_rate: FloatParam,
    #[param(name = "LFO 3 Mode", default = 0)]
    pub lfo_3_mode: EnumParam<LfoModeType>,
    #[param(name = "LFO 3 Route 1 Destination", default = 0)]
    pub lfo_3_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 3 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_3_mod_1_amount: FloatParam,
    #[param(name = "LFO 3 Route 2 Destination", default = 0)]
    pub lfo_3_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 3 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_3_mod_2_amount: FloatParam,
    #[param(name = "LFO 3 Route 3 Destination", default = 0)]
    pub lfo_3_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 3 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_3_mod_3_amount: FloatParam,
    #[param(name = "LFO 3 Route 4 Destination", default = 0)]
    pub lfo_3_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 3 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_3_mod_4_amount: FloatParam,
    #[param(name = "LFO 4 Shape", default = 0)]
    pub lfo_4_shape: EnumParam<LfoShapeType>,
    #[param(
        name = "LFO 4 Rate",
        range = "log(0.01, 30)",
        default = 1.0,
        unit = "Hz",
        format = "format_lfo_rate"
    )]
    pub lfo_4_rate: FloatParam,
    #[param(name = "LFO 4 Mode", default = 0)]
    pub lfo_4_mode: EnumParam<LfoModeType>,
    #[param(name = "LFO 4 Route 1 Destination", default = 0)]
    pub lfo_4_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 4 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_4_mod_1_amount: FloatParam,
    #[param(name = "LFO 4 Route 2 Destination", default = 0)]
    pub lfo_4_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 4 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_4_mod_2_amount: FloatParam,
    #[param(name = "LFO 4 Route 3 Destination", default = 0)]
    pub lfo_4_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 4 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_4_mod_3_amount: FloatParam,
    #[param(name = "LFO 4 Route 4 Destination", default = 0)]
    pub lfo_4_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "LFO 4 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub lfo_4_mod_4_amount: FloatParam,
    #[meter]
    pub lfo_2_position_0: MeterSlot,
    #[meter]
    pub lfo_2_position_1: MeterSlot,
    #[meter]
    pub lfo_2_position_2: MeterSlot,
    #[meter]
    pub lfo_2_position_3: MeterSlot,
    #[meter]
    pub lfo_2_position_4: MeterSlot,
    #[meter]
    pub lfo_2_position_5: MeterSlot,
    #[meter]
    pub lfo_2_position_6: MeterSlot,
    #[meter]
    pub lfo_2_position_7: MeterSlot,
    #[meter]
    pub lfo_2_newest: MeterSlot,
    #[meter]
    pub lfo_3_position_0: MeterSlot,
    #[meter]
    pub lfo_3_position_1: MeterSlot,
    #[meter]
    pub lfo_3_position_2: MeterSlot,
    #[meter]
    pub lfo_3_position_3: MeterSlot,
    #[meter]
    pub lfo_3_position_4: MeterSlot,
    #[meter]
    pub lfo_3_position_5: MeterSlot,
    #[meter]
    pub lfo_3_position_6: MeterSlot,
    #[meter]
    pub lfo_3_position_7: MeterSlot,
    #[meter]
    pub lfo_3_newest: MeterSlot,
    #[meter]
    pub lfo_4_position_0: MeterSlot,
    #[meter]
    pub lfo_4_position_1: MeterSlot,
    #[meter]
    pub lfo_4_position_2: MeterSlot,
    #[meter]
    pub lfo_4_position_3: MeterSlot,
    #[meter]
    pub lfo_4_position_4: MeterSlot,
    #[meter]
    pub lfo_4_position_5: MeterSlot,
    #[meter]
    pub lfo_4_position_6: MeterSlot,
    #[meter]
    pub lfo_4_position_7: MeterSlot,
    #[meter]
    pub lfo_4_newest: MeterSlot,
    #[param(
        name = "Filter Mix",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "linear(20)",
        format = "format_percent"
    )]
    pub filter_mix: FloatParam,
    #[param(name = "Envelopes", range = "discrete(0, 4)", default = 1)]
    pub env_count: IntParam,
    #[param(name = "ENV 1 Route 1 Destination", default = 2)]
    pub env_1_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 1 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 100.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_1_mod_1_amount: FloatParam,
    #[param(name = "ENV 1 Route 2 Destination", default = 0)]
    pub env_1_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 1 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_1_mod_2_amount: FloatParam,
    #[param(name = "ENV 1 Route 3 Destination", default = 0)]
    pub env_1_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 1 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_1_mod_3_amount: FloatParam,
    #[param(name = "ENV 1 Route 4 Destination", default = 0)]
    pub env_1_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 1 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_1_mod_4_amount: FloatParam,
    #[meter]
    pub env_1_level: MeterSlot,
    #[param(
        name = "ENV 2 Attack",
        range = "skewed(0, 10000, 0.2)",
        default = 10.0,
        unit = "ms"
    )]
    pub env_2_attack: FloatParam,
    #[param(
        name = "ENV 2 Decay",
        range = "skewed(0, 10000, 0.2)",
        default = 500.0,
        unit = "ms"
    )]
    pub env_2_decay: FloatParam,
    #[param(name = "ENV 2 Sustain", range = "linear(-60, 0)", default = -6.0, unit = "dB")]
    pub env_2_sustain: FloatParam,
    #[param(
        name = "ENV 2 Release",
        range = "skewed(0, 10000, 0.2)",
        default = 1000.0,
        unit = "ms"
    )]
    pub env_2_release: FloatParam,
    #[param(name = "ENV 2 Route 1 Destination", default = 0)]
    pub env_2_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 2 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_2_mod_1_amount: FloatParam,
    #[param(name = "ENV 2 Route 2 Destination", default = 0)]
    pub env_2_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 2 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_2_mod_2_amount: FloatParam,
    #[param(name = "ENV 2 Route 3 Destination", default = 0)]
    pub env_2_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 2 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_2_mod_3_amount: FloatParam,
    #[param(name = "ENV 2 Route 4 Destination", default = 0)]
    pub env_2_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 2 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_2_mod_4_amount: FloatParam,
    #[meter]
    pub env_2_level: MeterSlot,
    #[param(
        name = "ENV 3 Attack",
        range = "skewed(0, 10000, 0.2)",
        default = 10.0,
        unit = "ms"
    )]
    pub env_3_attack: FloatParam,
    #[param(
        name = "ENV 3 Decay",
        range = "skewed(0, 10000, 0.2)",
        default = 500.0,
        unit = "ms"
    )]
    pub env_3_decay: FloatParam,
    #[param(name = "ENV 3 Sustain", range = "linear(-60, 0)", default = -6.0, unit = "dB")]
    pub env_3_sustain: FloatParam,
    #[param(
        name = "ENV 3 Release",
        range = "skewed(0, 10000, 0.2)",
        default = 1000.0,
        unit = "ms"
    )]
    pub env_3_release: FloatParam,
    #[param(name = "ENV 3 Route 1 Destination", default = 0)]
    pub env_3_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 3 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_3_mod_1_amount: FloatParam,
    #[param(name = "ENV 3 Route 2 Destination", default = 0)]
    pub env_3_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 3 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_3_mod_2_amount: FloatParam,
    #[param(name = "ENV 3 Route 3 Destination", default = 0)]
    pub env_3_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 3 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_3_mod_3_amount: FloatParam,
    #[param(name = "ENV 3 Route 4 Destination", default = 0)]
    pub env_3_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 3 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_3_mod_4_amount: FloatParam,
    #[meter]
    pub env_3_level: MeterSlot,
    #[param(
        name = "ENV 4 Attack",
        range = "skewed(0, 10000, 0.2)",
        default = 10.0,
        unit = "ms"
    )]
    pub env_4_attack: FloatParam,
    #[param(
        name = "ENV 4 Decay",
        range = "skewed(0, 10000, 0.2)",
        default = 500.0,
        unit = "ms"
    )]
    pub env_4_decay: FloatParam,
    #[param(name = "ENV 4 Sustain", range = "linear(-60, 0)", default = -6.0, unit = "dB")]
    pub env_4_sustain: FloatParam,
    #[param(
        name = "ENV 4 Release",
        range = "skewed(0, 10000, 0.2)",
        default = 1000.0,
        unit = "ms"
    )]
    pub env_4_release: FloatParam,
    #[param(name = "ENV 4 Route 1 Destination", default = 0)]
    pub env_4_mod_1_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 4 Route 1 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_4_mod_1_amount: FloatParam,
    #[param(name = "ENV 4 Route 2 Destination", default = 0)]
    pub env_4_mod_2_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 4 Route 2 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_4_mod_2_amount: FloatParam,
    #[param(name = "ENV 4 Route 3 Destination", default = 0)]
    pub env_4_mod_3_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 4 Route 3 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_4_mod_3_amount: FloatParam,
    #[param(name = "ENV 4 Route 4 Destination", default = 0)]
    pub env_4_mod_4_destination: EnumParam<ModDestinationType>,
    #[param(
        name = "ENV 4 Route 4 Amount",
        range = "linear(-100, 100)",
        default = 0.0,
        unit = "%",
        smooth = "linear(10)",
        format = "format_percent"
    )]
    pub env_4_mod_4_amount: FloatParam,
    #[meter]
    pub env_4_level: MeterSlot,
}

#[derive(Clone, Copy)]
pub struct EnvelopeParamIds {
    pub adsr: [SynthParamsParamId; 4],
    pub destinations: [SynthParamsParamId; MOD_SLOTS],
    pub amounts: [SynthParamsParamId; MOD_SLOTS],
    pub level: SynthParamsParamId,
}

impl EnvelopeParamIds {
    pub fn all(self) -> [SynthParamsParamId; 4 + 2 * MOD_SLOTS] {
        std::array::from_fn(|index| match index {
            0..4 => self.adsr[index],
            index if index < 4 + MOD_SLOTS => self.destinations[index - 4],
            index => self.amounts[index - 4 - MOD_SLOTS],
        })
    }
}

pub const ENV_PARAMS: [EnvelopeParamIds; MAX_ENVELOPES] = [
    EnvelopeParamIds {
        adsr: [
            SynthParamsParamId::Attack,
            SynthParamsParamId::Decay,
            SynthParamsParamId::Sustain,
            SynthParamsParamId::Release,
        ],
        destinations: [
            SynthParamsParamId::Env1Mod1Destination,
            SynthParamsParamId::Env1Mod2Destination,
            SynthParamsParamId::Env1Mod3Destination,
            SynthParamsParamId::Env1Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Env1Mod1Amount,
            SynthParamsParamId::Env1Mod2Amount,
            SynthParamsParamId::Env1Mod3Amount,
            SynthParamsParamId::Env1Mod4Amount,
        ],
        level: SynthParamsParamId::Env1Level,
    },
    EnvelopeParamIds {
        adsr: [
            SynthParamsParamId::Env2Attack,
            SynthParamsParamId::Env2Decay,
            SynthParamsParamId::Env2Sustain,
            SynthParamsParamId::Env2Release,
        ],
        destinations: [
            SynthParamsParamId::Env2Mod1Destination,
            SynthParamsParamId::Env2Mod2Destination,
            SynthParamsParamId::Env2Mod3Destination,
            SynthParamsParamId::Env2Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Env2Mod1Amount,
            SynthParamsParamId::Env2Mod2Amount,
            SynthParamsParamId::Env2Mod3Amount,
            SynthParamsParamId::Env2Mod4Amount,
        ],
        level: SynthParamsParamId::Env2Level,
    },
    EnvelopeParamIds {
        adsr: [
            SynthParamsParamId::Env3Attack,
            SynthParamsParamId::Env3Decay,
            SynthParamsParamId::Env3Sustain,
            SynthParamsParamId::Env3Release,
        ],
        destinations: [
            SynthParamsParamId::Env3Mod1Destination,
            SynthParamsParamId::Env3Mod2Destination,
            SynthParamsParamId::Env3Mod3Destination,
            SynthParamsParamId::Env3Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Env3Mod1Amount,
            SynthParamsParamId::Env3Mod2Amount,
            SynthParamsParamId::Env3Mod3Amount,
            SynthParamsParamId::Env3Mod4Amount,
        ],
        level: SynthParamsParamId::Env3Level,
    },
    EnvelopeParamIds {
        adsr: [
            SynthParamsParamId::Env4Attack,
            SynthParamsParamId::Env4Decay,
            SynthParamsParamId::Env4Sustain,
            SynthParamsParamId::Env4Release,
        ],
        destinations: [
            SynthParamsParamId::Env4Mod1Destination,
            SynthParamsParamId::Env4Mod2Destination,
            SynthParamsParamId::Env4Mod3Destination,
            SynthParamsParamId::Env4Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Env4Mod1Amount,
            SynthParamsParamId::Env4Mod2Amount,
            SynthParamsParamId::Env4Mod3Amount,
            SynthParamsParamId::Env4Mod4Amount,
        ],
        level: SynthParamsParamId::Env4Level,
    },
];

#[derive(Clone, Copy)]
pub struct LfoParamIds {
    pub shape: SynthParamsParamId,
    pub rate: SynthParamsParamId,
    pub mode: SynthParamsParamId,
    pub destinations: [SynthParamsParamId; MOD_SLOTS],
    pub amounts: [SynthParamsParamId; MOD_SLOTS],
    pub positions: [SynthParamsParamId; MAX_VOICES],
    pub newest: SynthParamsParamId,
}

impl LfoParamIds {
    pub fn all(&self) -> [SynthParamsParamId; 3 + 2 * MOD_SLOTS] {
        std::array::from_fn(|index| match index {
            0 => self.shape,
            1 => self.rate,
            2 => self.mode,
            index if index < 3 + MOD_SLOTS => self.destinations[index - 3],
            index => self.amounts[index - 3 - MOD_SLOTS],
        })
    }
}

pub const LFO_PARAMS: [LfoParamIds; MAX_LFOS] = [
    LfoParamIds {
        shape: SynthParamsParamId::LfoShape,
        rate: SynthParamsParamId::LfoRate,
        mode: SynthParamsParamId::LfoMode,
        destinations: MOD_DESTINATION_PARAMS,
        amounts: MOD_AMOUNT_PARAMS,
        positions: LFO_POSITION_METERS,
        newest: SynthParamsParamId::LfoNewest,
    },
    LfoParamIds {
        shape: SynthParamsParamId::Lfo2Shape,
        rate: SynthParamsParamId::Lfo2Rate,
        mode: SynthParamsParamId::Lfo2Mode,
        destinations: [
            SynthParamsParamId::Lfo2Mod1Destination,
            SynthParamsParamId::Lfo2Mod2Destination,
            SynthParamsParamId::Lfo2Mod3Destination,
            SynthParamsParamId::Lfo2Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Lfo2Mod1Amount,
            SynthParamsParamId::Lfo2Mod2Amount,
            SynthParamsParamId::Lfo2Mod3Amount,
            SynthParamsParamId::Lfo2Mod4Amount,
        ],
        positions: [
            SynthParamsParamId::Lfo2Position0,
            SynthParamsParamId::Lfo2Position1,
            SynthParamsParamId::Lfo2Position2,
            SynthParamsParamId::Lfo2Position3,
            SynthParamsParamId::Lfo2Position4,
            SynthParamsParamId::Lfo2Position5,
            SynthParamsParamId::Lfo2Position6,
            SynthParamsParamId::Lfo2Position7,
        ],
        newest: SynthParamsParamId::Lfo2Newest,
    },
    LfoParamIds {
        shape: SynthParamsParamId::Lfo3Shape,
        rate: SynthParamsParamId::Lfo3Rate,
        mode: SynthParamsParamId::Lfo3Mode,
        destinations: [
            SynthParamsParamId::Lfo3Mod1Destination,
            SynthParamsParamId::Lfo3Mod2Destination,
            SynthParamsParamId::Lfo3Mod3Destination,
            SynthParamsParamId::Lfo3Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Lfo3Mod1Amount,
            SynthParamsParamId::Lfo3Mod2Amount,
            SynthParamsParamId::Lfo3Mod3Amount,
            SynthParamsParamId::Lfo3Mod4Amount,
        ],
        positions: [
            SynthParamsParamId::Lfo3Position0,
            SynthParamsParamId::Lfo3Position1,
            SynthParamsParamId::Lfo3Position2,
            SynthParamsParamId::Lfo3Position3,
            SynthParamsParamId::Lfo3Position4,
            SynthParamsParamId::Lfo3Position5,
            SynthParamsParamId::Lfo3Position6,
            SynthParamsParamId::Lfo3Position7,
        ],
        newest: SynthParamsParamId::Lfo3Newest,
    },
    LfoParamIds {
        shape: SynthParamsParamId::Lfo4Shape,
        rate: SynthParamsParamId::Lfo4Rate,
        mode: SynthParamsParamId::Lfo4Mode,
        destinations: [
            SynthParamsParamId::Lfo4Mod1Destination,
            SynthParamsParamId::Lfo4Mod2Destination,
            SynthParamsParamId::Lfo4Mod3Destination,
            SynthParamsParamId::Lfo4Mod4Destination,
        ],
        amounts: [
            SynthParamsParamId::Lfo4Mod1Amount,
            SynthParamsParamId::Lfo4Mod2Amount,
            SynthParamsParamId::Lfo4Mod3Amount,
            SynthParamsParamId::Lfo4Mod4Amount,
        ],
        positions: [
            SynthParamsParamId::Lfo4Position0,
            SynthParamsParamId::Lfo4Position1,
            SynthParamsParamId::Lfo4Position2,
            SynthParamsParamId::Lfo4Position3,
            SynthParamsParamId::Lfo4Position4,
            SynthParamsParamId::Lfo4Position5,
            SynthParamsParamId::Lfo4Position6,
            SynthParamsParamId::Lfo4Position7,
        ],
        newest: SynthParamsParamId::Lfo4Newest,
    },
];

/// Destination and amount parameters for each LFO routing slot.
pub const MOD_DESTINATION_PARAMS: [SynthParamsParamId; MOD_SLOTS] = [
    SynthParamsParamId::Mod1Destination,
    SynthParamsParamId::Mod2Destination,
    SynthParamsParamId::Mod3Destination,
    SynthParamsParamId::Mod4Destination,
];
pub const MOD_AMOUNT_PARAMS: [SynthParamsParamId; MOD_SLOTS] = [
    SynthParamsParamId::Mod1Amount,
    SynthParamsParamId::Mod2Amount,
    SynthParamsParamId::Mod3Amount,
    SynthParamsParamId::Mod4Amount,
];

/// One oscillator's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OscillatorParamIds {
    pub waveform: SynthParamsParamId,
    pub phase: SynthParamsParamId,
    pub pitch: SynthParamsParamId,
    pub level: SynthParamsParamId,
}

impl OscillatorParamIds {
    pub const fn all(&self) -> [SynthParamsParamId; 4] {
        [self.waveform, self.phase, self.pitch, self.level]
    }
}

pub const OSCILLATOR_PARAMS: [OscillatorParamIds; MAX_OSCILLATORS] = [
    OscillatorParamIds {
        waveform: SynthParamsParamId::Osc1Type,
        phase: SynthParamsParamId::Osc1Phase,
        pitch: SynthParamsParamId::Osc1Pitch,
        level: SynthParamsParamId::Osc1Level,
    },
    OscillatorParamIds {
        waveform: SynthParamsParamId::Osc2Type,
        phase: SynthParamsParamId::Osc2Phase,
        pitch: SynthParamsParamId::Osc2Pitch,
        level: SynthParamsParamId::Osc2Level,
    },
    OscillatorParamIds {
        waveform: SynthParamsParamId::Osc3Type,
        phase: SynthParamsParamId::Osc3Phase,
        pitch: SynthParamsParamId::Osc3Pitch,
        level: SynthParamsParamId::Osc3Level,
    },
    OscillatorParamIds {
        waveform: SynthParamsParamId::Osc4Type,
        phase: SynthParamsParamId::Osc4Phase,
        pitch: SynthParamsParamId::Osc4Pitch,
        level: SynthParamsParamId::Osc4Level,
    },
];

/// A routing slot's destination parameter index; 0 is None.
pub fn mod_destination_index(destination: Option<ModDestination>) -> u32 {
    ModDestinationType::from(destination).to_index() as u32
}

pub fn mod_destination_from_index(index: u32) -> Option<ModDestination> {
    let index = index as usize;
    if index >= ModDestinationType::variant_count() {
        return None;
    }
    ModDestinationType::from_index(index).into()
}

/// Meter for each voice slot's LFO position, indexed like
/// `LfoPositions::phases`.
pub const LFO_POSITION_METERS: [SynthParamsParamId; MAX_VOICES] = [
    SynthParamsParamId::LfoPosition0,
    SynthParamsParamId::LfoPosition1,
    SynthParamsParamId::LfoPosition2,
    SynthParamsParamId::LfoPosition3,
    SynthParamsParamId::LfoPosition4,
    SynthParamsParamId::LfoPosition5,
    SynthParamsParamId::LfoPosition6,
    SynthParamsParamId::LfoPosition7,
];

// Meters read 0.0 before the first block is processed, so 0.0 must mean
// "nothing running"; present values are offset by one.

pub fn encode_lfo_position(phase: Option<f32>) -> f32 {
    phase.map_or(0.0, |phase| 1.0 + phase.clamp(0.0, 1.0))
}

pub fn decode_lfo_position(meter: f32) -> Option<f32> {
    (meter >= 1.0).then(|| (meter - 1.0).min(1.0))
}

pub fn encode_lfo_newest(slot: Option<usize>) -> f32 {
    slot.map_or(0.0, |slot| slot as f32 + 1.0)
}

pub fn decode_lfo_newest(meter: f32) -> Option<usize> {
    (meter >= 1.0).then(|| meter.round() as usize - 1)
}

fn publish_lfo_positions(context: &ProcessContext, ids: &LfoParamIds, positions: &LfoPositions) {
    for (meter, phase) in ids.positions.iter().zip(positions.phases) {
        context.set_meter(*meter, encode_lfo_position(phase));
    }
    context.set_meter(ids.newest, encode_lfo_newest(positions.newest));
}

impl SynthParams {
    fn format_pitch(&self, value: f64) -> String {
        format!("{value:+.2} st")
    }

    // Values are already percentages; the default "%" format assumes 0..1.
    fn format_percent(&self, value: f64) -> String {
        format!("{value:.0} %")
    }

    fn oscillator_params(
        &self,
    ) -> [(
        &EnumParam<OscillatorType>,
        &FloatParam,
        &FloatParam,
        &FloatParam,
    ); MAX_OSCILLATORS] {
        [
            (
                &self.osc_1_type,
                &self.osc_1_phase,
                &self.osc_1_pitch,
                &self.osc_1_level,
            ),
            (
                &self.osc_2_type,
                &self.osc_2_phase,
                &self.osc_2_pitch,
                &self.osc_2_level,
            ),
            (
                &self.osc_3_type,
                &self.osc_3_phase,
                &self.osc_3_pitch,
                &self.osc_3_level,
            ),
            (
                &self.osc_4_type,
                &self.osc_4_phase,
                &self.osc_4_pitch,
                &self.osc_4_level,
            ),
        ]
    }

    fn lfo_settings(&self) -> [LfoSettings; MAX_LFOS] {
        [
            (&self.lfo_shape, &self.lfo_rate, &self.lfo_mode),
            (&self.lfo_2_shape, &self.lfo_2_rate, &self.lfo_2_mode),
            (&self.lfo_3_shape, &self.lfo_3_rate, &self.lfo_3_mode),
            (&self.lfo_4_shape, &self.lfo_4_rate, &self.lfo_4_mode),
        ]
        .map(|(shape, rate, mode)| LfoSettings {
            waveform: shape.value().into(),
            frequency_hz: rate.read(),
            mode: mode.value().into(),
        })
    }

    fn envelope_settings(&self) -> [AdsrSettings; MAX_ENVELOPES] {
        ENV_PARAMS.map(|ids| {
            let [attack, decay, sustain, release] = ids.adsr.map(|id| {
                self.get_plain(id.into())
                    .expect("envelope parameter must exist")
            });
            AdsrSettings {
                attack: Duration::from_secs_f64(attack / 1000.0),
                decay: Duration::from_secs_f64(decay / 1000.0),
                sustain_db: sustain as f32,
                release: Duration::from_secs_f64(release / 1000.0),
            }
        })
    }

    fn envelope_routes(&self) -> [[ModRoute; MOD_SLOTS]; MAX_ENVELOPES] {
        let amounts = [
            [
                &self.env_1_mod_1_amount,
                &self.env_1_mod_2_amount,
                &self.env_1_mod_3_amount,
                &self.env_1_mod_4_amount,
            ],
            [
                &self.env_2_mod_1_amount,
                &self.env_2_mod_2_amount,
                &self.env_2_mod_3_amount,
                &self.env_2_mod_4_amount,
            ],
            [
                &self.env_3_mod_1_amount,
                &self.env_3_mod_2_amount,
                &self.env_3_mod_3_amount,
                &self.env_3_mod_4_amount,
            ],
            [
                &self.env_4_mod_1_amount,
                &self.env_4_mod_2_amount,
                &self.env_4_mod_3_amount,
                &self.env_4_mod_4_amount,
            ],
        ];
        std::array::from_fn(|index| {
            std::array::from_fn(|slot| ModRoute {
                destination: mod_destination_from_index(
                    self.get_plain(ENV_PARAMS[index].destinations[slot].into())
                        .expect("envelope route parameter must exist") as u32,
                ),
                amount: amounts[index][slot].read() / 100.0,
            })
        })
    }

    fn mod_routes(&self) -> [[ModRoute; MOD_SLOTS]; MAX_LFOS] {
        let slots = [
            [
                (&self.mod_1_destination, &self.mod_1_amount),
                (&self.mod_2_destination, &self.mod_2_amount),
                (&self.mod_3_destination, &self.mod_3_amount),
                (&self.mod_4_destination, &self.mod_4_amount),
            ],
            [
                (&self.lfo_2_mod_1_destination, &self.lfo_2_mod_1_amount),
                (&self.lfo_2_mod_2_destination, &self.lfo_2_mod_2_amount),
                (&self.lfo_2_mod_3_destination, &self.lfo_2_mod_3_amount),
                (&self.lfo_2_mod_4_destination, &self.lfo_2_mod_4_amount),
            ],
            [
                (&self.lfo_3_mod_1_destination, &self.lfo_3_mod_1_amount),
                (&self.lfo_3_mod_2_destination, &self.lfo_3_mod_2_amount),
                (&self.lfo_3_mod_3_destination, &self.lfo_3_mod_3_amount),
                (&self.lfo_3_mod_4_destination, &self.lfo_3_mod_4_amount),
            ],
            [
                (&self.lfo_4_mod_1_destination, &self.lfo_4_mod_1_amount),
                (&self.lfo_4_mod_2_destination, &self.lfo_4_mod_2_amount),
                (&self.lfo_4_mod_3_destination, &self.lfo_4_mod_3_amount),
                (&self.lfo_4_mod_4_destination, &self.lfo_4_mod_4_amount),
            ],
        ];
        // Amounts are smoothed, so read every one each sample.
        slots.map(|slots| {
            slots.map(|(destination, amount)| ModRoute {
                destination: destination.value().into(),
                amount: amount.read() / 100.0,
            })
        })
    }

    /// The built-in Hz formatting has no decimals, which would show the
    /// slowest rates as "0 Hz".
    fn format_lfo_rate(&self, value: f64) -> String {
        if value < 1.0 {
            format!("{value:.2} Hz")
        } else if value < 10.0 {
            format!("{value:.1} Hz")
        } else {
            format!("{value:.0} Hz")
        }
    }
}

pub struct Synth;

#[derive(Default)]
pub struct SynthState {
    engine: SynthEngine,
}

impl PluginLogic for Synth {
    type Params = SynthParams;
    type DspState = SynthState;

    fn bus_layouts() -> Vec<BusLayout> {
        BusLayout::stereo_and_mono_output()
    }

    fn reset(state: &mut SynthState, _params: &SynthParams, config: &AudioConfig) {
        state.engine.reset(config.sample_rate as f32);
    }

    fn process(
        state: &mut SynthState,
        params: &SynthParams,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        state
            .engine
            .set_envelope_count(params.env_count.value_usize());
        for (index, settings) in params.envelope_settings().into_iter().enumerate() {
            state.engine.set_envelope(index, settings);
        }

        state.engine.set_voice_limit(params.voices.value_usize());
        state
            .engine
            .set_oscillator_count(params.osc_count.value_usize());
        let oscillators = params.oscillator_params();
        for (index, (waveform, phase, _, _)) in oscillators.iter().enumerate() {
            state.engine.set_waveform(index, waveform.value().into());
            state.engine.set_start_phase(index, phase.read() / 360.0);
        }

        state.engine.set_lfo_count(params.lfo_count.value_usize());
        for (index, settings) in params.lfo_settings().into_iter().enumerate() {
            state.engine.set_lfo(index, settings);
        }

        let filter_mode: FilterMode = params.filter_type.value().into();

        let mut next_event = 0;
        let output_channels = buffer.num_output_channels();

        for sample_index in 0..buffer.num_samples() {
            while let Some(event) = events.get(next_event) {
                if event.sample_offset as usize > sample_index {
                    break;
                }

                if let Some(event) = midi_event_from_host(&event.body) {
                    state.engine.handle_event(event);
                }
                next_event += 1;
            }

            // Continuous controls are smoothed per sample to avoid zipper
            // noise.
            state.engine.set_filter_settings(FilterSettings {
                mode: filter_mode,
                cutoff_hz: params.filter_cutoff.read(),
                q: params.filter_q.read(),
                mix: params.filter_mix.read() / 100.0,
            });
            // Inactive oscillators are read too so their smoothers stay
            // current for when they're added.
            for (index, (_, _, pitch, level)) in oscillators.iter().enumerate() {
                state.engine.set_oscillator_pitch(index, pitch.read());
                state
                    .engine
                    .set_oscillator_level(index, level.read() / 100.0);
            }
            for (index, routes) in params.mod_routes().iter().enumerate() {
                state.engine.set_lfo_modulation(index, routes);
            }
            for (index, routes) in params.envelope_routes().iter().enumerate() {
                state.engine.set_envelope_modulation(index, routes);
            }
            let sample = state.engine.next_sample(db_to_linear(params.volume.read()));
            for channel in 0..output_channels {
                buffer.output(channel)[sample_index] = sample;
            }
        }

        for (index, ids) in LFO_PARAMS.iter().enumerate() {
            publish_lfo_positions(context, ids, &state.engine.lfo_positions_at(index));
        }
        for (index, ids) in ENV_PARAMS.iter().enumerate() {
            let level = state
                .engine
                .envelope_level(index)
                .map_or(0.0, |level| level + 1.0);
            context.set_meter(ids.level, level);
        }

        if state.engine.has_active_note() {
            ProcessStatus::Normal
        } else {
            ProcessStatus::Tail(0)
        }
    }

    fn editor(params: Arc<SynthParams>) -> Box<dyn Editor> {
        editor::create(params)
    }
}

fn midi_event_from_host(event: &EventBody) -> Option<MidiEvent> {
    match event {
        EventBody::NoteOn { note, velocity, .. } => Some(MidiEvent::NoteOn {
            note: *note,
            velocity: *velocity,
        }),
        EventBody::NoteOff { note, .. } => Some(MidiEvent::NoteOff { note: *note }),
        _ => None,
    }
}
