use std::sync::Arc;

use truce::prelude::*;

use crate::editor;
use crate::engine::{MidiEvent, SynthEngine};

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
