use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use truce::prelude::*;
use truce_slint::{PluginContext, SlintEditor, SyncFn};

use crate::engine::MAX_VOICES;
use crate::plugin::{SynthParams, SynthParamsParamId};

slint::include_modules!();

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    SlintEditor::new(
        params.clone(),
        (720, 540),
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

fn envelope_parameter(id: i32) -> Option<SynthParamsParamId> {
    match id {
        0 => Some(SynthParamsParamId::Attack),
        1 => Some(SynthParamsParamId::Decay),
        2 => Some(SynthParamsParamId::Sustain),
        3 => Some(SynthParamsParamId::Release),
        _ => None,
    }
}
