use std::sync::Arc;
use std::time::Duration;

use truce::prelude::*;

use crate::editor;
use crate::engine::node::envelope::AdsrSettings;
use crate::engine::{
    FilterMode, FilterSettings, LfoMode, LfoPositions, LfoSettings, MAX_VOICES, MOD_SLOTS,
    MidiEvent, ModDestination, ModRoute, SynthEngine, Waveform,
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

/// Where an LFO routing slot sends the LFO.
#[derive(ParamEnum)]
pub enum ModDestinationType {
    None,
    #[name = "Osc Pitch"]
    OscPitch,
    #[name = "Osc Level"]
    OscLevel,
    #[name = "Filter Cutoff"]
    FilterCutoff,
    #[name = "Filter Q"]
    FilterQ,
}

impl From<ModDestinationType> for Option<ModDestination> {
    fn from(destination: ModDestinationType) -> Self {
        match destination {
            ModDestinationType::None => None,
            ModDestinationType::OscPitch => Some(ModDestination::OscPitch),
            ModDestinationType::OscLevel => Some(ModDestination::OscLevel),
            ModDestinationType::FilterCutoff => Some(ModDestination::FilterCutoff),
            ModDestinationType::FilterQ => Some(ModDestination::FilterQ),
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
    #[param(name = "Oscillator", default = 0)]
    pub oscillator: EnumParam<OscillatorType>,
    #[param(name = "Phase", range = "linear(0, 360)", default = 0.0, unit = "°")]
    pub phase: FloatParam,
    // The pitch and level ranges must match `engine::modulation`.
    #[param(
        name = "Osc Pitch",
        range = "linear(-24, 24)",
        default = 0.0,
        unit = "st",
        smooth = "linear(10)",
        format = "format_pitch"
    )]
    pub osc_pitch: FloatParam,
    #[param(
        name = "Osc Level",
        range = "linear(0, 100)",
        default = 100.0,
        unit = "%",
        smooth = "exp(5)",
        format = "format_percent"
    )]
    pub osc_level: FloatParam,
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
}

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

/// Destination dropdown index for each engine destination; index 0 is None.
pub fn mod_destination_index(destination: Option<ModDestination>) -> u32 {
    destination.map_or(0, |destination| destination.index() as u32 + 1)
}

pub fn mod_destination_from_index(index: u32) -> Option<ModDestination> {
    index
        .checked_sub(1)
        .and_then(|index| ModDestination::ALL.get(index as usize).copied())
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

fn publish_lfo_positions(context: &ProcessContext, positions: &LfoPositions) {
    for (meter, phase) in LFO_POSITION_METERS.iter().zip(positions.phases) {
        context.set_meter(*meter, encode_lfo_position(phase));
    }
    context.set_meter(
        SynthParamsParamId::LfoNewest,
        encode_lfo_newest(positions.newest),
    );
}

impl SynthParams {
    fn format_pitch(&self, value: f64) -> String {
        format!("{value:+.2} st")
    }

    // Values are already percentages; the default "%" format assumes 0..1.
    fn format_percent(&self, value: f64) -> String {
        format!("{value:.0} %")
    }

    fn mod_routes(&self) -> [ModRoute; MOD_SLOTS] {
        let slots = [
            (self.mod_1_destination.value(), &self.mod_1_amount),
            (self.mod_2_destination.value(), &self.mod_2_amount),
            (self.mod_3_destination.value(), &self.mod_3_amount),
            (self.mod_4_destination.value(), &self.mod_4_amount),
        ];
        // Amounts are smoothed, so read every one each sample.
        slots.map(|(destination, amount)| ModRoute {
            destination: destination.into(),
            amount: amount.read() / 100.0,
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
        state.engine.set_output_envelope_settings(AdsrSettings {
            attack: Duration::from_secs_f64(f64::from(params.attack.read()) / 1000.0),
            decay: Duration::from_secs_f64(f64::from(params.decay.read()) / 1000.0),
            sustain_db: params.sustain.read() as f32,
            release: Duration::from_secs_f64(f64::from(params.release.read()) / 1000.0),
        });

        state.engine.set_voice_limit(params.voices.value_usize());
        state.engine.set_waveform(params.oscillator.value().into());
        state.engine.set_start_phase(params.phase.read() / 360.0);

        state.engine.set_lfo_settings(LfoSettings {
            waveform: params.lfo_shape.value().into(),
            frequency_hz: params.lfo_rate.read(),
            mode: params.lfo_mode.value().into(),
        });

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
            });
            state.engine.set_oscillator_pitch(params.osc_pitch.read());
            state
                .engine
                .set_oscillator_level(params.osc_level.read() / 100.0);
            state.engine.set_modulation(&params.mod_routes());
            let sample = state.engine.next_sample(db_to_linear(params.volume.read()));
            for channel in 0..output_channels {
                buffer.output(channel)[sample_index] = sample;
            }
        }

        publish_lfo_positions(context, &state.engine.lfo_positions());

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
