use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use truce::prelude::*;
use truce_slint::{PluginContext, SlintEditor, SyncFn};

use crate::engine::node::oscillator::render_cycle;
use crate::engine::{MAX_VOICES, Waveform};
use crate::plugin::{OscillatorType, SynthParams, SynthParamsParamId};

slint::include_modules!();

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    SlintEditor::new(
        params.clone(),
        (720, 600),
        |state: PluginContext<SynthParams>| -> SyncFn<SynthParams> {
            let ui = SynthUi::new().expect("failed to create Slint editor");
            let pending_edits = Rc::new(RefCell::new(Vec::<(SynthParamsParamId, f64)>::new()));

            let state_for_ui = state.clone();
            ui.on_gain_changed(move |value| {
                state_for_ui
                    .params()
                    .set_normalized(SynthParamsParamId::Volume.into(), f64::from(value));
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_gain_released(move || {
                let id = SynthParamsParamId::Volume;
                enqueue_edit(
                    &pending_edits_for_ui,
                    (id, f64::from(state_for_ui.get_param(id))),
                );
            });

            let state_for_ui = state.clone();
            ui.on_envelope_changed(move |id, value| {
                if let Some(parameter) = envelope_parameter(id) {
                    state_for_ui
                        .params()
                        .set_normalized(parameter.into(), f64::from(value));
                }
            });
            let state_for_ui = state.clone();
            ui.on_envelope_sustain_level_changed(move |level| {
                let sustain_db = 20.0 * level.clamp(0.001, 1.0).log10();
                let normalized = ((sustain_db + 60.0) / 60.0).clamp(0.0, 1.0);
                state_for_ui
                    .params()
                    .set_normalized(SynthParamsParamId::Sustain.into(), f64::from(normalized));
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_envelope_released(move |id| {
                if let Some(parameter) = envelope_parameter(id) {
                    enqueue_edit(
                        &pending_edits_for_ui,
                        (parameter, f64::from(state_for_ui.get_param(parameter))),
                    );
                }
            });

            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_voices_selected(move |count| {
                let id = SynthParamsParamId::Voices;
                let normalized = voices_to_normalized(count);
                state_for_ui.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits_for_ui, (id, normalized));
            });

            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_oscillator_selected(move |index| {
                let id = SynthParamsParamId::Oscillator;
                let normalized = oscillator_to_normalized(index);
                state_for_ui.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits_for_ui, (id, normalized));
            });

            ui.on_oscillator_cycle_path(|index, width, height| {
                let waveform = oscillator_from_index(index);
                slint::SharedString::from(waveform_cycle_path(waveform, width, height))
            });

            Box::new(move |state: &PluginContext<SynthParams>| {
                for (id, value) in pending_edits.borrow_mut().drain(..) {
                    state.automate(id, value);
                }

                ui.set_gain(state.get_param(SynthParamsParamId::Volume));
                ui.set_gain_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Volume),
                ));
                ui.set_attack(state.get_param(SynthParamsParamId::Attack));
                ui.set_attack_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Attack),
                ));
                ui.set_decay(state.get_param(SynthParamsParamId::Decay));
                ui.set_decay_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Decay),
                ));
                let sustain = state.get_param(SynthParamsParamId::Sustain);
                ui.set_sustain(sustain);
                ui.set_sustain_level(10.0_f32.powf((-60.0 + sustain * 60.0) / 20.0));
                ui.set_sustain_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Sustain),
                ));
                ui.set_release(state.get_param(SynthParamsParamId::Release));
                ui.set_release_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Release),
                ));
                ui.set_voices(state.params().voices.value_i32());
                ui.set_oscillator(state.params().oscillator.index() as i32);
            })
        },
    )
    .into_editor()
}

fn enqueue_edit(
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    edit: (SynthParamsParamId, f64),
) {
    let mut edits = edits.borrow_mut();
    if let Some((_, previous_value)) = edits.iter_mut().find(|(id, _)| *id == edit.0) {
        *previous_value = edit.1;
    } else {
        edits.push(edit);
    }
}

fn voices_to_normalized(count: i32) -> f64 {
    let max = MAX_VOICES as f64;
    ((f64::from(count) - 1.0) / (max - 1.0)).clamp(0.0, 1.0)
}

fn oscillator_to_normalized(index: i32) -> f64 {
    let last = (OscillatorType::variant_count() - 1) as f64;
    (f64::from(index) / last).clamp(0.0, 1.0)
}

fn oscillator_from_index(index: i32) -> Waveform {
    let last = OscillatorType::variant_count() - 1;
    OscillatorType::from_index(usize::try_from(index).unwrap_or(0).min(last)).into()
}

/// Segments per drawn cycle; enough that band-limited edges look vertical.
const CYCLE_PATH_SEGMENTS: usize = 256;

/// SVG path commands tracing one cycle of `waveform` across a `width` by
/// `height` box: 0 degrees on the left, 360 on the right, +1 at the top.
fn waveform_cycle_path(waveform: Waveform, width: f32, height: f32) -> String {
    let mut samples = [0.0; CYCLE_PATH_SEGMENTS + 1];
    render_cycle(waveform, &mut samples);

    let mut commands = String::with_capacity(samples.len() * 16);
    for (index, sample) in samples.iter().enumerate() {
        let x = width * index as f32 / CYCLE_PATH_SEGMENTS as f32;
        let y = height * (1.0 - sample.clamp(-1.0, 1.0)) / 2.0;
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(commands, "{command}{x:.2} {y:.2} ");
    }
    commands
}

fn envelope_parameter(id: i32) -> Option<SynthParamsParamId> {
    match id {
        0 => Some(SynthParamsParamId::Attack),
        1 => Some(SynthParamsParamId::Decay),
        2 => Some(SynthParamsParamId::Sustain),
        3 => Some(SynthParamsParamId::Release),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(commands: &str) -> Vec<(f32, f32)> {
        commands
            .split_whitespace()
            .collect::<Vec<_>>()
            .chunks(2)
            .map(|pair| {
                let x = pair[0][1..].parse().unwrap();
                let y = pair[1].parse().unwrap();
                (x, y)
            })
            .collect()
    }

    #[test]
    fn cycle_path_spans_the_box_from_0_to_360_degrees() {
        let commands = waveform_cycle_path(Waveform::Sine, 200.0, 50.0);
        assert!(commands.starts_with('M'));
        let points = points(&commands);
        assert_eq!(points.len(), CYCLE_PATH_SEGMENTS + 1);
        assert_eq!(points[0], (0.0, 25.0));
        let quarter = points[CYCLE_PATH_SEGMENTS / 4];
        assert!((quarter.0 - 50.0).abs() < 0.01 && quarter.1.abs() < 0.01);
        let three_quarters = points[CYCLE_PATH_SEGMENTS * 3 / 4];
        assert!((three_quarters.1 - 50.0).abs() < 0.01);
        let last = points[CYCLE_PATH_SEGMENTS];
        assert!((last.0 - 200.0).abs() < 0.01 && (last.1 - 25.0).abs() < 0.01);
    }

    #[test]
    fn cycle_path_follows_the_selected_oscillator() {
        let paths: Vec<_> = (0..OscillatorType::variant_count() as i32)
            .map(|index| waveform_cycle_path(oscillator_from_index(index), 100.0, 40.0))
            .collect();
        for (index, path) in paths.iter().enumerate() {
            assert!(!paths[index + 1..].contains(path));
        }
        assert_eq!(oscillator_from_index(-1), Waveform::Sine);
        assert_eq!(oscillator_from_index(99), Waveform::Sawtooth);
    }
}
