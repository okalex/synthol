use std::sync::Arc;

use truce::prelude::*;
use truce_slint::{PluginContext, SlintEditor, SyncFn};

use crate::plugin::{SynthParams, SynthParamsParamId};

slint::include_modules!();

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    SlintEditor::new(
        params.clone(),
        (420, 180),
        |state: PluginContext<SynthParams>| -> SyncFn<SynthParams> {
            let ui = SynthUi::new().expect("failed to create Slint editor");

            let state_for_ui = state.clone();
            ui.on_gain_pressed(move || state_for_ui.begin_edit(SynthParamsParamId::Volume));
            let state_for_ui = state.clone();
            ui.on_gain_changed(move |value| {
                state_for_ui.set_param(SynthParamsParamId::Volume, f64::from(value));
            });
            let state_for_ui = state.clone();
            ui.on_gain_released(move || state_for_ui.end_edit(SynthParamsParamId::Volume));

            Box::new(move |state: &PluginContext<SynthParams>| {
                ui.set_gain(state.get_param(SynthParamsParamId::Volume) as f32);
                ui.set_gain_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Volume),
                ));
            })
        },
    )
    .into_editor()
}
