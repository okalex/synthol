use std::sync::Arc;
use std::time::Duration;

use truce::prelude::*;

use crate::editor;
use crate::engine::node::envelope::AdsrSettings;
use crate::engine::{FilterMode, FilterSettings, MidiEvent, SynthEngine, Waveform};

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
        _context: &mut ProcessContext,
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

            // Cutoff and Q are smoothed per sample to avoid zipper noise.
            state.engine.set_filter_settings(FilterSettings {
                mode: filter_mode,
                cutoff_hz: params.filter_cutoff.read(),
                q: params.filter_q.read(),
            });
            let sample = state.engine.next_sample(db_to_linear(params.volume.read()));
            for channel in 0..output_channels {
                buffer.output(channel)[sample_index] = sample;
            }
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
