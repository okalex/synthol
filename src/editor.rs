use std::cell::{Cell, RefCell};
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use slint::Model;

use truce::prelude::*;
use truce_slint::{KeyboardCapture, PluginContext, SlintEditor, SyncFn};

mod effects;
mod patches;

use self::patches::PatchController;

use crate::engine::modulation::MAX_PITCH_SEMITONES;
use crate::engine::node::filter::{BiquadCoefficients, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ};
use crate::engine::node::filter::{MAX_Q, MIN_Q};
use crate::engine::node::oscillator::render_cycle;
use crate::engine::{
    FilterMode, FilterSettings, LfoPoint, LfoShape, MAX_ENVELOPES, MAX_LFOS, MAX_OSCILLATORS,
    MAX_VOICES, MOD_SLOTS, ModDestination, ModRoute, Waveform,
};
use crate::patch::PatchLibrary;
use crate::plugin::{
    ENV_PARAMS, FilterType, LFO_PARAMS, LfoModeType, LfoShapeType, ModDestinationType,
    OSCILLATOR_PARAMS, OscillatorType, SynthParams, SynthParamsParamId, decode_lfo_newest,
    decode_lfo_position, mod_destination_from_index, mod_destination_index,
};

slint::include_modules!();

const EDITOR_SIZE: (u32, u32) = (1100, 1100);

const ENVELOPE_PLOT_MAX_MS: f32 = 30_000.0;

// A 1 ms offset makes the logarithmic axis finite at instantaneous stages.
fn envelope_time_position(milliseconds: f32) -> f32 {
    milliseconds.clamp(0.0, ENVELOPE_PLOT_MAX_MS).ln_1p() / ENVELOPE_PLOT_MAX_MS.ln_1p()
}

fn envelope_position_time(position: f32) -> f32 {
    (position.clamp(0.0, 1.0) * ENVELOPE_PLOT_MAX_MS.ln_1p()).exp_m1()
}

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    let keyboard = KeyboardCapture::new();
    let keyboard_for_setup = keyboard.clone();
    SlintEditor::new(
        params,
        EDITOR_SIZE,
        move |state: PluginContext<SynthParams>| -> SyncFn<SynthParams> {
            let ui = SynthUi::new().expect("failed to create Slint editor");
            setup_editor_with(state, ui, PatchLibrary::user(), keyboard_for_setup.clone())
        },
    )
    .keyboard_capture(keyboard)
    .into_editor()
}

#[cfg(test)]
fn setup_editor(state: PluginContext<SynthParams>, ui: SynthUi) -> SyncFn<SynthParams> {
    let library = PatchLibrary::new(std::env::temp_dir().join("synthol-editor-tests-unused"));
    setup_editor_with(state, ui, library, KeyboardCapture::new())
}

fn setup_editor_with(
    state: PluginContext<SynthParams>,
    ui: SynthUi,
    library: PatchLibrary,
    keyboard: KeyboardCapture,
) -> SyncFn<SynthParams> {
    let range = state.params().attack.info.range;
    ui.global::<EnvelopeTimeScale>()
        .on_duration(move |value| range.denormalize(f64::from(value.clamp(0.0, 1.0))) as f32);
    ui.global::<EnvelopeTimeScale>()
        .on_value(move |milliseconds| {
            range.normalize(f64::from(milliseconds.clamp(0.0, 10_000.0))) as f32
        });
    ui.global::<EnvelopeTimeScale>()
        .on_position(envelope_time_position);
    ui.global::<EnvelopeTimeScale>()
        .on_time(envelope_position_time);
    let patch_controller = Rc::new(RefCell::new(PatchController::new(library)));
    patches::wire(&ui, &state, &patch_controller);
    let sync_effects = effects::wire(&ui, &state);
    let pending_edits = Rc::new(RefCell::new(Vec::<(SynthParamsParamId, f64)>::new()));
    let selected_envelope = {
        let ui = ui.as_weak();
        move || {
            ui.upgrade()
                .expect("envelope callback requires a live editor")
                .get_current_envelope() as usize
        }
    };
    let selected_modulator = {
        let ui = ui.as_weak();
        move || {
            let ui = ui
                .upgrade()
                .expect("modulation callback requires a live editor");
            if ui.get_envelope_selected() {
                Modulator::Envelope(ui.get_current_envelope() as usize)
            } else {
                Modulator::Lfo(ui.get_current_lfo() as usize)
            }
        }
    };

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
    let selected = selected_envelope.clone();
    ui.on_envelope_changed(move |id, value| {
        if let Some(parameter) = envelope_parameter(selected(), id) {
            state_for_ui
                .params()
                .set_normalized(parameter.into(), f64::from(value));
        }
    });
    let state_for_ui = state.clone();
    let selected = selected_envelope.clone();
    ui.on_envelope_sustain_level_changed(move |level| {
        let sustain_db = 20.0 * level.clamp(0.001, 1.0).log10();
        let normalized = ((sustain_db + 60.0) / 60.0).clamp(0.0, 1.0);
        state_for_ui
            .params()
            .set_normalized(ENV_PARAMS[selected()].adsr[2].into(), f64::from(normalized));
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_envelope.clone();
    ui.on_envelope_released(move |id| {
        if let Some(parameter) = envelope_parameter(selected(), id) {
            enqueue_edit(
                &pending_edits_for_ui,
                (parameter, f64::from(state_for_ui.get_param(parameter))),
            );
        }
    });

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let ui_for_callback = ui.as_weak();
    ui.on_envelope_add(move || {
        let count = state_for_ui.params().env_count.value_usize();
        if count < MAX_ENVELOPES {
            reset_modulator(
                &state_for_ui,
                &pending_edits_for_ui,
                &ENV_PARAMS[count].all(),
            );
            set_param(
                &state_for_ui,
                &pending_edits_for_ui,
                SynthParamsParamId::EnvCount,
                (count + 1) as f64 / MAX_ENVELOPES as f64,
            );
            let ui = ui_for_callback
                .upgrade()
                .expect("envelope callback requires a live editor");
            ui.set_envelope_selected(true);
            ui.set_selected_envelope(count as i32);
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_envelope_remove(move |index| {
        if let Ok(index) = usize::try_from(index) {
            remove_envelope(&state_for_ui, &pending_edits_for_ui, index);
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
    ui.on_oscillator_selected(move |oscillator, index| {
        let Some(params) = oscillator_params(oscillator) else {
            return;
        };
        let id = params.waveform;
        let normalized = oscillator_to_normalized(index);
        state_for_ui.params().set_normalized(id.into(), normalized);
        enqueue_edit(&pending_edits_for_ui, (id, normalized));
    });

    let state_for_ui = state.clone();
    ui.on_osc_changed(move |oscillator, id, value| {
        if let Some(parameter) = oscillator_parameter(oscillator, id) {
            state_for_ui
                .params()
                .set_normalized(parameter.into(), f64::from(value));
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_osc_released(move |oscillator, id| {
        if let Some(parameter) = oscillator_parameter(oscillator, id) {
            enqueue_edit(
                &pending_edits_for_ui,
                (parameter, f64::from(state_for_ui.get_param(parameter))),
            );
        }
    });

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_oscillator_add(move || {
        let count = state_for_ui.params().osc_count.value_usize();
        if count < MAX_OSCILLATORS {
            set_param(
                &state_for_ui,
                &pending_edits_for_ui,
                SynthParamsParamId::OscCount,
                oscillator_count_to_normalized(count + 1),
            );
        }
    });

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_oscillator_remove(move |oscillator| {
        let Ok(removed) = usize::try_from(oscillator) else {
            return;
        };
        remove_oscillator(&state_for_ui, &pending_edits_for_ui, removed);
    });

    ui.on_oscillator_cycle_path(|index, phase, width, height| {
        let waveform = oscillator_from_index(index);
        slint::SharedString::from(waveform_cycle_path(waveform, phase, width, height))
    });

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_filter_type_selected(move |index| {
        let id = SynthParamsParamId::FilterType;
        let normalized = filter_type_to_normalized(index);
        state_for_ui.params().set_normalized(id.into(), normalized);
        enqueue_edit(&pending_edits_for_ui, (id, normalized));
    });

    let state_for_ui = state.clone();
    ui.on_filter_changed(move |id, value| {
        if let Some(parameter) = filter_parameter(id) {
            state_for_ui
                .params()
                .set_normalized(parameter.into(), f64::from(value));
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_filter_released(move |id| {
        if let Some(parameter) = filter_parameter(id) {
            enqueue_edit(
                &pending_edits_for_ui,
                (parameter, f64::from(state_for_ui.get_param(parameter))),
            );
        }
    });

    // Takes the knobs' normalized values so the plot tracks a drag
    // without waiting for the next sync.
    let state_for_ui = state.clone();
    ui.on_filter_response_path(move |index, cutoff, q, mix, width, height| {
        let params = state_for_ui.params();
        let settings = FilterSettings {
            mode: filter_mode_from_index(index),
            cutoff_hz: params
                .filter_cutoff
                .info
                .range
                .denormalize(f64::from(cutoff)) as f32,
            q: params.filter_q.info.range.denormalize(f64::from(q)) as f32,
            mix,
        };
        slint::SharedString::from(filter_response_path(settings, width, height))
    });

    let selected_lfo = {
        let ui = ui.as_weak();
        move || {
            ui.upgrade()
                .expect("LFO callback requires a live editor")
                .get_current_lfo() as usize
        }
    };

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let ui_for_callback = ui.as_weak();
    ui.on_lfo_add(move || {
        let count = state_for_ui.params().lfo_count.value_usize();
        if count < MAX_LFOS {
            reset_lfo(&state_for_ui, &pending_edits_for_ui, count);
            set_param(
                &state_for_ui,
                &pending_edits_for_ui,
                SynthParamsParamId::LfoCount,
                lfo_count_to_normalized(count + 1),
            );
            ui_for_callback
                .upgrade()
                .expect("LFO callback requires a live editor")
                .set_envelope_selected(false);
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    ui.on_lfo_remove(move |index| {
        if let Ok(index) = usize::try_from(index) {
            remove_lfo(&state_for_ui, &pending_edits_for_ui, index);
        }
    });

    let lfo_view = Rc::new(LfoShapeView::new(&ui));

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    let view = lfo_view.clone();
    let ui_for_callback = ui.as_weak();
    ui.on_lfo_shape_selected(move |index| {
        let lfo = selected();
        let params = state_for_ui.params();
        match LfoPreset::from_index(index) {
            Some(LfoPreset::Parameter(_)) => {
                let id = LFO_PARAMS[lfo].shape;
                let normalized = lfo_shape_to_normalized(index);
                params.set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits_for_ui, (id, normalized));
                params.set_custom_lfo_shape(lfo, None);
            }
            Some(LfoPreset::Nodes(shape)) => params.set_custom_lfo_shape(lfo, Some(*shape)),
            None => return,
        }
        let ui = ui_for_callback
            .upgrade()
            .expect("LFO callback requires a live editor");
        view.show(&ui, params.lfo_shape(lfo));
    });

    // Applies a node edit to the selected LFO's shape, turning it custom.
    let edit_lfo_shape: Rc<LfoShapeEdit> = {
        let state = state.clone();
        let selected = selected_lfo.clone();
        let view = lfo_view.clone();
        let ui = ui.as_weak();
        Rc::new(move |edit: &dyn Fn(&mut LfoShape)| {
            let lfo = selected();
            let params = state.params();
            let mut shape = params.lfo_shape(lfo);
            edit(&mut shape);
            params.set_custom_lfo_shape(lfo, Some(shape));
            view.show(
                &ui.upgrade().expect("LFO callback requires a live editor"),
                shape,
            );
        })
    };
    let edit = edit_lfo_shape.clone();
    ui.on_lfo_node_moved(move |index, x, y, snap| {
        let (x, y) = snap_lfo_point(x, y, snap);
        edit(&|shape| shape.move_point(index as usize, x, y));
    });
    let edit = edit_lfo_shape.clone();
    ui.on_lfo_node_added(move |x, y, snap| {
        let (x, y) = snap_lfo_point(x, y, snap);
        edit(&|shape| {
            shape.insert_point(x, y);
        });
    });
    let edit = edit_lfo_shape.clone();
    ui.on_lfo_node_removed(move |index| {
        edit(&|shape| {
            shape.remove_point(index as usize);
        });
    });
    let curve_drag_start = Rc::new(Cell::new(0.0_f32));
    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    let drag_start = curve_drag_start.clone();
    ui.on_lfo_curve_pressed(move |index| {
        let shape = state_for_ui.params().lfo_shape(selected());
        if let Some(point) = shape.points().get(index as usize) {
            drag_start.set(point.curve);
        }
    });
    let edit = edit_lfo_shape.clone();
    ui.on_lfo_curve_dragged(move |index, amount| {
        let start = curve_drag_start.get();
        edit(&|shape| {
            if let Some(curve) = dragged_lfo_curve(shape, index as usize, start, amount) {
                shape.set_curve(index as usize, curve);
            }
        });
    });
    let edit = edit_lfo_shape.clone();
    ui.on_lfo_curve_reset(move |index| edit(&|shape| shape.set_curve(index as usize, 0.0)));
    let edit = edit_lfo_shape;
    ui.on_lfo_smooth_toggled(move || edit(&|shape| shape.set_smooth(!shape.is_smooth())));
    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    ui.on_lfo_shape_path(move |_revision, width, height, filled| {
        let shape = state_for_ui.params().lfo_shape(selected());
        slint::SharedString::from(lfo_shape_path(&shape, width, height, filled))
    });

    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    ui.on_lfo_rate_changed(move |value| {
        state_for_ui
            .params()
            .set_normalized(LFO_PARAMS[selected()].rate.into(), f64::from(value));
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    ui.on_lfo_rate_released(move || {
        let id = LFO_PARAMS[selected()].rate;
        enqueue_edit(
            &pending_edits_for_ui,
            (id, f64::from(state_for_ui.get_param(id))),
        );
    });

    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_lfo.clone();
    ui.on_lfo_mode_selected(move |index| {
        let id = LFO_PARAMS[selected()].mode;
        let normalized = lfo_mode_to_normalized(index);
        state_for_ui.params().set_normalized(id.into(), normalized);
        enqueue_edit(&pending_edits_for_ui, (id, normalized));
    });

    let lfo_marker_model = Rc::new(slint::VecModel::from(vec![
        PositionMarker::default();
        MAX_VOICES
    ]));
    ui.set_lfo_markers(slint::ModelRc::from(lfo_marker_model.clone()));

    // Writes a routing slot's parameters and records them for the host.
    let set_slot = {
        let pending_edits = pending_edits.clone();
        let state = state.clone();
        let selected = selected_modulator.clone();
        move |slot: usize, destination: Option<Option<ModDestination>>, amount: Option<f32>| {
            if let Some(destination) = destination {
                let filter_id = selected().filters()[slot];
                set_param(
                    &state,
                    &pending_edits,
                    filter_id,
                    destination.and_then(ModDestination::effect).unwrap_or(0) as f64 / 31.0,
                );
                let id = selected().routing().0[slot];
                let normalized = destination_to_normalized(destination);
                state.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits, (id, normalized));
            }
            if let Some(amount) = amount {
                let id = selected().routing().1[slot];
                let normalized = amount_to_normalized(amount);
                state.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits, (id, normalized));
            }
        }
    };

    let state_for_ui = state.clone();
    let set_slot_for_ui = set_slot.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_assign(move |target| {
        let Some(destination) = mod_target(target) else {
            return;
        };
        if let Some(slot) = slot_for_new_route(
            &read_modulator_routes(&state_for_ui, selected()),
            destination,
        ) {
            set_slot_for_ui(
                slot,
                Some(Some(destination)),
                Some(selected().default_amount(destination)),
            );
        }
    });

    let state_for_ui = state.clone();
    let set_slot_for_ui = set_slot.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_destination_selected(move |slot, index| {
        let Some(slot) = mod_slot(slot) else {
            return;
        };
        let routes = read_modulator_routes(&state_for_ui, selected());
        let choices = routing_choices(state_for_ui.params(), &routes);
        let destination = usize::try_from(index)
            .ok()
            .and_then(|index| choices.get(index).copied().flatten());
        let route = routes[slot];
        // A freshly routed slot starts at a useful depth rather than zero.
        let amount = match destination {
            Some(destination) if route.amount == 0.0 => {
                Some(selected().default_amount(destination))
            }
            _ => None,
        };
        set_slot_for_ui(slot, Some(destination), amount);
    });

    let state_for_ui = state.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_amount_changed(move |slot, value| {
        if let Some(slot) = mod_slot(slot) {
            state_for_ui
                .params()
                .set_normalized(selected().routing().1[slot].into(), f64::from(value));
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_amount_released(move |slot| {
        if let Some(slot) = mod_slot(slot) {
            let id = selected().routing().1[slot];
            enqueue_edit(
                &pending_edits_for_ui,
                (id, f64::from(state_for_ui.get_param(id))),
            );
        }
    });

    let set_slot_for_ui = set_slot.clone();
    ui.on_mod_remove(move |slot| {
        if let Some(slot) = mod_slot(slot) {
            set_slot_for_ui(slot, Some(None), Some(0.0));
        }
    });

    let state_for_ui = state.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_depth_changed(move |target, depth| {
        let Some(destination) = mod_target(target) else {
            return;
        };
        if let Some((slot, amount)) = depth_edit(
            &read_modulator_routes(&state_for_ui, selected()),
            destination,
            depth,
        ) {
            state_for_ui.params().set_normalized(
                selected().routing().1[slot].into(),
                amount_to_normalized(amount),
            );
        }
    });
    let pending_edits_for_ui = pending_edits.clone();
    let state_for_ui = state.clone();
    let selected = selected_modulator.clone();
    ui.on_mod_depth_released(move |target| {
        let Some(destination) = mod_target(target) else {
            return;
        };
        let routes = read_modulator_routes(&state_for_ui, selected());
        if let Some(slot) = routes
            .iter()
            .position(|route| route.destination == Some(destination))
        {
            let id = selected().routing().1[slot];
            enqueue_edit(
                &pending_edits_for_ui,
                (id, f64::from(state_for_ui.get_param(id))),
            );
        }
    });

    let mod_slot_model = Rc::new(slint::VecModel::from(vec![
        ModRouteSlot::default();
        MOD_SLOTS
    ]));
    ui.set_mod_slots(slint::ModelRc::from(mod_slot_model.clone()));
    let knob_mod_model = Rc::new(slint::VecModel::from(vec![
        KnobModulation::default();
        ModDestination::ALL.len()
    ]));
    ui.set_knob_mods(slint::ModelRc::from(knob_mod_model.clone()));
    let oscillator_model = Rc::new(slint::VecModel::from(vec![OscillatorRow::default()]));
    ui.set_oscillators(slint::ModelRc::from(oscillator_model.clone()));
    let destination_model = Rc::new(slint::VecModel::from(destination_options(1)));
    ui.set_mod_destinations(slint::ModelRc::from(destination_model.clone()));

    Box::new(move |state: &PluginContext<SynthParams>| {
        for (id, value) in pending_edits.borrow_mut().drain(..) {
            state.automate(id, value);
        }
        patches::sync(&ui, state, &patch_controller, &keyboard);
        sync_effects(state);

        ui.set_gain(state.get_param(SynthParamsParamId::Volume));
        ui.set_gain_text(slint::SharedString::from(
            state.format_param(SynthParamsParamId::Volume),
        ));
        let env_count = state.params().env_count.value_usize();
        let lfo_count = state.params().lfo_count.value_usize();
        ui.set_env_count(env_count as i32);
        ui.set_lfo_count(lfo_count as i32);
        if env_count == 0 && lfo_count > 0 {
            ui.set_envelope_selected(false);
        } else if lfo_count == 0 && env_count > 0 {
            ui.set_envelope_selected(true);
        }
        let [attack, decay, sustain_id, release] =
            ENV_PARAMS[ui.get_current_envelope() as usize].adsr;
        ui.set_attack(state.get_param(attack));
        ui.set_attack_text(slint::SharedString::from(state.format_param(attack)));
        ui.set_decay(state.get_param(decay));
        ui.set_decay_text(slint::SharedString::from(state.format_param(decay)));
        let sustain = state.get_param(sustain_id);
        ui.set_sustain(sustain);
        ui.set_sustain_level(10.0_f32.powf((-60.0 + sustain * 60.0) / 20.0));
        ui.set_sustain_text(slint::SharedString::from(state.format_param(sustain_id)));
        ui.set_release(state.get_param(release));
        ui.set_release_text(slint::SharedString::from(state.format_param(release)));
        ui.set_voices(state.params().voices.value_i32());
        let oscillator_count = state.params().osc_count.value_usize();
        let rows: Vec<_> = OSCILLATOR_PARAMS[..oscillator_count]
            .iter()
            .map(|params| oscillator_row(state, params))
            .collect();
        sync_model(&oscillator_model, rows);
        ui.set_filter_type(state.params().filter_type.index() as i32);
        ui.set_filter_cutoff(state.get_param(SynthParamsParamId::FilterCutoff));
        ui.set_filter_cutoff_text(slint::SharedString::from(
            state.format_param(SynthParamsParamId::FilterCutoff),
        ));
        ui.set_filter_q(state.get_param(SynthParamsParamId::FilterQ));
        ui.set_filter_q_text(slint::SharedString::from(
            state.format_param(SynthParamsParamId::FilterQ),
        ));
        ui.set_filter_mix(state.get_param(SynthParamsParamId::FilterMix));
        ui.set_filter_mix_text(state.format_param(SynthParamsParamId::FilterMix).into());
        let selected = ui.get_current_lfo() as usize;
        let ids = &LFO_PARAMS[selected];
        let preset = (state.get_param(ids.shape) * (LfoShapeType::variant_count() - 1) as f32)
            .round() as i32;
        ui.set_lfo_shape(lfo_dropdown_index(
            preset,
            state.params().custom_lfo_shape(selected),
        ));
        lfo_view.show(&ui, state.params().lfo_shape(selected));
        ui.set_lfo_rate(state.get_param(ids.rate));
        ui.set_lfo_rate_text(slint::SharedString::from(state.format_param(ids.rate)));
        ui.set_lfo_mode(
            (state.get_param(ids.mode) * (LfoModeType::variant_count() - 1) as f32).round() as i32,
        );
        // Runs every frame, so the markers follow the audio thread at
        // block granularity. Rows are only touched when they change.
        let phases = ids
            .positions
            .map(|id| decode_lfo_position(state.get_meter(id)));
        let newest = decode_lfo_newest(state.get_meter(ids.newest));
        for (row, marker) in lfo_markers(phases, newest).into_iter().enumerate() {
            if lfo_marker_model.row_data(row).as_ref() != Some(&marker) {
                lfo_marker_model.set_row_data(row, marker);
            }
        }

        let routes = if lfo_count + env_count > 0 {
            read_modulator_routes(state, selected_modulator())
        } else {
            [ModRoute::default(); MOD_SLOTS]
        };
        let shown = shown_oscillators(oscillator_count, &routes);
        let choices = routing_choices(state.params(), &routes);
        sync_model(
            &destination_model,
            choices
                .iter()
                .map(|&destination| destination_label(destination).into())
                .collect(),
        );
        let slots = mod_route_slots(&routes, shown);
        for (row, mut slot) in slots.into_iter().enumerate() {
            slot.destination = choices
                .iter()
                .position(|destination| *destination == routes[row].destination)
                .unwrap_or(0) as i32;
            if mod_slot_model.row_data(row).as_ref() != Some(&slot) {
                mod_slot_model.set_row_data(row, slot);
            }
        }
        let bases = ModDestination::ALL
            .map(|destination| state.get_param(destination_parameter(destination)));
        let sources = std::array::from_fn(|index| {
            if index >= lfo_count {
                return ([ModRoute::default(); MOD_SLOTS], None);
            }
            let ids = &LFO_PARAMS[index];
            let phases = ids
                .positions
                .map(|id| decode_lfo_position(state.get_meter(id)));
            let newest = decode_lfo_newest(state.get_meter(ids.newest));
            let phase = newest.and_then(|slot| phases.get(slot).copied().flatten());
            let value = phase.map(|phase| state.params().lfo_shape(index).sample(phase));
            (read_routes(state, index), value)
        });
        let mut mods = combined_knob_modulation(&routes, bases, &sources);
        let envelope_sources: [_; MAX_ENVELOPES] = std::array::from_fn(|index| {
            if index >= env_count {
                return ([ModRoute::default(); MOD_SLOTS], None);
            }
            let meter = state.get_meter(ENV_PARAMS[index].level);
            (
                read_modulator_routes(state, Modulator::Envelope(index)),
                (meter >= 1.0).then_some(meter - 1.0),
            )
        });
        apply_envelope_knob_modulation(&mut mods, &envelope_sources, ui.get_envelope_selected());
        for (row, modulation) in mods.into_iter().enumerate() {
            if knob_mod_model.row_data(row).as_ref() != Some(&modulation) {
                knob_mod_model.set_row_data(row, modulation);
            }
        }
    })
}

/// Updates `model` to `rows`, touching only rows that changed.
fn sync_model<T: Clone + PartialEq + 'static>(model: &slint::VecModel<T>, rows: Vec<T>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (index, row) in rows.into_iter().enumerate() {
        if index >= model.row_count() {
            model.push(row);
        } else if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

fn oscillator_row(
    state: &PluginContext<SynthParams>,
    params: &crate::plugin::OscillatorParamIds,
) -> OscillatorRow {
    let last = (OscillatorType::variant_count() - 1) as f32;
    OscillatorRow {
        waveform: (state.get_param(params.waveform) * last).round() as i32,
        phase: state.get_param(params.phase),
        phase_text: state.format_param(params.phase).into(),
        pitch: state.get_param(params.pitch),
        pitch_text: state.format_param(params.pitch).into(),
        level: state.get_param(params.level),
        level_text: state.format_param(params.level).into(),
    }
}

fn set_param(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    id: SynthParamsParamId,
    normalized: f64,
) {
    state.params().set_normalized(id.into(), normalized);
    enqueue_edit(edits, (id, normalized));
}

fn lfo_count_to_normalized(count: usize) -> f64 {
    count.min(MAX_LFOS) as f64 / MAX_LFOS as f64
}

fn reset_lfo(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    index: usize,
) {
    reset_modulator(state, edits, &LFO_PARAMS[index].all());
    state.params().set_custom_lfo_shape(index, None);
}

fn reset_modulator(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    ids: &[SynthParamsParamId],
) {
    let infos = state.params().param_infos();
    for &id in ids {
        let info = infos
            .iter()
            .find(|info| info.id == u32::from(id))
            .expect("modulator parameters must have parameter metadata");
        set_param(state, edits, id, info.range.normalize(info.default_plain));
    }
}

fn remove_envelope(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    removed: usize,
) {
    let count = state.params().env_count.value_usize();
    if removed >= count {
        return;
    }
    let values = ENV_PARAMS.map(|ids| ids.all().map(|id| f64::from(state.get_param(id))));
    for index in removed..count - 1 {
        for (id, value) in ENV_PARAMS[index].all().into_iter().zip(values[index + 1]) {
            set_param(state, edits, id, value);
        }
    }
    reset_modulator(state, edits, &ENV_PARAMS[count - 1].all());
    set_param(
        state,
        edits,
        SynthParamsParamId::EnvCount,
        (count - 1) as f64 / MAX_ENVELOPES as f64,
    );
}

fn remove_lfo(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    removed: usize,
) {
    let count = state.params().lfo_count.value_usize();
    if removed >= count {
        return;
    }
    let values = LFO_PARAMS.map(|ids| ids.all().map(|id| f64::from(state.get_param(id))));
    for index in removed..count - 1 {
        for (id, value) in LFO_PARAMS[index].all().into_iter().zip(values[index + 1]) {
            set_param(state, edits, id, value);
        }
    }
    let mut shapes = state.params().custom_lfo_shapes();
    shapes.0.copy_within(removed + 1..count, removed);
    state.params().set_custom_lfo_shapes(shapes);
    reset_lfo(state, edits, count - 1);
    set_param(
        state,
        edits,
        SynthParamsParamId::LfoCount,
        lfo_count_to_normalized(count - 1),
    );
}

/// Removes oscillator `removed`: later oscillators move down to fill its
/// place, the last one returns to its defaults, and LFO routes follow the
/// oscillators they target, those to the removed one being cleared.
fn remove_oscillator(
    state: &PluginContext<SynthParams>,
    edits: &Rc<RefCell<Vec<(SynthParamsParamId, f64)>>>,
    removed: usize,
) {
    let count = state.params().osc_count.value_usize();
    if count <= 1 || removed >= count {
        return;
    }

    let infos = state.params().param_infos();
    let default_normalized = |id: SynthParamsParamId| {
        let id = u32::from(id);
        infos
            .iter()
            .find(|info| info.id == id)
            .map_or(0.0, |info| info.range.normalize(info.default_plain))
    };
    let values =
        OSCILLATOR_PARAMS.map(|params| params.all().map(|id| f64::from(state.get_param(id))));
    let defaults = OSCILLATOR_PARAMS[0].all().map(default_normalized);
    let shifted = oscillators_after_removal(values, defaults, count, removed);
    for (params, (old, new)) in OSCILLATOR_PARAMS.iter().zip(values.iter().zip(&shifted)) {
        for (id, (old, new)) in params.all().into_iter().zip(old.iter().zip(new)) {
            if old != new {
                set_param(state, edits, id, *new);
            }
        }
    }

    for source in (0..MAX_LFOS)
        .map(Modulator::Lfo)
        .chain((0..MAX_ENVELOPES).map(Modulator::Envelope))
    {
        let (destinations, amounts) = source.routing();
        let routes = read_modulator_routes(state, source);
        let remapped = routes_after_removal(&routes, removed);
        for (slot, (old, new)) in routes.iter().zip(&remapped).enumerate() {
            if old.destination != new.destination {
                set_param(
                    state,
                    edits,
                    destinations[slot],
                    destination_to_normalized(new.destination),
                );
            }
            if old.amount != new.amount {
                set_param(
                    state,
                    edits,
                    amounts[slot],
                    amount_to_normalized(new.amount),
                );
            }
        }
    }

    set_param(
        state,
        edits,
        SynthParamsParamId::OscCount,
        oscillator_count_to_normalized(count - 1),
    );
}

/// Each oscillator's normalized parameter values once oscillator `removed`
/// of the first `count` is taken out: later ones move down a place and the
/// vacated last place gets `defaults`.
fn oscillators_after_removal<const N: usize>(
    mut values: [[f64; N]; MAX_OSCILLATORS],
    defaults: [f64; N],
    count: usize,
    removed: usize,
) -> [[f64; N]; MAX_OSCILLATORS] {
    let count = count.min(MAX_OSCILLATORS);
    if removed >= count {
        return values;
    }
    values.copy_within(removed + 1..count, removed);
    values[count - 1] = defaults;
    values
}

/// Routes once oscillator `removed` is taken out: routes to it are cleared
/// and routes to later oscillators follow them down a place.
fn routes_after_removal(routes: &[ModRoute; MOD_SLOTS], removed: usize) -> [ModRoute; MOD_SLOTS] {
    routes.map(|route| {
        let destination = match route.destination {
            Some(destination) if destination.oscillator() == Some(removed) => {
                return ModRoute::default();
            }
            Some(ModDestination::OscPitch(index)) if index > removed => {
                Some(ModDestination::OscPitch(index - 1))
            }
            Some(ModDestination::OscLevel(index)) if index > removed => {
                Some(ModDestination::OscLevel(index - 1))
            }
            destination => destination,
        };
        ModRoute {
            destination,
            amount: route.amount,
        }
    })
}

fn oscillator_count_to_normalized(count: usize) -> f64 {
    let last = (MAX_OSCILLATORS - 1) as f64;
    ((count as f64 - 1.0) / last).clamp(0.0, 1.0)
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

/// SVG path commands tracing one cycle of `waveform`, starting at normalized
/// `start_phase`, across a `width` by `height` box: 0 degrees after the start
/// on the left, 360 on the right, +1 at the top.
fn waveform_cycle_path(waveform: Waveform, start_phase: f32, width: f32, height: f32) -> String {
    let mut samples = [0.0_f32; CYCLE_PATH_SEGMENTS + 1];
    render_cycle(waveform, start_phase, &mut samples);
    cycle_samples_path(&samples, width, height)
}

/// SVG path commands tracing one cycle of `shape` across a `width` by
/// `height` box, laid out like `waveform_cycle_path`. Vertices are added on
/// both sides of every node so jumps are drawn vertical. `filled` closes the
/// path along the zero line for the area under the curve.
fn lfo_shape_path(shape: &LfoShape, width: f32, height: f32, filled: bool) -> String {
    const NODE_EDGE: f32 = 1e-4;
    let mut phases: Vec<f32> = (0..=CYCLE_PATH_SEGMENTS)
        .map(|index| index as f32 / CYCLE_PATH_SEGMENTS as f32)
        .collect();
    for point in shape.points() {
        phases.extend(
            [point.x - NODE_EDGE, point.x]
                .into_iter()
                .filter(|x| (0.0..=1.0).contains(x)),
        );
    }
    phases.sort_by(f32::total_cmp);
    phases.dedup();
    let mut commands = String::with_capacity(phases.len() * 16 + 48);
    for (index, &phase) in phases.iter().enumerate() {
        // The cycle's right edge is the limit approaching the wrap.
        let value = shape.sample(if phase >= 1.0 { 1.0 - NODE_EDGE } else { phase });
        let x = width * phase;
        let y = height * (1.0 - value.clamp(-1.0, 1.0)) / 2.0;
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(commands, "{command}{x:.2} {y:.2} ");
    }
    if filled {
        let middle = height / 2.0;
        let _ = write!(commands, "L{width:.2} {middle:.2} L0 {middle:.2} Z");
    }
    commands
}

/// Applies an edit to the selected LFO's shape.
type LfoShapeEdit = dyn Fn(&dyn Fn(&mut LfoShape));

/// The selected LFO's nodes and bend handles as shown in the editor.
struct LfoShapeView {
    nodes: Rc<slint::VecModel<LfoNode>>,
    handles: Rc<slint::VecModel<LfoCurveHandle>>,
    shown: Cell<Option<LfoShape>>,
}

impl LfoShapeView {
    fn new(ui: &SynthUi) -> Self {
        let nodes = Rc::new(slint::VecModel::default());
        let handles = Rc::new(slint::VecModel::default());
        ui.set_lfo_nodes(slint::ModelRc::from(nodes.clone()));
        ui.set_lfo_curve_handles(slint::ModelRc::from(handles.clone()));
        Self {
            nodes,
            handles,
            shown: Cell::new(None),
        }
    }

    /// Shows `shape`, touching the UI only when it changed.
    fn show(&self, ui: &SynthUi, shape: LfoShape) {
        if self.shown.get() == Some(shape) {
            return;
        }
        self.shown.set(Some(shape));
        let (nodes, handles) = lfo_shape_view_rows(&shape);
        sync_model(&self.nodes, nodes);
        sync_model(&self.handles, handles);
        ui.set_lfo_smooth(shape.is_smooth());
        ui.set_lfo_shape_revision(ui.get_lfo_shape_revision().wrapping_add(1));
    }
}

/// Node rows and one bend handle per segment; handles of jumps and flat
/// segments, which bending can't change, are hidden.
fn lfo_shape_view_rows(shape: &LfoShape) -> (Vec<LfoNode>, Vec<LfoCurveHandle>) {
    let points = shape.points();
    let nodes = points
        .iter()
        .map(|point| LfoNode {
            x: point.x,
            y: point.y,
        })
        .collect();
    let handles = (0..points.len())
        .map(|index| {
            let (x, y) = shape.segment_midpoint(index).unwrap_or_default();
            LfoCurveHandle {
                x,
                y,
                visible: lfo_segment_direction(shape, index).is_some(),
            }
        })
        .collect();
    (nodes, handles)
}

/// Whether the segment leaving node `index` rises (1) or falls (-1); `None`
/// for jumps and flat segments.
fn lfo_segment_direction(shape: &LfoShape, index: usize) -> Option<f32> {
    let points = shape.points();
    let start = points.get(index)?;
    let (end, end_x) = match points.get(index + 1) {
        Some(next) => (next, next.x),
        None => (&points[0], points[0].x + 1.0),
    };
    let rise = end.y - start.y;
    (end_x > start.x && rise.abs() > 1e-6).then(|| rise.signum())
}

/// Bend change per unit of vertical drag (the plot spans 2 units).
const LFO_CURVE_DRAG_SCALE: f32 = 8.0;

/// The bend of segment `index` after dragging its handle `amount` value units
/// up from where the drag began with bend `start`: the middle of the curve
/// follows the pointer.
fn dragged_lfo_curve(shape: &LfoShape, index: usize, start: f32, amount: f32) -> Option<f32> {
    let direction = lfo_segment_direction(shape, index)?;
    Some(start - direction * amount * LFO_CURVE_DRAG_SCALE)
}

/// Grid used when snapping LFO nodes: sixteenths of the cycle, eighths of
/// the value range.
fn snap_lfo_point(x: f32, y: f32, snap: bool) -> (f32, f32) {
    if snap {
        ((x * 16.0).round() / 16.0, (y * 8.0).round() / 8.0)
    } else {
        (x, y)
    }
}

/// Names of the Shape dropdown entries: the Shape parameter's waveforms,
/// then node presets applied as custom shapes, then "Custom" for edits.
#[cfg(test)]
const LFO_PRESET_NAMES: [&str; 9] = [
    "Sine", "Square", "Triangle", "Sawtooth", "Saw Down", "Exp Rise", "Exp Fall", "Steps", "Custom",
];
const LFO_CUSTOM_INDEX: i32 = 8;

#[derive(Clone, Debug, PartialEq)]
enum LfoPreset {
    /// A value of the LFO's Shape parameter.
    Parameter(Waveform),
    /// A node layout applied as a custom shape.
    Nodes(Box<LfoShape>),
}

impl LfoPreset {
    fn from_index(index: i32) -> Option<Self> {
        let parameter_count = LfoShapeType::variant_count() as i32;
        match index {
            0.. if index < parameter_count => Some(Self::Parameter(lfo_shape_from_index(index))),
            4 => Some(Self::Nodes(Box::new(LfoShape::new(
                &[LfoPoint::new(0.5, -1.0), LfoPoint::new(0.5, 1.0)],
                false,
            )))),
            5 => Some(Self::Nodes(Box::new(LfoShape::new(
                &[LfoPoint::curved(0.0, -1.0, 5.0), LfoPoint::new(1.0, 1.0)],
                false,
            )))),
            6 => Some(Self::Nodes(Box::new(LfoShape::new(
                &[LfoPoint::curved(0.0, 1.0, -5.0), LfoPoint::new(1.0, -1.0)],
                false,
            )))),
            7 => {
                let levels = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
                let points: Vec<_> = levels
                    .iter()
                    .enumerate()
                    .flat_map(|(step, &level)| {
                        let start = step as f32 / 4.0;
                        [
                            LfoPoint::new(start, level),
                            LfoPoint::new(start + 0.25, level),
                        ]
                    })
                    .collect();
                Some(Self::Nodes(Box::new(LfoShape::new(&points, false))))
            }
            _ => None,
        }
    }

    fn shape(self) -> LfoShape {
        match self {
            Self::Parameter(waveform) => LfoShape::from_waveform(waveform),
            Self::Nodes(shape) => *shape,
        }
    }
}

/// The Shape dropdown entry for an LFO with Shape parameter index `preset`
/// and node edits `custom`.
fn lfo_dropdown_index(preset: i32, custom: Option<LfoShape>) -> i32 {
    let Some(custom) = custom else {
        return preset;
    };
    (0..LFO_CUSTOM_INDEX)
        .find(|&index| LfoPreset::from_index(index).is_some_and(|p| p.shape() == custom))
        .unwrap_or(LFO_CUSTOM_INDEX)
}

/// Spreads `samples` evenly from left to right, +1 at the top.
fn cycle_samples_path(samples: &[f32], width: f32, height: f32) -> String {
    let segments = samples.len().saturating_sub(1).max(1) as f32;
    let mut commands = String::with_capacity(samples.len() * 16);
    for (index, sample) in samples.iter().enumerate() {
        let x = width * index as f32 / segments;
        let y = height * (1.0 - sample.clamp(-1.0, 1.0)) / 2.0;
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(commands, "{command}{x:.2} {y:.2} ");
    }
    commands
}

fn lfo_shape_to_normalized(index: i32) -> f64 {
    let last = (LfoShapeType::variant_count() - 1) as f64;
    (f64::from(index) / last).clamp(0.0, 1.0)
}

fn lfo_shape_from_index(index: i32) -> Waveform {
    let last = LfoShapeType::variant_count() - 1;
    LfoShapeType::from_index(usize::try_from(index).unwrap_or(0).min(last)).into()
}

/// One marker per voice slot; slots without a running LFO are hidden.
fn lfo_markers(
    phases: [Option<f32>; MAX_VOICES],
    newest: Option<usize>,
) -> [PositionMarker; MAX_VOICES] {
    std::array::from_fn(|slot| match phases[slot] {
        Some(position) => PositionMarker {
            position,
            visible: true,
            newest: newest == Some(slot),
        },
        None => PositionMarker::default(),
    })
}

fn lfo_mode_to_normalized(index: i32) -> f64 {
    let last = (LfoModeType::variant_count() - 1) as f64;
    (f64::from(index) / last).clamp(0.0, 1.0)
}

fn filter_type_to_normalized(index: i32) -> f64 {
    let last = (FilterType::variant_count() - 1) as f64;
    (f64::from(index) / last).clamp(0.0, 1.0)
}

fn filter_mode_from_index(index: i32) -> FilterMode {
    let last = FilterType::variant_count() - 1;
    FilterType::from_index(usize::try_from(index).unwrap_or(0).min(last)).into()
}

fn filter_parameter(id: i32) -> Option<SynthParamsParamId> {
    match id {
        0 => Some(SynthParamsParamId::FilterCutoff),
        1 => Some(SynthParamsParamId::FilterQ),
        2 => Some(SynthParamsParamId::FilterMix),
        _ => None,
    }
}

/// Points along the plotted frequency response.
const RESPONSE_PATH_SEGMENTS: usize = 192;
/// The editor doesn't know the host sample rate, so the response is drawn as
/// the filter behaves at this typical rate.
const RESPONSE_SAMPLE_RATE: f32 = 48_000.0;
/// Gain at the top and bottom of the plot. Must match the grid in
/// `FilterResponseDisplay`.
const RESPONSE_TOP_DB: f32 = 24.0;
const RESPONSE_BOTTOM_DB: f32 = -48.0;

/// SVG path commands tracing the filter's magnitude response across a
/// `width` by `height` box: 20 Hz to 20 kHz on a log scale from left to right,
/// `RESPONSE_TOP_DB` at the top and `RESPONSE_BOTTOM_DB` at the bottom.
/// Evaluated from the same coefficients the DSP runs with.
fn filter_response_path(settings: FilterSettings, width: f32, height: f32) -> String {
    let coefficients = BiquadCoefficients::new(settings, RESPONSE_SAMPLE_RATE);
    let octaves = (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2();

    let mut commands = String::with_capacity((RESPONSE_PATH_SEGMENTS + 1) * 16);
    for index in 0..=RESPONSE_PATH_SEGMENTS {
        let position = index as f32 / RESPONSE_PATH_SEGMENTS as f32;
        let frequency = MIN_CUTOFF_HZ * (position * octaves).exp2();
        let gain_db = 20.0
            * coefficients
                .magnitude(frequency, RESPONSE_SAMPLE_RATE)
                .max(1e-6)
                .log10();
        let level = (RESPONSE_TOP_DB - gain_db) / (RESPONSE_TOP_DB - RESPONSE_BOTTOM_DB);
        let x = width * position;
        let y = height * level.clamp(0.0, 1.0);
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(commands, "{command}{x:.2} {y:.2} ");
    }
    commands
}

fn oscillator_params(oscillator: i32) -> Option<&'static crate::plugin::OscillatorParamIds> {
    usize::try_from(oscillator)
        .ok()
        .and_then(|index| OSCILLATOR_PARAMS.get(index))
}

/// An oscillator knob's parameter. Id 0 is pitch, 1 level and 2 phase.
fn oscillator_parameter(oscillator: i32, id: i32) -> Option<SynthParamsParamId> {
    let params = oscillator_params(oscillator)?;
    match id {
        0 => Some(params.pitch),
        1 => Some(params.level),
        2 => Some(params.phase),
        _ => None,
    }
}

/// The knob parameter each modulation destination moves.
fn destination_parameter(destination: ModDestination) -> u32 {
    match destination {
        ModDestination::OscPitch(index) => OSCILLATOR_PARAMS[index.min(MAX_OSCILLATORS - 1)]
            .pitch
            .into(),
        ModDestination::OscLevel(index) => OSCILLATOR_PARAMS[index.min(MAX_OSCILLATORS - 1)]
            .level
            .into(),
        destination => {
            let slot = destination.effect().expect("filter destination");
            let control = (destination.index() - 2 * MAX_OSCILLATORS) % 3;
            crate::plugin::effects::effect_ids(slot)[control + 1]
        }
    }
}

/// How many oscillators the routing dropdowns list: the active ones, plus
/// any a route still targets.
fn shown_oscillators(count: usize, routes: &[ModRoute]) -> usize {
    routes
        .iter()
        .filter_map(|route| route.destination?.oscillator())
        .map(|index| index + 1)
        .fold(count, usize::max)
        .clamp(1, MAX_OSCILLATORS)
}

/// The routing dropdown's choices for `oscillators` oscillators: None, each
/// oscillator's pitch and level, then the filter.
fn destination_options(oscillators: usize) -> Vec<slint::SharedString> {
    std::iter::once(None)
        .chain(dropdown_destinations(oscillators).map(Some))
        .map(|destination| destination_label(destination).into())
        .collect()
}

fn dropdown_destinations(oscillators: usize) -> impl Iterator<Item = ModDestination> {
    (0..oscillators.min(MAX_OSCILLATORS))
        .flat_map(|index| {
            [
                ModDestination::OscPitch(index),
                ModDestination::OscLevel(index),
            ]
        })
        .chain(ModDestination::filter_destinations(0))
}

fn destination_label(destination: Option<ModDestination>) -> String {
    match destination {
        Some(destination) if destination.effect().is_some_and(|slot| slot > 0) => {
            let slot = destination.effect().expect("filter destination");
            let control = (destination.index() - 2 * MAX_OSCILLATORS) % 3;
            format!("Filter {} {}", slot + 1, ["Cutoff", "Q", "Mix"][control])
        }
        destination => ModDestinationType::from(destination).name().to_owned(),
    }
}

fn routing_choices(params: &SynthParams, routes: &[ModRoute]) -> Vec<Option<ModDestination>> {
    let oscillators = shown_oscillators(params.osc_count.value_usize(), routes);
    let mut choices = vec![None];
    choices.extend((0..oscillators).flat_map(|index| {
        [
            Some(ModDestination::OscPitch(index)),
            Some(ModDestination::OscLevel(index)),
        ]
    }));
    let chain = params.effect_chain();
    for &slot in chain.slots() {
        choices.extend(ModDestination::filter_destinations(slot).map(Some));
    }
    for route in routes {
        if route.destination.is_some() && !choices.contains(&route.destination) {
            choices.push(route.destination);
        }
    }
    choices
}

/// A destination's index in `destination_options(oscillators)`.
fn destination_option(oscillators: usize, destination: Option<ModDestination>) -> i32 {
    destination
        .and_then(|destination| {
            dropdown_destinations(oscillators).position(|option| option == destination)
        })
        .map_or(0, |index| index as i32 + 1)
}

#[cfg(test)]
fn destination_from_option(oscillators: usize, index: i32) -> Option<ModDestination> {
    let index = usize::try_from(index).ok()?.checked_sub(1)?;
    dropdown_destinations(oscillators).nth(index)
}

/// A knob's `mod-target` (destination index) as a destination.
fn mod_target(target: i32) -> Option<ModDestination> {
    usize::try_from(target)
        .ok()
        .and_then(|index| ModDestination::ALL.get(index).copied())
}

fn mod_slot(slot: i32) -> Option<usize> {
    usize::try_from(slot).ok().filter(|&slot| slot < MOD_SLOTS)
}

fn read_routes(state: &PluginContext<SynthParams>, lfo: usize) -> [ModRoute; MOD_SLOTS] {
    read_modulator_routes(state, Modulator::Lfo(lfo))
}

#[derive(Clone, Copy)]
enum Modulator {
    Lfo(usize),
    Envelope(usize),
}

impl Modulator {
    fn filters(self) -> [SynthParamsParamId; MOD_SLOTS] {
        match self {
            Self::Lfo(index) => LFO_PARAMS[index].filters,
            Self::Envelope(index) => ENV_PARAMS[index].filters,
        }
    }
    fn routing(
        self,
    ) -> (
        [SynthParamsParamId; MOD_SLOTS],
        [SynthParamsParamId; MOD_SLOTS],
    ) {
        match self {
            Self::Lfo(index) => (LFO_PARAMS[index].destinations, LFO_PARAMS[index].amounts),
            Self::Envelope(index) => (ENV_PARAMS[index].destinations, ENV_PARAMS[index].amounts),
        }
    }

    fn default_amount(self, destination: ModDestination) -> f32 {
        match (self, destination) {
            (Self::Envelope(_), ModDestination::OscLevel(_)) => 1.0,
            _ => default_mod_amount(destination),
        }
    }
}

fn read_modulator_routes(
    state: &PluginContext<SynthParams>,
    source: Modulator,
) -> [ModRoute; MOD_SLOTS] {
    let (destinations, amounts) = source.routing();
    std::array::from_fn(|slot| {
        let destination = state.get_param(destinations[slot]);
        let index = (destination * destination_steps() as f32).round();
        let amount = state.get_param(amounts[slot]);
        ModRoute {
            destination: mod_destination_from_index(index as u32).map(|destination| {
                destination
                    .with_effect((state.get_param(source.filters()[slot]) * 31.0).round() as usize)
            }),
            amount: (2.0 * amount - 1.0).clamp(-1.0, 1.0),
        }
    })
}

fn destination_steps() -> usize {
    ModDestinationType::variant_count() - 1
}

fn destination_to_normalized(destination: Option<ModDestination>) -> f64 {
    f64::from(mod_destination_index(destination)) / destination_steps() as f64
}

/// A bipolar route amount as its parameter's normalized value.
fn amount_to_normalized(amount: f32) -> f64 {
    ((f64::from(amount) + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Depth given to a new route: a semitone, a quarter of the level, an
/// octave of cutoff or a doubling of Q.
fn default_mod_amount(destination: ModDestination) -> f32 {
    match destination {
        ModDestination::OscPitch(_) => 1.0 / (2.0 * MAX_PITCH_SEMITONES),
        ModDestination::OscLevel(_) | ModDestination::FilterMix | ModDestination::EffectMix(_) => {
            0.25
        }
        ModDestination::FilterCutoff | ModDestination::EffectCutoff(_) => {
            1.0 / (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2()
        }
        ModDestination::FilterQ | ModDestination::EffectQ(_) => 1.0 / (MAX_Q / MIN_Q).log2(),
    }
}

/// How far a route of `amount` swings `destination` at the LFO's peak, in the
/// destination's units.
fn format_mod_amount(destination: ModDestination, amount: f32) -> String {
    match destination {
        ModDestination::OscPitch(_) => {
            format!("{:+.2} st", amount * 2.0 * MAX_PITCH_SEMITONES)
        }
        ModDestination::OscLevel(_) | ModDestination::FilterMix | ModDestination::EffectMix(_) => {
            format!("{:+.0} %", amount * 100.0)
        }
        ModDestination::FilterCutoff | ModDestination::EffectCutoff(_) => {
            format!(
                "{:+.2} oct",
                amount * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2()
            )
        }
        ModDestination::FilterQ | ModDestination::EffectQ(_) => {
            let ratio = (amount * (MAX_Q / MIN_Q).log2()).exp2();
            if ratio >= 1.0 {
                format!("×{ratio:.2}")
            } else {
                format!("÷{:.2}", ratio.recip())
            }
        }
    }
}

/// The routing rows, with destinations indexing
/// `destination_options(oscillators)`.
fn mod_route_slots(
    routes: &[ModRoute; MOD_SLOTS],
    oscillators: usize,
) -> [ModRouteSlot; MOD_SLOTS] {
    routes.map(|route| ModRouteSlot {
        destination: destination_option(oscillators, route.destination),
        amount: amount_to_normalized(route.amount) as f32,
        amount_text: route
            .destination
            .map(|destination| format_mod_amount(destination, route.amount))
            .unwrap_or_default()
            .into(),
    })
}

/// The slot a drop onto `destination` should fill: none if it's already
/// routed, otherwise the first empty slot.
fn slot_for_new_route(routes: &[ModRoute], destination: ModDestination) -> Option<usize> {
    if routes
        .iter()
        .any(|route| route.destination == Some(destination))
    {
        return None;
    }
    routes.iter().position(|route| route.destination.is_none())
}

/// The slot and amount that make the total depth on `destination` equal
/// `depth`, adjusting the first route to it and keeping any others.
fn depth_edit(
    routes: &[ModRoute],
    destination: ModDestination,
    depth: f32,
) -> Option<(usize, f32)> {
    let slot = routes
        .iter()
        .position(|route| route.destination == Some(destination))?;
    let others: f32 = routes
        .iter()
        .enumerate()
        .filter(|&(index, route)| index != slot && route.destination == Some(destination))
        .map(|(_, route)| route.amount)
        .sum();
    Some((slot, (depth - others).clamp(-1.0, 1.0)))
}

/// Modulation arcs per destination, in normalized knob units. `bases` are
/// the knobs' values and `lfo` the running LFO's current output, if any.
fn knob_modulation(
    routes: &[ModRoute],
    bases: [f32; ModDestination::ALL.len()],
    lfo: Option<f32>,
) -> [KnobModulation; ModDestination::ALL.len()] {
    ModDestination::ALL.map(|destination| {
        let routed: Vec<_> = routes
            .iter()
            .filter(|route| route.destination == Some(destination))
            .collect();
        let depth = routed
            .iter()
            .map(|route| route.amount)
            .sum::<f32>()
            .clamp(-1.0, 1.0);
        let base = bases[destination.index()];
        KnobModulation {
            routed: !routed.is_empty(),
            depth,
            live: lfo.map_or(base, |lfo| (base + depth * lfo).clamp(0.0, 1.0)),
            live_visible: lfo.is_some() && depth != 0.0,
            unipolar: false,
            level_envelope: false,
        }
    })
}

fn combined_knob_modulation(
    selected_routes: &[ModRoute],
    bases: [f32; ModDestination::ALL.len()],
    sources: &[([ModRoute; MOD_SLOTS], Option<f32>); MAX_LFOS],
) -> [KnobModulation; ModDestination::ALL.len()] {
    let mut mods = knob_modulation(selected_routes, bases, None);
    for destination in ModDestination::ALL {
        let mut offset = 0.0;
        let mut live = false;
        for (routes, value) in sources {
            if let Some(value) = value {
                for route in routes
                    .iter()
                    .filter(|route| route.destination == Some(destination))
                {
                    offset += route.amount * value;
                    live |= route.amount != 0.0;
                }
            }
        }
        let index = destination.index();
        mods[index].live = (bases[index] + offset).clamp(0.0, 1.0);
        mods[index].live_visible = live && mods[index].routed;
    }
    mods
}

fn apply_envelope_knob_modulation(
    mods: &mut [KnobModulation; ModDestination::ALL.len()],
    sources: &[([ModRoute; MOD_SLOTS], Option<f32>); MAX_ENVELOPES],
    envelope_selected: bool,
) {
    let sources = sources.map(|(routes, level)| {
        (
            crate::engine::modulation::ModDepths::from_routes(&routes),
            level,
        )
    });
    for destination in ModDestination::ALL {
        let modulation = &mut mods[destination.index()];
        modulation.unipolar = envelope_selected;
        modulation.level_envelope =
            envelope_selected && matches!(destination, ModDestination::OscLevel(_));
        let mut value = destination.denormalize(modulation.live);
        let mut live = modulation.live_visible;
        for (depths, level) in sources {
            if let Some(level) = level {
                let depth = depths.depth(destination);
                value = destination.modulate_envelope(value, level, depth);
                live |= depth != 0.0;
            }
        }
        modulation.live = destination.normalize(value);
        modulation.live_visible = live && modulation.routed;
    }
}

fn envelope_parameter(envelope: usize, id: i32) -> Option<SynthParamsParamId> {
    usize::try_from(id)
        .ok()
        .and_then(|id| ENV_PARAMS[envelope].adsr.get(id).copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_time_axis_is_zero_safe_logarithmic_and_invertible() {
        assert_eq!(envelope_time_position(0.0), 0.0);
        assert_eq!(envelope_position_time(0.0), 0.0);
        assert_eq!(envelope_time_position(30_000.0), 1.0);
        for time in [0.0, 1.0, 10.0, 100.0, 1000.0, 10_000.0, 30_000.0] {
            let restored = envelope_position_time(envelope_time_position(time));
            assert!((restored - time).abs() <= 0.00001 * time.max(1.0));
        }
        let decade_steps: Vec<_> = [10.0, 100.0, 1000.0, 10_000.0]
            .windows(2)
            .map(|times| envelope_time_position(times[1]) - envelope_time_position(times[0]))
            .collect();
        assert!((decade_steps[0] - decade_steps[2]).abs() < 0.01);
        assert_eq!(envelope_time_position(-1.0), 0.0);
        assert_eq!(envelope_time_position(40_000.0), 1.0);
    }

    #[test]
    fn envelope_knob_markers_match_level_multiplication_after_lfo_offsets() {
        let routes = [
            ModRoute {
                destination: Some(ModDestination::OscLevel(0)),
                amount: 1.0,
            },
            ModRoute::default(),
            ModRoute::default(),
            ModRoute::default(),
        ];
        let lfo_routes = [
            ModRoute {
                destination: Some(ModDestination::OscLevel(0)),
                amount: 0.25,
            },
            ModRoute::default(),
            ModRoute::default(),
            ModRoute::default(),
        ];
        let mut lfos = [([ModRoute::default(); MOD_SLOTS], None); MAX_LFOS];
        lfos[0] = (lfo_routes, Some(1.0));
        let mut envelopes = [([ModRoute::default(); MOD_SLOTS], None); MAX_ENVELOPES];
        envelopes[0] = (routes, Some(0.5));
        let mut mods = combined_knob_modulation(&routes, [0.25; ModDestination::ALL.len()], &lfos);
        apply_envelope_knob_modulation(&mut mods, &envelopes, true);
        let level = &mods[ModDestination::OscLevel(0).index()];
        assert_eq!(level.live, 0.25);
        assert!(level.live_visible && level.unipolar && level.level_envelope);
        envelopes[0].1 = None;
        let mut mods = combined_knob_modulation(&routes, [0.25; ModDestination::ALL.len()], &lfos);
        apply_envelope_knob_modulation(&mut mods, &envelopes, false);
        assert_eq!(mods[1].live, 0.5);
        assert!(!mods[1].unipolar);
    }

    #[test]
    fn envelopes_share_modulator_routing_and_preserve_independent_parameters() {
        use slint::ComponentHandle;
        use slint::platform::software_renderer::PremultipliedRgbaColor;

        truce_slint::platform::ensure_platform();
        let window = truce_slint::platform::create_slint_window();
        window.set_size(slint::PhysicalSize::new(1100, 1100));
        let ui = SynthUi::new().unwrap();
        let params = Arc::new(SynthParams::default());
        let state = editor_test_context(params.clone());
        let sync = setup_editor(state.clone(), ui.clone_strong());
        sync(&state);
        assert_eq!(ui.get_env_count(), 1);
        assert!(ui.get_envelope_selected());
        assert_eq!(ui.get_current_envelope(), 0);
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[0],
            ModRoute {
                destination: Some(ModDestination::OscLevel(0)),
                amount: 1.0
            }
        );
        assert!(ui.get_knob_mods().row_data(1).unwrap().level_envelope);
        ui.show().unwrap();
        let mut pixels = vec![PremultipliedRgbaColor::default(); 1100 * 1100];
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1100);
        }));
        let dark_rows = (150..400)
            .filter(|&y| {
                (570..1070)
                    .filter(|&x| {
                        let pixel = pixels[y * 1100 + x];
                        (pixel.red, pixel.green, pixel.blue) == (17, 19, 25)
                    })
                    .count()
                    >= 450
            })
            .collect::<Vec<_>>();
        assert!(
            dark_rows.len() >= 140,
            "ENV 1 must display its ADSR plot in the Modulators column"
        );

        let original_attack = state.get_param(ENV_PARAMS[0].adsr[0]);
        let grid_columns = |y: usize| {
            (570..1070)
                .filter(|&x| {
                    let pixel = pixels[y * 1100 + x];
                    (pixel.red, pixel.green, pixel.blue) == (40, 44, 52)
                })
                .collect::<Vec<_>>()
        };
        let grid_rows: Vec<_> = (150..400)
            .filter(|&y| grid_columns(y).len() >= 400)
            .collect();
        let plot_top = grid_rows[0];
        let plot_bottom = *grid_rows.last().unwrap();
        let top_columns = grid_columns(plot_top);
        let plot_left = top_columns[0] as f32;
        let plot_right = (top_columns.last().unwrap() + 1) as f32;
        let plot_width = plot_right - plot_left;
        for attack in [0.0, 0.251, 0.549, 1.0] {
            ui.invoke_envelope_changed(0, attack);
            ui.invoke_envelope_released(0);
            sync(&state);
            assert_eq!(ui.get_attack(), attack);
            window.request_redraw();
            assert!(window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 1100);
            }));
            let blue_columns = |y: usize| {
                (570..1070)
                    .filter(|&x| {
                        let pixel = pixels[y * 1100 + x];
                        (pixel.red, pixel.green, pixel.blue) == (86, 167, 255)
                    })
                    .collect::<Vec<_>>()
            };
            let peak_columns = blue_columns(plot_top);
            let peak_center = (peak_columns[0] + peak_columns.last().unwrap()) as f32 / 2.0;
            let attack_ms = params.attack.info.range.denormalize(f64::from(attack)) as f32;
            let expected_peak = plot_left + envelope_time_position(attack_ms) * plot_width;
            assert!(
                (peak_center - expected_peak).abs() <= 1.5,
                "attack peak must follow logarithmic time: {peak_center} vs {expected_peak}"
            );
            let endpoint_columns = blue_columns(plot_bottom);
            let end = *endpoint_columns.last().unwrap();
            let endpoint_start = endpoint_columns
                .iter()
                .rev()
                .take_while(|&&x| end - x <= 12)
                .last()
                .unwrap();
            let endpoint_center = (endpoint_start + end) as f32 / 2.0;
            let total_ms = attack_ms + params.decay.value() + params.release.value();
            let expected_end = plot_left + envelope_time_position(total_ms) * plot_width;
            assert!(
                (endpoint_center - expected_end).abs() <= 1.5,
                "release endpoint must follow cumulative time: {endpoint_center} vs {expected_end}"
            );
            let decay_time = attack_ms + params.decay.value();
            let expected_middle = plot_left + envelope_time_position(decay_time) * plot_width;
            let sustain_y = (plot_bottom as f32
                - ui.get_sustain_level() * (plot_bottom - plot_top) as f32)
                .round() as usize;
            let middle_columns: Vec<_> = blue_columns(sustain_y)
                .into_iter()
                .filter(|&x| (x as f32 - expected_middle).abs() <= 7.0)
                .collect();
            assert!(
                middle_columns.len() >= 8,
                "decay handle must be at attack + decay"
            );
            assert!(endpoint_center < plot_right - 10.0);
        }
        ui.invoke_envelope_changed(0, original_attack);
        ui.invoke_envelope_released(0);
        sync(&state);

        for id in [0, 1, 3] {
            let values = [
                params.attack.value(),
                params.decay.value(),
                params.release.value(),
            ];
            let (offset, duration, y) = match id {
                0 => (0.0, values[0], plot_top as f32),
                1 => (
                    values[0],
                    values[1],
                    plot_bottom as f32 - ui.get_sustain_level() * (plot_bottom - plot_top) as f32,
                ),
                3 => (values[0] + values[1], values[2], plot_bottom as f32),
                _ => unreachable!(),
            };
            let original = state.get_param(ENV_PARAMS[0].adsr[id as usize]);
            let position = envelope_time_position(offset + duration);
            let start = slint::LogicalPosition::new(plot_left + position * plot_width, y);
            let end = slint::LogicalPosition::new(start.x + 10.0, y);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                    position: start,
                    button: slint::platform::PointerEventButton::Left,
                });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved { position: end });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                    position: end,
                    button: slint::platform::PointerEventButton::Left,
                });
            sync(&state);
            let expected_ms = (envelope_position_time(position + 10.0 / plot_width) - offset)
                .clamp(0.0, 10_000.0);
            let expected_value = params.attack.info.range.normalize(f64::from(expected_ms)) as f32;
            assert!(
                (state.get_param(ENV_PARAMS[0].adsr[id as usize]) - expected_value).abs() < 0.002,
                "dragging envelope point {id} must invert the logarithmic axis"
            );
            ui.invoke_envelope_changed(id, original);
            ui.invoke_envelope_released(id);
            sync(&state);
        }

        for index in 1..MAX_ENVELOPES {
            ui.invoke_envelope_add();
            sync(&state);
            assert_eq!(ui.get_current_envelope(), index as i32);
            assert_eq!(
                read_modulator_routes(&state, Modulator::Envelope(index)),
                [ModRoute::default(); MOD_SLOTS]
            );
            ui.invoke_envelope_changed(0, index as f32 / 4.0);
            ui.invoke_envelope_released(0);
            ui.invoke_mod_assign(ModDestination::FilterCutoff.index() as i32);
            sync(&state);
            assert_eq!(
                read_modulator_routes(&state, Modulator::Envelope(index))[0].destination,
                Some(ModDestination::FilterCutoff)
            );
        }
        ui.invoke_envelope_add();
        sync(&state);
        assert_eq!(ui.get_env_count(), MAX_ENVELOPES as i32);
        let before = ENV_PARAMS.map(|ids| ids.all().map(|id| state.get_param(id)));
        ui.invoke_envelope_remove(1);
        sync(&state);
        assert_eq!(ui.get_env_count(), 3);
        assert_eq!(ENV_PARAMS[1].all().map(|id| state.get_param(id)), before[2]);
        assert_eq!(ENV_PARAMS[2].all().map(|id| state.get_param(id)), before[3]);
        assert_eq!(ENV_PARAMS[0].all().map(|id| state.get_param(id)), before[0]);

        ui.invoke_lfo_add();
        sync(&state);
        assert!(!ui.get_envelope_selected());
        window.request_redraw();
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1100);
        }));
        let click_tab = |x| {
            let position = slint::LogicalPosition::new(x, 120.0);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                });
        };
        click_tab(570.0);
        sync(&state);
        assert!(
            ui.get_envelope_selected(),
            "ENV and LFO tabs must be individually clickable"
        );
        assert_eq!(ui.get_current_envelope(), 0);
        click_tab(790.0);
        sync(&state);
        assert!(
            !ui.get_envelope_selected(),
            "LFO tabs must sit after the ENV tabs"
        );
        ui.invoke_mod_assign(ModDestination::OscPitch(0).index() as i32);
        sync(&state);
        assert_eq!(
            read_routes(&state, 0)[0].destination,
            Some(ModDestination::OscPitch(0))
        );
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[0].destination,
            Some(ModDestination::OscLevel(0))
        );
        for _ in 0..3 {
            ui.invoke_envelope_remove(0);
            sync(&state);
        }
        assert_eq!(ui.get_env_count(), 0);
        assert!(!ui.get_envelope_selected());
        ui.invoke_envelope_add();
        sync(&state);
        assert!(ui.get_envelope_selected());
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[0].amount,
            1.0
        );
        assert_eq!(params.attack.value(), SynthParams::default().attack.value());

        params.set_normalized(SynthParamsParamId::OscCount.into(), 1.0 / 3.0);
        set_param(
            &state,
            &Rc::new(RefCell::new(Vec::new())),
            ENV_PARAMS[0].destinations[0],
            destination_to_normalized(Some(ModDestination::OscLevel(1))),
        );
        ui.invoke_oscillator_remove(0);
        sync(&state);
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[0].destination,
            Some(ModDestination::OscLevel(0))
        );
        assert_eq!(read_routes(&state, 0)[0], ModRoute::default());
    }

    #[test]
    fn effects_chain_editor_adds_edits_pointer_reorders_removes_and_caps_at_32() {
        use crate::plugin::effects::effect_ids;
        use slint::ComponentHandle;
        use slint::platform::software_renderer::PremultipliedRgbaColor;

        truce_slint::platform::ensure_platform();
        let window = truce_slint::platform::create_slint_window();
        window.set_size(slint::PhysicalSize::new(1100, 1500));
        let ui = SynthUi::new().unwrap();
        let params = Arc::new(SynthParams::default());
        let state = editor_test_context(params.clone());
        let sync = setup_editor(state.clone(), ui.clone_strong());
        sync(&state);
        assert_eq!(ui.get_effects().row_count(), 1);
        ui.invoke_effect_add();
        sync(&state);
        assert_eq!(params.effect_chain().slots(), &[0, 1]);
        ui.invoke_effect_changed(1, 0, 0.2);
        ui.invoke_effect_released(1, 0);
        ui.invoke_mod_assign(ModDestination::EffectCutoff(1).index() as i32);
        sync(&state);
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[1].destination,
            Some(ModDestination::EffectCutoff(1))
        );
        assert_eq!(ui.get_effects().row_data(1).unwrap().cutoff, 0.2);

        ui.show().unwrap();
        let mut pixels = vec![PremultipliedRgbaColor::default(); 1100 * 1500];
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1100);
        }));
        let top = (300..1200)
            .find(|&y| {
                let pixel = pixels[y * 1100 + 18];
                (pixel.red, pixel.green, pixel.blue) == (52, 57, 67)
            })
            .expect("visible effects card border");
        let start = slint::LogicalPosition::new(50.0, top as f32 + 16.0);
        let end = slint::LogicalPosition::new(410.0, top as f32 + 16.0);
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                position: start,
                button: slint::platform::PointerEventButton::Left,
            });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position: end });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position: end,
                button: slint::platform::PointerEventButton::Left,
            });
        sync(&state);
        assert_eq!(
            params.effect_chain().slots(),
            &[1, 0],
            "title drag must reorder actual cards"
        );
        assert_eq!(ui.get_effects().row_data(0).unwrap().slot, 1);
        assert!((params.get_normalized(effect_ids(1)[1]).unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[1].destination,
            Some(ModDestination::EffectCutoff(1))
        );

        ui.invoke_effect_remove(1);
        sync(&state);
        assert_eq!(
            read_modulator_routes(&state, Modulator::Envelope(0))[1],
            ModRoute::default()
        );
        ui.invoke_effect_remove(0);
        sync(&state);
        assert_eq!(ui.get_effects().row_count(), 0);
        window.request_redraw();
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1100);
        }));
        for _ in 0..33 {
            ui.invoke_effect_add();
            sync(&state);
        }
        assert_eq!(ui.get_effects().row_count(), 32);
        assert_eq!(params.effect_chain().slots(), &(0..32).collect::<Vec<_>>());
        assert!((params.get_plain(effect_ids(1)[1]).unwrap() - 20_000.0).abs() < 1e-6);
        window.request_redraw();
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1100);
        }));
        ui.invoke_effect_moved(31, 0);
        sync(&state);
        assert_eq!(params.effect_chain().slots()[0], 31);
        ui.invoke_effect_remove(15);
        sync(&state);
        assert_eq!(ui.get_effects().row_count(), 31);
        assert!(!params.effect_chain().slots().contains(&15));
    }

    fn editor_test_context(params: Arc<SynthParams>) -> PluginContext<SynthParams> {
        use truce_slint::truce_core::editor::ClosureBridge;
        let set_params = params.clone();
        let get_params = params.clone();
        let plain_params = params.clone();
        let format_params = params.clone();
        let bridge = ClosureBridge {
            begin_edit: Box::new(|_| {}),
            set_param: Box::new(move |id, value| set_params.set_normalized(id, value)),
            end_edit: Box::new(|_| {}),
            request_resize: Box::new(|_, _| false),
            get_param: Box::new(move |id| get_params.get_normalized(id).unwrap()),
            get_param_plain: Box::new(move |id| plain_params.get_plain(id).unwrap()),
            format_param: Box::new(move |id| {
                format_params
                    .format_value(id, format_params.get_plain(id).unwrap())
                    .unwrap()
            }),
            get_meter: Box::new(|_| 0.0),
            get_state: Box::new(Vec::new),
            set_state: Box::new(|_| panic!("unexpected custom-state edit")),
            transport: Box::new(|| None),
        };
        PluginContext::new(Arc::new(bridge), params)
    }

    #[test]
    fn lfo_nodes_are_edited_from_the_plot() {
        use slint::Model;

        truce_slint::platform::ensure_platform();
        let ui = SynthUi::new().unwrap();
        let params = Arc::new(SynthParams::default());
        params.set_normalized(SynthParamsParamId::EnvCount.into(), 0.0);
        let state = editor_test_context(params.clone());
        let sync = setup_editor(state.clone(), ui.clone_strong());
        ui.invoke_lfo_add();
        ui.set_selected_lfo(1);
        ui.invoke_lfo_add();
        sync(&state);
        assert_eq!(ui.get_current_lfo(), 1);

        ui.invoke_lfo_shape_selected(2);
        sync(&state);
        assert_eq!(ui.get_lfo_shape(), 2);
        assert_eq!(params.custom_lfo_shape(1), None);
        assert_eq!(ui.get_lfo_nodes().row_count(), 2);
        let revision = ui.get_lfo_shape_revision();

        ui.invoke_lfo_node_added(0.5, 0.33, true);
        sync(&state);
        assert_eq!(ui.get_lfo_shape(), LFO_CUSTOM_INDEX);
        assert_ne!(ui.get_lfo_shape_revision(), revision);
        let nodes = ui.get_lfo_nodes();
        assert_eq!(nodes.row_count(), 3);
        assert_eq!(nodes.row_data(1).unwrap(), LfoNode { x: 0.5, y: 0.375 });
        assert_eq!(params.custom_lfo_shape(1).unwrap().points().len(), 3);
        assert_eq!(params.custom_lfo_shape(0), None);

        ui.invoke_lfo_node_moved(1, 0.6, -0.5, false);
        assert_eq!(
            ui.get_lfo_nodes().row_data(1).unwrap(),
            LfoNode { x: 0.6, y: -0.5 }
        );

        // Segment 0 falls from 1 to -0.5; dragging its handle up bends it.
        let before = params.lfo_shape(1).sample(0.425);
        ui.invoke_lfo_curve_pressed(0);
        ui.invoke_lfo_curve_dragged(0, 0.3);
        assert!(params.lfo_shape(1).sample(0.425) > before);
        ui.invoke_lfo_curve_reset(0);
        assert_eq!(params.lfo_shape(1).points()[0].curve, 0.0);

        ui.invoke_lfo_smooth_toggled();
        assert!(ui.get_lfo_smooth() && params.lfo_shape(1).is_smooth());
        ui.invoke_lfo_node_removed(1);
        assert_eq!(ui.get_lfo_nodes().row_count(), 2);
        assert!(!ui.invoke_lfo_shape_path(0, 100.0, 40.0, false).is_empty());

        ui.invoke_lfo_shape_selected(7);
        sync(&state);
        assert_eq!(ui.get_lfo_shape(), 7);
        assert_eq!(ui.get_lfo_nodes().row_count(), 8);

        // Removing the first LFO moves the custom shape down with it.
        let steps = params.custom_lfo_shape(1);
        ui.set_selected_lfo(0);
        ui.invoke_lfo_remove(0);
        sync(&state);
        assert_eq!(params.custom_lfo_shape(0), steps);
        assert_eq!(params.custom_lfo_shape(1), None);
    }

    #[test]
    fn modulator_tabs_add_edit_remove_and_render_empty() {
        use slint::ComponentHandle;
        use slint::platform::software_renderer::PremultipliedRgbaColor;

        truce_slint::platform::ensure_platform();
        let window = truce_slint::platform::create_slint_window();
        let (width, height) = (EDITOR_SIZE.0, 1500);
        window.set_size(slint::PhysicalSize::new(width, height));
        let ui = SynthUi::new().unwrap();
        let params = Arc::new(SynthParams::default());
        params.set_normalized(SynthParamsParamId::EnvCount.into(), 0.0);
        let state = editor_test_context(params.clone());
        let sync = setup_editor(state.clone(), ui.clone_strong());
        sync(&state);
        assert_eq!(ui.get_lfo_count(), 0);
        ui.show().unwrap();
        let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        }));
        let empty_pixels: Vec<_> = pixels
            .iter()
            .map(|pixel| (pixel.red, pixel.green, pixel.blue))
            .collect();

        params.set_normalized(SynthParamsParamId::OscCount.into(), 1.0 / 3.0);
        for index in 0..MAX_LFOS {
            ui.set_selected_lfo(index as i32);
            ui.invoke_lfo_add();
            sync(&state);
            assert_eq!(ui.get_current_lfo(), index as i32);
            ui.invoke_lfo_shape_selected(index as i32);
            ui.invoke_lfo_rate_changed(index as f32 / 4.0);
            ui.invoke_lfo_rate_released();
            ui.invoke_lfo_mode_selected((index % 2) as i32);
            ui.invoke_mod_assign(ModDestination::OscLevel(1).index() as i32);
            ui.invoke_mod_amount_changed(0, 0.6 + index as f32 * 0.1);
            ui.invoke_mod_amount_released(0);
            sync(&state);
            assert_eq!(ui.get_lfo_shape(), index as i32);
            assert_eq!(
                read_routes(&state, index)[0].destination,
                Some(ModDestination::OscLevel(1))
            );
        }
        ui.invoke_lfo_add();
        sync(&state);
        assert_eq!(params.lfo_count.value_usize(), MAX_LFOS);
        window.request_redraw();
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        }));
        let active_pixels: Vec<_> = pixels
            .iter()
            .map(|pixel| (pixel.red, pixel.green, pixel.blue))
            .collect();
        let pixel_at = |image: &[(u8, u8, u8)], x: usize, y: usize| image[y * width as usize + x];
        let plot_rows = |image: &[(u8, u8, u8)], left: usize, right: usize| {
            (150..500)
                .filter(|&y| {
                    (left..right)
                        // Plot background, or the LFO shape's area fill.
                        .filter(|&x| matches!(pixel_at(image, x, y), (17, 19, 25) | (25, 38, 55)))
                        .count()
                        >= 450
                })
                .collect::<Vec<_>>()
        };
        let oscillator_plot = plot_rows(&active_pixels, 30, 530);
        let lfo_plot = plot_rows(&active_pixels, 570, 1070);
        assert!(
            oscillator_plot.len() >= 140,
            "oscillator plot must be wide and tall"
        );
        assert!(lfo_plot.len() >= 140, "LFO plot must be wide and tall");
        assert_eq!(
            oscillator_plot.first(),
            lfo_plot.first(),
            "plots must align in two columns"
        );
        let plot_bottom = *oscillator_plot.last().unwrap();
        for x in [184, 280, 376, 820] {
            assert_eq!(
                pixel_at(&active_pixels, x, plot_bottom + 52),
                (41, 45, 54),
                "oscillator and LFO knobs must sit directly below their plots"
            );
        }
        assert_eq!(
            pixel_at(&active_pixels, 790, 150),
            (41, 45, 54),
            "LFO mode selector must be beside the shape selector above the plot"
        );
        assert!(plot_rows(&empty_pixels, 30, 530).len() >= 140);
        assert!(
            plot_rows(&empty_pixels, 570, 1070).is_empty(),
            "an empty Modulators column must not show an LFO plot"
        );
        for image in [&empty_pixels, &active_pixels] {
            let filter_plot_rows = (plot_bottom + 60..height as usize)
                .filter(|&y| {
                    pixel_at(image, 190, y) == (17, 19, 25)
                        && pixel_at(image, 700, y) == (23, 25, 31)
                })
                .collect::<Vec<_>>();
            assert!(
                filter_plot_rows.len() >= 140,
                "filter response plot must be tall: {} background rows, bounds {:?}..{:?}",
                filter_plot_rows.len(),
                filter_plot_rows.first(),
                filter_plot_rows.last()
            );
            assert_eq!(
                pixel_at(image, 700, filter_plot_rows[0]),
                (23, 25, 31),
                "filter response must remain narrow inside the full-width Effects group"
            );
            assert!(
                (filter_plot_rows.last().unwrap() + 150..height as usize).all(|y| pixel_at(
                    image, 700, y
                ) != (
                    17, 19, 25
                )),
                "there must be no separate output envelope below Effects"
            );
        }
        assert_ne!(active_pixels, empty_pixels);
        let untouched = read_routes(&state, 0);
        ui.invoke_mod_depth_changed(ModDestination::OscLevel(1).index() as i32, -0.25);
        ui.invoke_mod_depth_released(ModDestination::OscLevel(1).index() as i32);
        sync(&state);
        assert_eq!(read_routes(&state, 3)[0].amount, -0.25);
        assert_eq!(read_routes(&state, 0), untouched);

        ui.invoke_oscillator_remove(0);
        sync(&state);
        for index in 0..MAX_LFOS {
            assert_eq!(
                read_routes(&state, index)[0].destination,
                Some(ModDestination::OscLevel(0))
            );
        }

        let before = LFO_PARAMS.map(|ids| ids.all().map(|id| state.get_param(id)));
        ui.invoke_lfo_remove(1);
        sync(&state);
        assert_eq!(params.lfo_count.value_usize(), 3);
        assert_eq!(ui.get_current_lfo(), 2);
        for (index, expected) in [(0, before[0]), (1, before[2]), (2, before[3])] {
            assert_eq!(
                LFO_PARAMS[index].all().map(|id| state.get_param(id)),
                expected
            );
        }
        let defaults = SynthParams::default();
        for id in LFO_PARAMS[3].all() {
            assert_eq!(
                params.get_normalized(id.into()),
                defaults.get_normalized(id.into())
            );
        }
        for _ in 0..3 {
            ui.invoke_lfo_remove(0);
            sync(&state);
        }
        assert_eq!(ui.get_lfo_count(), 0);
        assert_eq!(ui.get_current_lfo(), 0);
        assert!(
            ui.get_knob_mods()
                .iter()
                .all(|modulation| !modulation.routed)
        );
        ui.set_selected_lfo(0);
        ui.invoke_lfo_add();
        sync(&state);
        assert_eq!(ui.get_lfo_shape(), 0);
        assert_eq!(ui.get_lfo_mode(), 0);
        assert_eq!(read_routes(&state, 0), [ModRoute::default(); MOD_SLOTS]);
        ui.invoke_mod_destination_selected(2, 3);
        sync(&state);
        assert_eq!(
            read_routes(&state, 0)[2].destination,
            Some(ModDestination::FilterCutoff)
        );
        ui.invoke_mod_remove(2);
        sync(&state);
        assert_eq!(read_routes(&state, 0)[2], ModRoute::default());

        params.set_normalized(SynthParamsParamId::LfoCount.into(), 1.0);
        sync(&state);
        ui.set_selected_lfo(3);
        params.set_normalized(SynthParamsParamId::LfoCount.into(), 0.0);
        sync(&state);
        assert_eq!(ui.get_lfo_count(), 0);
        assert_eq!(ui.get_current_lfo(), 0);
    }

    #[test]
    fn live_knob_dot_combines_lfos_but_depth_edits_are_selected() {
        let target = ModDestination::FilterCutoff;
        let first = routes(&[(Some(target), 0.4)]);
        let second = routes(&[(Some(target), -0.2)]);
        let mut sources = [([ModRoute::default(); MOD_SLOTS], None); MAX_LFOS];
        sources[0] = (first, Some(1.0));
        sources[1] = (second, Some(-0.5));
        let modulation =
            combined_knob_modulation(&second, [0.25; ModDestination::ALL.len()], &sources);
        let cutoff = &modulation[target.index()];
        assert!((cutoff.live - 0.75).abs() < 1e-6);
        assert_eq!(cutoff.depth, -0.2);
        assert!(cutoff.live_visible);
        assert_eq!(depth_edit(&second, target, 0.1), Some((0, 0.1)));
        sources[0].1 = None;
        sources[1].1 = None;
        assert!(
            !combined_knob_modulation(&second, [0.25; ModDestination::ALL.len()], &sources)
                [target.index()]
            .live_visible
        );
    }

    #[test]
    fn filter_mix_knob_routing_and_response_are_wired() {
        use slint::ComponentHandle;
        truce_slint::platform::ensure_platform();
        let window = truce_slint::platform::create_slint_window();
        window.set_size(slint::PhysicalSize::new(720, 1010));
        let ui = SynthUi::new().unwrap();
        let params = Arc::new(SynthParams::default());
        let state = editor_test_context(params.clone());
        let sync = setup_editor(state.clone(), ui.clone_strong());
        sync(&state);
        assert_eq!(ui.get_filter_mix(), 1.0);
        assert_eq!(ui.get_filter_mix_text(), "100 %");
        assert_eq!(filter_parameter(2), Some(SynthParamsParamId::FilterMix));
        ui.invoke_filter_changed(2, 0.25);
        ui.invoke_filter_released(2);
        sync(&state);
        assert_eq!(
            params.get_plain(SynthParamsParamId::FilterMix.into()),
            Some(25.0)
        );
        assert_eq!(ui.get_filter_mix(), 0.25);
        assert_eq!(ui.get_filter_mix_text(), "25 %");

        let path = ui.invoke_filter_response_path(0, 0.2, 0.5, 0.0, 210.0, 64.0);
        assert!(
            points(&path)
                .iter()
                .all(|(_, y)| (*y - 64.0 / 3.0).abs() < 0.01)
        );
        let wet_path = ui.invoke_filter_response_path(0, 0.2, 0.5, 1.0, 210.0, 64.0);
        assert_ne!(path, wet_path);
        ui.invoke_lfo_add();
        sync(&state);
        ui.invoke_mod_assign(ModDestination::FilterMix.index() as i32);
        sync(&state);
        assert_eq!(
            read_routes(&state, 0)[0],
            ModRoute {
                destination: Some(ModDestination::FilterMix),
                amount: 0.25,
            }
        );
        assert_eq!(ui.get_mod_slots().row_data(0).unwrap().amount_text, "+25 %");
        ui.invoke_mod_depth_changed(ModDestination::FilterMix.index() as i32, -0.1);
        ui.invoke_mod_depth_released(ModDestination::FilterMix.index() as i32);
        sync(&state);
        assert!((read_routes(&state, 0)[0].amount + 0.1).abs() < 1e-6);
        ui.show().unwrap();
        let mut pixels =
            vec![slint::platform::software_renderer::PremultipliedRgbaColor::default(); 720 * 1010];
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 720);
        }));
    }

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
        let commands = waveform_cycle_path(Waveform::Sine, 0.0, 200.0, 50.0);
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
            .map(|index| waveform_cycle_path(oscillator_from_index(index), 0.0, 100.0, 40.0))
            .collect();
        for (index, path) in paths.iter().enumerate() {
            assert!(!paths[index + 1..].contains(path));
        }
        assert_eq!(oscillator_from_index(-1), Waveform::Sine);
        assert_eq!(oscillator_from_index(99), Waveform::Sawtooth);
    }

    #[test]
    fn lfo_shape_path_spans_the_box_and_draws_jumps_vertically() {
        let saw = LfoShape::from_waveform(Waveform::Sawtooth);
        let points = points(&lfo_shape_path(&saw, 200.0, 50.0, false));
        assert_eq!(points[0], (0.0, 25.0));
        assert!((points.last().unwrap().0 - 200.0).abs() < 0.01);
        assert!(points.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        // The jump at the middle is drawn from the top to the bottom.
        let jump = points
            .windows(2)
            .find(|pair| (pair[0].0 - 100.0).abs() < 0.1 && (pair[1].0 - 100.0).abs() < 0.1)
            .expect("vertices on both sides of the jump");
        assert!(jump[0].1 < 0.1 && (jump[1].1 - 50.0).abs() < 0.1);

        let filled = lfo_shape_path(&saw, 200.0, 50.0, true);
        assert!(filled.ends_with("L200.00 25.00 L0 25.00 Z"));
    }

    #[test]
    fn lfo_presets_round_trip_through_the_dropdown() {
        assert_eq!(LFO_PRESET_NAMES.len() as i32, LFO_CUSTOM_INDEX + 1);
        for index in 0..LfoShapeType::variant_count() as i32 {
            assert_eq!(lfo_dropdown_index(index, None), index);
            // A custom shape equal to a parameter preset shows that preset.
            let shape = LfoPreset::from_index(index).unwrap().shape();
            assert_eq!(lfo_dropdown_index(0, Some(shape)), index);
        }
        let shapes: Vec<_> = (0..LFO_CUSTOM_INDEX)
            .map(|index| LfoPreset::from_index(index).unwrap().shape())
            .collect();
        for (index, shape) in shapes.iter().enumerate() {
            assert!(!shapes[index + 1..].contains(shape));
            assert_eq!(lfo_dropdown_index(0, Some(*shape)), index as i32);
        }
        assert_eq!(LfoPreset::from_index(LFO_CUSTOM_INDEX), None);
        let mut edited = shapes[0];
        edited.move_point(0, 0.3, 0.5);
        assert_eq!(lfo_dropdown_index(0, Some(edited)), LFO_CUSTOM_INDEX);
        assert_eq!(lfo_shape_from_index(-1), Waveform::Sine);
        assert_eq!(lfo_shape_from_index(99), Waveform::Sawtooth);
    }

    #[test]
    fn lfo_view_hides_handles_that_cannot_bend() {
        let square = LfoShape::from_waveform(Waveform::Square);
        let (nodes, handles) = lfo_shape_view_rows(&square);
        assert_eq!(nodes.len(), 4);
        assert!(handles.iter().all(|handle| !handle.visible));

        let triangle = LfoShape::from_waveform(Waveform::Triangle);
        let (_, handles) = lfo_shape_view_rows(&triangle);
        assert!(handles.iter().all(|handle| handle.visible));
    }

    #[test]
    fn dragging_a_curve_handle_moves_the_middle_with_the_pointer() {
        let rise = LfoShape::new(&[LfoPoint::new(0.0, -1.0), LfoPoint::new(1.0, 1.0)], false);
        for index in 0..2 {
            let shape = if index == 0 {
                rise
            } else {
                LfoShape::new(&[LfoPoint::new(0.0, 1.0), LfoPoint::new(1.0, -1.0)], false)
            };
            let middle = shape.sample(0.5);
            for amount in [-0.3, 0.3] {
                let mut bent = shape;
                bent.set_curve(0, dragged_lfo_curve(&shape, 0, 0.0, amount).unwrap());
                assert!((bent.sample(0.5) - middle) * amount > 0.0);
            }
        }
        let flat = LfoShape::new(&[LfoPoint::new(0.0, 0.5), LfoPoint::new(1.0, 0.5)], false);
        assert_eq!(dragged_lfo_curve(&flat, 0, 0.0, 0.3), None);
        assert_eq!(snap_lfo_point(0.33, 0.3, true), (0.3125, 0.25));
        assert_eq!(snap_lfo_point(0.33, 0.3, false), (0.33, 0.3));
    }

    #[test]
    fn lfo_dropdown_indices_map_to_normalized_values() {
        assert_eq!(lfo_shape_to_normalized(0), 0.0);
        assert_eq!(lfo_shape_to_normalized(3), 1.0);
        assert_eq!(lfo_mode_to_normalized(0), 0.0);
        assert_eq!(lfo_mode_to_normalized(1), 0.5);
        assert_eq!(lfo_mode_to_normalized(2), 1.0);
        assert_eq!(lfo_mode_to_normalized(5), 1.0);
    }

    #[test]
    fn lfo_markers_show_running_slots_and_highlight_the_newest() {
        let mut phases = [None; MAX_VOICES];
        phases[0] = Some(0.75);
        phases[3] = Some(0.25);
        let markers = lfo_markers(phases, Some(3));
        assert_eq!(
            markers[0],
            PositionMarker {
                position: 0.75,
                visible: true,
                newest: false
            }
        );
        assert_eq!(
            markers[3],
            PositionMarker {
                position: 0.25,
                visible: true,
                newest: true
            }
        );
        let hidden = markers.iter().filter(|marker| !marker.visible).count();
        assert_eq!(hidden, MAX_VOICES - 2);
    }

    fn response_points(mode: FilterMode, cutoff_hz: f32, q: f32) -> Vec<(f32, f32)> {
        points(&filter_response_path(
            FilterSettings {
                mode,
                cutoff_hz,
                q,
                ..Default::default()
            },
            300.0,
            72.0,
        ))
    }

    /// Plot y for a gain in dB on the 72 px high test plot (1 px per dB).
    fn y_for_db(db: f32) -> f32 {
        RESPONSE_TOP_DB - db
    }

    #[test]
    fn response_path_spans_20_hz_to_20_khz() {
        let points = response_points(FilterMode::LowPass, 1_000.0, 0.707);
        assert_eq!(points.len(), RESPONSE_PATH_SEGMENTS + 1);
        assert_eq!(points[0].0, 0.0);
        assert!((points[RESPONSE_PATH_SEGMENTS].0 - 300.0).abs() < 0.01);
        assert!(points.iter().all(|&(_, y)| (0.0..=72.0).contains(&y)));
    }

    #[test]
    fn response_path_shows_each_filter_type() {
        let low = response_points(FilterMode::LowPass, 1_000.0, 0.707);
        let high = response_points(FilterMode::HighPass, 1_000.0, 0.707);
        let band = response_points(FilterMode::BandPass, 1_000.0, 0.707);
        let last = RESPONSE_PATH_SEGMENTS;
        // Low pass is flat at 20 Hz and falls off by 20 kHz; high pass mirrors it.
        assert!((low[0].1 - y_for_db(0.0)).abs() < 0.1);
        assert!(low[last].1 > y_for_db(-30.0));
        assert!(high[0].1 > y_for_db(-30.0));
        assert!((high[last].1 - y_for_db(0.0)).abs() < 0.5);
        // Band pass is down at both ends and peaks at 0 dB at the cutoff.
        assert!(band[0].1 > y_for_db(-30.0) && band[last].1 > y_for_db(-30.0));
        let peak = band.iter().map(|&(_, y)| y).fold(f32::MAX, f32::min);
        assert!((peak - y_for_db(0.0)).abs() < 0.1);
    }

    #[test]
    fn response_path_follows_cutoff_and_q() {
        let x_of_peak =
            |points: &[(f32, f32)]| points.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        // 20 Hz to 20 kHz is ~10 octaves, so 632 Hz sits halfway across.
        let band = response_points(FilterMode::BandPass, 632.5, 4.0);
        assert!((x_of_peak(&band) - 150.0).abs() < 2.0);

        let gentle = response_points(FilterMode::LowPass, 632.5, 0.707);
        let resonant = response_points(FilterMode::LowPass, 632.5, 8.0);
        assert!((x_of_peak(&resonant) - 150.0).abs() < 2.0);
        let resonant_peak = resonant.iter().map(|&(_, y)| y).fold(f32::MAX, f32::min);
        assert!((resonant_peak - y_for_db(18.1)).abs() < 0.2);
        assert!(gentle.iter().all(|&(_, y)| y >= y_for_db(0.0) - 0.01));
    }

    #[test]
    fn filter_type_index_maps_to_each_mode() {
        assert_eq!(filter_mode_from_index(0), FilterMode::LowPass);
        assert_eq!(filter_mode_from_index(1), FilterMode::HighPass);
        assert_eq!(filter_mode_from_index(2), FilterMode::BandPass);
        assert_eq!(filter_mode_from_index(-1), FilterMode::LowPass);
        assert_eq!(filter_mode_from_index(9), FilterMode::BandPass);
        assert_eq!(filter_type_to_normalized(1), 0.5);
    }

    fn routes(list: &[(Option<ModDestination>, f32)]) -> [ModRoute; MOD_SLOTS] {
        let mut routes = [ModRoute::default(); MOD_SLOTS];
        for (route, &(destination, amount)) in routes.iter_mut().zip(list) {
            *route = ModRoute {
                destination,
                amount,
            };
        }
        routes
    }

    #[test]
    fn dropping_on_a_knob_fills_the_first_free_slot_once() {
        let cutoff = ModDestination::FilterCutoff;
        let empty = routes(&[]);
        assert_eq!(slot_for_new_route(&empty, cutoff), Some(0));
        let used = routes(&[(Some(ModDestination::OscPitch(0)), 0.1), (None, 0.3)]);
        assert_eq!(slot_for_new_route(&used, cutoff), Some(1));
        let routed = routes(&[(None, 0.0), (Some(cutoff), 0.1)]);
        assert_eq!(slot_for_new_route(&routed, cutoff), None);
        let full = routes(&[(Some(ModDestination::OscPitch(0)), 0.1); MOD_SLOTS]);
        assert_eq!(slot_for_new_route(&full, cutoff), None);
    }

    #[test]
    fn knob_depth_edits_the_first_route_and_keeps_the_rest() {
        let level = ModDestination::OscLevel(0);
        let routes = routes(&[
            (Some(ModDestination::OscPitch(0)), 0.5),
            (Some(level), 0.2),
            (Some(level), 0.3),
        ]);
        assert_eq!(depth_edit(&routes, level, 0.8), Some((1, 0.5)));
        assert_eq!(depth_edit(&routes, level, -1.0), Some((1, -1.0)));
        assert_eq!(depth_edit(&routes, ModDestination::FilterQ, 0.5), None);
    }

    #[test]
    fn knob_modulation_sums_routes_and_follows_the_lfo() {
        let cutoff = ModDestination::FilterCutoff;
        let routes = routes(&[(Some(cutoff), 0.25), (Some(cutoff), 0.25)]);
        let c = cutoff.index();
        let mut bases = [0.5; ModDestination::ALL.len()];
        bases[c] = 0.6;

        let idle = knob_modulation(&routes, bases, None);
        assert!(idle[c].routed && !idle[c].live_visible);
        assert_eq!(idle[c].depth, 0.5);
        assert!(!idle[0].routed && idle[0].depth == 0.0);

        let running = knob_modulation(&routes, bases, Some(-0.5));
        assert!(running[c].live_visible);
        assert!((running[c].live - 0.35).abs() < 1e-6);
        assert!(!running[1].live_visible);
        assert_eq!(knob_modulation(&routes, bases, Some(1.0))[c].live, 1.0);

        // Each oscillator's knobs show only their own routes.
        let level_2 = ModDestination::OscLevel(1);
        let level_routes = super::tests::routes(&[(Some(level_2), 0.25)]);
        let modulation = knob_modulation(&level_routes, bases, None);
        assert!(modulation[level_2.index()].routed);
        assert!(!modulation[ModDestination::OscLevel(0).index()].routed);
    }

    #[test]
    fn mod_amounts_read_in_destination_units() {
        let pitch = default_mod_amount(ModDestination::OscPitch(0));
        assert_eq!(
            format_mod_amount(ModDestination::OscPitch(0), pitch),
            "+1.00 st"
        );
        assert_eq!(
            format_mod_amount(ModDestination::OscLevel(0), -0.25),
            "-25 %"
        );
        let octave = default_mod_amount(ModDestination::FilterCutoff);
        assert_eq!(
            format_mod_amount(ModDestination::FilterCutoff, octave),
            "+1.00 oct"
        );
        let double = default_mod_amount(ModDestination::FilterQ);
        assert_eq!(format_mod_amount(ModDestination::FilterQ, double), "×2.00");
        assert_eq!(format_mod_amount(ModDestination::FilterQ, -double), "÷2.00");
    }

    #[test]
    fn mod_slots_show_destination_amount_and_text() {
        let slots = mod_route_slots(&routes(&[(Some(ModDestination::OscLevel(0)), -0.5)]), 1);
        assert_eq!(slots[0].destination, 2);
        assert_eq!(slots[0].amount, 0.25);
        assert_eq!(slots[0].amount_text, "-50 %");
        assert_eq!(slots[1].destination, 0);
        assert_eq!(slots[1].amount, 0.5);
        assert!(slots[1].amount_text.is_empty());
        assert_eq!(
            destination_to_normalized(Some(ModDestination::OscLevel(0))),
            2.0 / (ModDestinationType::variant_count() - 1) as f64
        );
        assert_eq!(mod_target(3), Some(ModDestination::OscLevel(1)));
        assert_eq!(mod_target(9), Some(ModDestination::FilterQ));
        assert_eq!(mod_target(10), Some(ModDestination::FilterMix));
        assert_eq!(mod_target(11), Some(ModDestination::EffectCutoff(1)));
        assert_eq!(mod_target(ModDestination::ALL.len() as i32), None);
        assert_eq!(mod_slot(-1), None);
    }

    #[test]
    fn routing_dropdown_lists_the_shown_oscillators() {
        assert_eq!(
            destination_options(1),
            [
                "None",
                "Osc 1 Pitch",
                "Osc 1 Level",
                "Filter Cutoff",
                "Filter Q",
                "Filter Mix"
            ]
        );
        let options = destination_options(3);
        assert_eq!(options.len(), 10);
        assert_eq!(options[5], "Osc 3 Pitch");
        assert_eq!(options[7], "Filter Cutoff");

        for oscillators in 1..=MAX_OSCILLATORS {
            for index in 0..destination_options(oscillators).len() as i32 {
                let destination = destination_from_option(oscillators, index);
                assert_eq!(destination_option(oscillators, destination), index);
            }
        }
        assert_eq!(destination_from_option(2, 0), None);
        assert_eq!(
            destination_from_option(2, 4),
            Some(ModDestination::OscLevel(1))
        );
        assert_eq!(
            destination_from_option(2, 5),
            Some(ModDestination::FilterCutoff)
        );
        assert_eq!(destination_from_option(2, 99), None);

        // A route to an oscillator beyond the count keeps it listed.
        let routes = routes(&[(Some(ModDestination::OscPitch(2)), 0.1)]);
        assert_eq!(shown_oscillators(1, &routes), 3);
        assert_eq!(shown_oscillators(4, &routes), 4);
        assert_eq!(shown_oscillators(0, &[]), 1);
    }

    #[test]
    fn oscillator_knobs_map_to_their_own_parameters() {
        assert_eq!(
            oscillator_parameter(0, 0),
            Some(SynthParamsParamId::Osc1Pitch)
        );
        assert_eq!(
            oscillator_parameter(1, 1),
            Some(SynthParamsParamId::Osc2Level)
        );
        assert_eq!(
            oscillator_parameter(3, 2),
            Some(SynthParamsParamId::Osc4Phase)
        );
        assert_eq!(oscillator_parameter(4, 0), None);
        assert_eq!(oscillator_parameter(-1, 0), None);
        assert_eq!(oscillator_parameter(0, 3), None);
        assert_eq!(
            destination_parameter(ModDestination::OscPitch(2)),
            u32::from(SynthParamsParamId::Osc3Pitch)
        );
        assert_eq!(oscillator_count_to_normalized(1), 0.0);
        assert_eq!(oscillator_count_to_normalized(4), 1.0);
    }

    #[test]
    fn removing_an_oscillator_shifts_the_later_ones_down() {
        let values = [[0.1], [0.2], [0.3], [0.4]];
        assert_eq!(
            oscillators_after_removal(values, [9.0], 3, 0),
            [[0.2], [0.3], [9.0], [0.4]]
        );
        assert_eq!(
            oscillators_after_removal(values, [9.0], 4, 3),
            [[0.1], [0.2], [0.3], [9.0]]
        );
        assert_eq!(
            oscillators_after_removal(values, [9.0], 4, 1),
            [[0.1], [0.3], [0.4], [9.0]]
        );
        assert_eq!(oscillators_after_removal(values, [9.0], 2, 2), values);
    }

    #[test]
    fn removing_an_oscillator_cleans_up_its_routes() {
        let routes = routes(&[
            (Some(ModDestination::OscPitch(1)), 0.5),
            (Some(ModDestination::OscLevel(2)), -0.25),
            (Some(ModDestination::OscPitch(0)), 0.1),
            (Some(ModDestination::FilterQ), 0.3),
        ]);
        let after = routes_after_removal(&routes, 1);
        assert_eq!(after[0], ModRoute::default());
        assert_eq!(
            after[1],
            ModRoute {
                destination: Some(ModDestination::OscLevel(1)),
                amount: -0.25
            }
        );
        assert_eq!(after[2], routes[2]);
        assert_eq!(after[3], routes[3]);
    }

    #[test]
    fn cycle_path_starts_at_the_start_phase() {
        let points = points(&waveform_cycle_path(Waveform::Sine, 0.25, 200.0, 50.0));
        // A sine started at 90 degrees begins at its peak and ends there.
        assert!(points[0].1.abs() < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS / 2].1 - 50.0 < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS].1.abs() < 0.01);
    }

    struct PatchHarness {
        window: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
        ui: SynthUi,
        params: Arc<SynthParams>,
        state: PluginContext<SynthParams>,
        sync: SyncFn<SynthParams>,
        keyboard: KeyboardCapture,
        automated: Arc<std::sync::Mutex<Vec<u32>>>,
        _dir: crate::patch::tests::TempDir,
    }

    impl PatchHarness {
        fn new() -> Self {
            use slint::ComponentHandle;
            use truce_slint::truce_core::editor::ClosureBridge;

            truce_slint::platform::ensure_platform();
            let window = truce_slint::platform::create_slint_window();
            window.set_size(slint::PhysicalSize::new(EDITOR_SIZE.0, EDITOR_SIZE.1));
            let ui = SynthUi::new().unwrap();
            let params = Arc::new(SynthParams::default());
            let automated = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (set_params, get_params, plain_params, format_params) = (
                params.clone(),
                params.clone(),
                params.clone(),
                params.clone(),
            );
            let log = automated.clone();
            let bridge = ClosureBridge {
                begin_edit: Box::new(|_| {}),
                set_param: Box::new(move |id, value| {
                    log.lock().unwrap().push(id);
                    set_params.set_normalized(id, value);
                }),
                end_edit: Box::new(|_| {}),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| get_params.get_normalized(id).unwrap()),
                get_param_plain: Box::new(move |id| plain_params.get_plain(id).unwrap()),
                format_param: Box::new(move |id| {
                    format_params
                        .format_value(id, format_params.get_plain(id).unwrap())
                        .unwrap()
                }),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| panic!("unexpected custom-state edit")),
                transport: Box::new(|| None),
            };
            let state = PluginContext::new(Arc::new(bridge), params.clone());
            let dir = crate::patch::tests::TempDir::new("editor-patches");
            let keyboard = KeyboardCapture::new();
            let sync = setup_editor_with(
                state.clone(),
                ui.clone_strong(),
                PatchLibrary::new(&dir.0),
                keyboard.clone(),
            );
            ui.show().unwrap();
            let harness = Self {
                window,
                ui,
                params,
                state,
                sync,
                keyboard,
                automated,
                _dir: dir,
            };
            harness.frame();
            harness
        }

        /// One editor frame: sync then render, which also instantiates
        /// conditional elements such as the dialogs.
        fn frame(&self) {
            use slint::platform::software_renderer::PremultipliedRgbaColor;
            (self.sync)(&self.state);
            let (width, height) = (EDITOR_SIZE.0 as usize, EDITOR_SIZE.1 as usize);
            let mut pixels = vec![PremultipliedRgbaColor::default(); width * height];
            self.window.request_redraw();
            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, width);
            });
            (self.sync)(&self.state);
        }

        fn key(&self, text: impl Into<slint::SharedString>) {
            use slint::ComponentHandle;
            use slint::platform::WindowEvent;
            let text = text.into();
            self.ui
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            self.ui
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        }

        fn type_text(&self, text: &str) {
            for c in text.chars() {
                self.key(c.to_string());
            }
        }

        fn set(&self, id: SynthParamsParamId, normalized: f64) {
            self.params.set_normalized(id.into(), normalized);
        }

        fn get(&self, id: SynthParamsParamId) -> f64 {
            self.params.get_normalized(id.into()).unwrap()
        }

        fn patches(&self) -> Vec<String> {
            self.ui
                .get_patches()
                .iter()
                .map(|name| name.to_string())
                .collect()
        }

        fn save_via_dialog(&self, name: &str) {
            self.ui.invoke_patch_save_requested();
            self.frame();
            self.ui.set_save_name(name.into());
            self.ui.invoke_patch_saved(name.into(), false);
            self.frame();
        }
    }

    #[test]
    fn patch_title_shows_default_and_tracks_modifications() {
        let h = PatchHarness::new();
        assert_eq!(h.ui.get_patch_name(), "Default");
        assert!(!h.ui.get_patch_modified());
        assert_eq!(h.keyboard.get(), truce_slint::KeyboardCaptureMode::None);

        h.set(SynthParamsParamId::FilterCutoff, 0.2);
        h.frame();
        assert!(h.ui.get_patch_modified());

        h.save_via_dialog("Dark");
        assert!(!h.ui.get_save_dialog_open());
        assert_eq!(h.ui.get_patch_name(), "Dark");
        assert!(!h.ui.get_patch_modified());
        assert_eq!(h.params.patch_name(), "Dark");

        h.set(SynthParamsParamId::FilterQ, 0.9);
        h.frame();
        assert!(h.ui.get_patch_modified());
    }

    #[test]
    fn patch_browser_loads_without_closing_and_escape_closes_it() {
        let h = PatchHarness::new();
        h.set(SynthParamsParamId::FilterCutoff, 0.2);
        h.set(SynthParamsParamId::OscCount, 1.0);
        h.save_via_dialog("Bright");
        let default_cutoff = {
            let fresh = SynthParams::default();
            fresh
                .get_normalized(SynthParamsParamId::FilterCutoff.into())
                .unwrap()
        };

        h.ui.invoke_patch_browser_toggled();
        h.frame();
        assert!(h.ui.get_patch_browser_open());
        assert_eq!(h.patches(), ["Default", "Bright"]);
        assert_eq!(h.ui.get_current_patch(), 1);
        assert_eq!(h.keyboard.get(), truce_slint::KeyboardCaptureMode::Escape);

        h.automated.lock().unwrap().clear();
        h.ui.invoke_patch_selected(0);
        h.frame();
        assert!(
            h.ui.get_patch_browser_open(),
            "loading keeps the browser open"
        );
        assert_eq!(h.ui.get_patch_name(), "Default");
        assert_eq!(h.ui.get_current_patch(), 0);
        assert!(!h.ui.get_patch_modified());
        assert!((h.get(SynthParamsParamId::FilterCutoff) - default_cutoff).abs() < 1e-9);
        assert_eq!(h.params.osc_count.value_usize(), 1);
        let automated = h.automated.lock().unwrap().clone();
        assert!(automated.contains(&SynthParamsParamId::FilterCutoff.into()));
        assert!(automated.contains(&SynthParamsParamId::OscCount.into()));
        assert!(
            !automated.contains(&SynthParamsParamId::Volume.into()),
            "unchanged params are left alone"
        );

        h.ui.invoke_patch_selected(1);
        h.frame();
        assert!(h.ui.get_patch_browser_open());
        assert!((h.get(SynthParamsParamId::FilterCutoff) - 0.2).abs() < 1e-6);
        assert_eq!(h.params.osc_count.value_usize(), MAX_OSCILLATORS);

        // Non-Escape keys are left alone (they go to the host in the plugin).
        h.key("a");
        h.frame();
        assert!(h.ui.get_patch_browser_open());
        h.key(slint::platform::Key::Escape);
        h.frame();
        assert!(!h.ui.get_patch_browser_open());
        assert_eq!(h.keyboard.get(), truce_slint::KeyboardCaptureMode::None);

        // The title-bar name toggles the browser.
        h.ui.invoke_patch_browser_toggled();
        h.ui.invoke_patch_browser_toggled();
        assert!(!h.ui.get_patch_browser_open());
    }

    #[test]
    fn save_dialog_accepts_typing_and_confirms_overwrites() {
        let h = PatchHarness::new();
        h.ui.invoke_patch_save_requested();
        h.frame();
        assert!(h.ui.get_save_dialog_open());
        assert_eq!(h.ui.get_save_name(), "", "Default isn't offered as a name");
        assert_eq!(h.keyboard.get(), truce_slint::KeyboardCaptureMode::All);

        h.type_text("Lead 1");
        assert_eq!(h.ui.get_save_name(), "Lead 1");
        h.key(slint::platform::Key::Return);
        h.frame();
        assert!(!h.ui.get_save_dialog_open());
        assert_eq!(h.ui.get_patch_name(), "Lead 1");

        // Saving again under the same name (any case) asks first.
        h.set(SynthParamsParamId::FilterQ, 0.8);
        h.ui.invoke_patch_save_requested();
        h.frame();
        assert_eq!(h.ui.get_save_name(), "Lead 1");
        for _ in 0.."Lead 1".len() {
            h.key(slint::platform::Key::Backspace);
        }
        h.type_text("LEAD 1");
        h.key(slint::platform::Key::Return);
        h.frame();
        assert!(h.ui.get_save_dialog_open());
        assert!(h.ui.get_save_confirm_overwrite());
        assert_eq!(h.ui.get_save_existing_name(), "Lead 1");

        // Editing the name clears the confirmation.
        h.type_text("x");
        assert!(!h.ui.get_save_confirm_overwrite());
        h.key(slint::platform::Key::Backspace);
        h.key(slint::platform::Key::Return);
        h.frame();
        assert!(h.ui.get_save_confirm_overwrite());
        h.key(slint::platform::Key::Return);
        h.frame();
        assert!(!h.ui.get_save_dialog_open());
        assert_eq!(h.ui.get_patch_name(), "LEAD 1");
        h.ui.invoke_patch_browser_toggled();
        assert_eq!(h.patches(), ["Default", "LEAD 1"]);

        // Invalid names show an error and keep the dialog open.
        h.ui.invoke_patch_save_requested();
        h.frame();
        h.ui.invoke_patch_saved("a/b".into(), false);
        assert!(h.ui.get_save_dialog_open());
        assert_ne!(h.ui.get_save_error(), "");
        h.key(slint::platform::Key::Escape);
        h.frame();
        assert!(!h.ui.get_save_dialog_open());
        assert!(
            h.ui.get_patch_browser_open(),
            "cancelling the save keeps the browser"
        );
        assert_eq!(h.keyboard.get(), truce_slint::KeyboardCaptureMode::Escape);
        h.key(slint::platform::Key::Escape);
        h.frame();
        assert!(
            !h.ui.get_patch_browser_open(),
            "Escape works again after the dialog"
        );
    }

    #[test]
    fn patch_arrows_step_with_wraparound_and_delete_removes_entries() {
        let h = PatchHarness::new();
        for (name, cutoff) in [("B", 0.3), ("a", 0.2), ("c", 0.4)] {
            h.set(SynthParamsParamId::FilterCutoff, cutoff);
            h.save_via_dialog(name);
        }
        h.ui.invoke_patch_selected(0);
        h.ui.invoke_patch_browser_toggled();
        h.frame();
        assert_eq!(h.patches(), ["Default", "a", "B", "c"]);

        let mut seen = Vec::new();
        for _ in 0..4 {
            h.ui.invoke_patch_next();
            h.frame();
            seen.push(h.ui.get_patch_name().to_string());
        }
        assert_eq!(seen, ["a", "B", "c", "Default"]);
        h.ui.invoke_patch_previous();
        h.frame();
        assert_eq!(h.ui.get_patch_name(), "c");
        assert!((h.get(SynthParamsParamId::FilterCutoff) - 0.4).abs() < 1e-6);

        h.ui.invoke_patch_deleted(2);
        h.frame();
        assert_eq!(h.patches(), ["Default", "a", "c"]);
        // Deleting the loaded patch keeps its name and sound.
        h.ui.invoke_patch_deleted(2);
        h.frame();
        assert_eq!(h.patches(), ["Default", "a"]);
        assert_eq!(h.ui.get_patch_name(), "c");
        assert_eq!(h.ui.get_current_patch(), -1);
        assert!(!h.ui.get_patch_modified());
        // The Default entry can't be deleted.
        h.ui.invoke_patch_deleted(0);
        assert_eq!(h.patches(), ["Default", "a"]);
        h.ui.invoke_patch_next();
        h.frame();
        assert_eq!(h.ui.get_patch_name(), "Default");
    }

    #[test]
    fn restored_session_patch_name_is_shown_unmodified() {
        let h = PatchHarness::new();
        h.set(SynthParamsParamId::FilterCutoff, 0.35);
        h.save_via_dialog("Session");
        h.ui.invoke_patch_selected(0);
        h.frame();

        // As if the host restored a session that had "Session" loaded.
        h.set(SynthParamsParamId::FilterCutoff, 0.35);
        h.params.set_patch_name("Session");
        h.frame();
        assert_eq!(h.ui.get_patch_name(), "Session");
        assert!(!h.ui.get_patch_modified());
    }

    /// Slint 1.15's software renderer draws dirty paths outside the dirty
    /// region, so partial repaints let the waveform bleed through the
    /// translucent dialog backdrops. The editor repaints whole frames while
    /// a dialog captures the keyboard.
    #[test]
    fn patch_dialogs_stay_opaque_over_redrawn_paths() {
        use slint::platform::software_renderer::PremultipliedRgbaColor;

        let h = PatchHarness::new();
        let (width, height) = EDITOR_SIZE;
        let mut pixels = Vec::<PremultipliedRgbaColor>::new();
        let mut rgba = Vec::new();
        h.window.set_size(slint::PhysicalSize::new(1, 1));
        h.window.set_size(slint::PhysicalSize::new(width, height));
        // Bright waveform-stroke pixels in the oscillator display.
        let mut bright_waveform = || {
            (h.sync)(&h.state);
            h.window.request_redraw();
            truce_slint::platform::render_to_rgba(
                &h.window,
                width,
                height,
                &mut pixels,
                &mut rgba,
                h.keyboard.get() != truce_slint::KeyboardCaptureMode::None,
            );
            (190..330)
                .flat_map(|y| (30..330).map(move |x| y * width as usize + x))
                .filter(|&i| pixels[i].blue > 200 && pixels[i].red < 120)
                .count()
        };

        assert!(bright_waveform() > 100);
        h.ui.invoke_patch_save_requested();
        assert_eq!(bright_waveform(), 0);
        h.ui.set_save_error("error".into());
        assert_eq!(bright_waveform(), 0);
        h.ui.set_save_confirm_overwrite(true);
        assert_eq!(bright_waveform(), 0);
        h.ui.invoke_patch_browser_toggled();
        h.key(slint::platform::Key::Escape);
        assert_eq!(bright_waveform(), 0, "the browser is still open");
        h.key(slint::platform::Key::Escape);
        assert!(bright_waveform() > 100);
    }
}
