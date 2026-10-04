use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use slint::Model;

use truce::prelude::*;
use truce_slint::{PluginContext, SlintEditor, SyncFn};

use crate::engine::modulation::MAX_PITCH_SEMITONES;
use crate::engine::node::filter::{BiquadCoefficients, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ};
use crate::engine::node::filter::{MAX_Q, MIN_Q};
use crate::engine::node::lfo::render_lfo_cycle;
use crate::engine::node::oscillator::{naive_waveform_sample, render_cycle};
use crate::engine::{
    FilterMode, FilterSettings, MAX_VOICES, MOD_SLOTS, ModDestination, ModRoute, Waveform,
};
use crate::plugin::{
    FilterType, LFO_POSITION_METERS, LfoModeType, LfoShapeType, MOD_AMOUNT_PARAMS,
    MOD_DESTINATION_PARAMS, OscillatorType, SynthParams, SynthParamsParamId, decode_lfo_newest,
    decode_lfo_position, mod_destination_from_index, mod_destination_index,
};

slint::include_modules!();

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    SlintEditor::new(
        params.clone(),
        (720, 1010),
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

            let state_for_ui = state.clone();
            ui.on_phase_changed(move |value| {
                state_for_ui
                    .params()
                    .set_normalized(SynthParamsParamId::Phase.into(), f64::from(value));
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_phase_released(move || {
                let id = SynthParamsParamId::Phase;
                enqueue_edit(
                    &pending_edits_for_ui,
                    (id, f64::from(state_for_ui.get_param(id))),
                );
            });

            let state_for_ui = state.clone();
            ui.on_osc_changed(move |id, value| {
                if let Some(parameter) = oscillator_parameter(id) {
                    state_for_ui
                        .params()
                        .set_normalized(parameter.into(), f64::from(value));
                }
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_osc_released(move |id| {
                if let Some(parameter) = oscillator_parameter(id) {
                    enqueue_edit(
                        &pending_edits_for_ui,
                        (parameter, f64::from(state_for_ui.get_param(parameter))),
                    );
                }
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
            ui.on_filter_response_path(move |index, cutoff, q, width, height| {
                let params = state_for_ui.params();
                let settings = FilterSettings {
                    mode: filter_mode_from_index(index),
                    cutoff_hz: params
                        .filter_cutoff
                        .info
                        .range
                        .denormalize(f64::from(cutoff)) as f32,
                    q: params.filter_q.info.range.denormalize(f64::from(q)) as f32,
                };
                slint::SharedString::from(filter_response_path(settings, width, height))
            });

            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_lfo_shape_selected(move |index| {
                let id = SynthParamsParamId::LfoShape;
                let normalized = lfo_shape_to_normalized(index);
                state_for_ui.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits_for_ui, (id, normalized));
            });

            let state_for_ui = state.clone();
            ui.on_lfo_rate_changed(move |value| {
                state_for_ui
                    .params()
                    .set_normalized(SynthParamsParamId::LfoRate.into(), f64::from(value));
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_lfo_rate_released(move || {
                let id = SynthParamsParamId::LfoRate;
                enqueue_edit(
                    &pending_edits_for_ui,
                    (id, f64::from(state_for_ui.get_param(id))),
                );
            });

            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_lfo_mode_selected(move |index| {
                let id = SynthParamsParamId::LfoMode;
                let normalized = lfo_mode_to_normalized(index);
                state_for_ui.params().set_normalized(id.into(), normalized);
                enqueue_edit(&pending_edits_for_ui, (id, normalized));
            });

            ui.on_lfo_cycle_path(|index, width, height| {
                let waveform = lfo_shape_from_index(index);
                slint::SharedString::from(lfo_cycle_path(waveform, width, height))
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
                move |slot: usize,
                      destination: Option<Option<ModDestination>>,
                      amount: Option<f32>| {
                    if let Some(destination) = destination {
                        let id = MOD_DESTINATION_PARAMS[slot];
                        let normalized = destination_to_normalized(destination);
                        state.params().set_normalized(id.into(), normalized);
                        enqueue_edit(&pending_edits, (id, normalized));
                    }
                    if let Some(amount) = amount {
                        let id = MOD_AMOUNT_PARAMS[slot];
                        let normalized = amount_to_normalized(amount);
                        state.params().set_normalized(id.into(), normalized);
                        enqueue_edit(&pending_edits, (id, normalized));
                    }
                }
            };

            let state_for_ui = state.clone();
            let set_slot_for_ui = set_slot.clone();
            ui.on_mod_assign(move |target| {
                let Some(destination) = mod_target(target) else {
                    return;
                };
                if let Some(slot) = slot_for_new_route(&read_routes(&state_for_ui), destination) {
                    set_slot_for_ui(
                        slot,
                        Some(Some(destination)),
                        Some(default_mod_amount(destination)),
                    );
                }
            });

            let state_for_ui = state.clone();
            let set_slot_for_ui = set_slot.clone();
            ui.on_mod_destination_selected(move |slot, index| {
                let Some(slot) = mod_slot(slot) else {
                    return;
                };
                let destination = mod_destination_from_index(u32::try_from(index).unwrap_or(0));
                let route = read_routes(&state_for_ui)[slot];
                // A freshly routed slot starts at a useful depth rather than zero.
                let amount = match destination {
                    Some(destination) if route.amount == 0.0 => {
                        Some(default_mod_amount(destination))
                    }
                    _ => None,
                };
                set_slot_for_ui(slot, Some(destination), amount);
            });

            let state_for_ui = state.clone();
            ui.on_mod_amount_changed(move |slot, value| {
                if let Some(slot) = mod_slot(slot) {
                    state_for_ui
                        .params()
                        .set_normalized(MOD_AMOUNT_PARAMS[slot].into(), f64::from(value));
                }
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_mod_amount_released(move |slot| {
                if let Some(slot) = mod_slot(slot) {
                    let id = MOD_AMOUNT_PARAMS[slot];
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
            ui.on_mod_depth_changed(move |target, depth| {
                let Some(destination) = mod_target(target) else {
                    return;
                };
                if let Some((slot, amount)) =
                    depth_edit(&read_routes(&state_for_ui), destination, depth)
                {
                    state_for_ui.params().set_normalized(
                        MOD_AMOUNT_PARAMS[slot].into(),
                        amount_to_normalized(amount),
                    );
                }
            });
            let pending_edits_for_ui = pending_edits.clone();
            let state_for_ui = state.clone();
            ui.on_mod_depth_released(move |target| {
                let Some(destination) = mod_target(target) else {
                    return;
                };
                let routes = read_routes(&state_for_ui);
                if let Some(slot) = routes
                    .iter()
                    .position(|route| route.destination == Some(destination))
                {
                    let id = MOD_AMOUNT_PARAMS[slot];
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
                ui.set_phase(state.get_param(SynthParamsParamId::Phase));
                ui.set_phase_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::Phase),
                ));
                ui.set_osc_pitch(state.get_param(SynthParamsParamId::OscPitch));
                ui.set_osc_pitch_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::OscPitch),
                ));
                ui.set_osc_level(state.get_param(SynthParamsParamId::OscLevel));
                ui.set_osc_level_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::OscLevel),
                ));
                ui.set_filter_type(state.params().filter_type.index() as i32);
                ui.set_filter_cutoff(state.get_param(SynthParamsParamId::FilterCutoff));
                ui.set_filter_cutoff_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::FilterCutoff),
                ));
                ui.set_filter_q(state.get_param(SynthParamsParamId::FilterQ));
                ui.set_filter_q_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::FilterQ),
                ));
                ui.set_lfo_shape(state.params().lfo_shape.index() as i32);
                ui.set_lfo_rate(state.get_param(SynthParamsParamId::LfoRate));
                ui.set_lfo_rate_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::LfoRate),
                ));
                ui.set_lfo_mode(state.params().lfo_mode.index() as i32);
                // Runs every frame, so the markers follow the audio thread at
                // block granularity. Rows are only touched when they change.
                let phases = LFO_POSITION_METERS.map(|id| decode_lfo_position(state.get_meter(id)));
                let newest = decode_lfo_newest(state.get_meter(SynthParamsParamId::LfoNewest));
                for (row, marker) in lfo_markers(phases, newest).into_iter().enumerate() {
                    if lfo_marker_model.row_data(row).as_ref() != Some(&marker) {
                        lfo_marker_model.set_row_data(row, marker);
                    }
                }

                let routes = read_routes(state);
                for (row, slot) in mod_route_slots(&routes).into_iter().enumerate() {
                    if mod_slot_model.row_data(row).as_ref() != Some(&slot) {
                        mod_slot_model.set_row_data(row, slot);
                    }
                }
                // Knobs follow the most recently started LFO.
                let lfo = newest
                    .and_then(|slot| phases.get(slot).copied().flatten())
                    .or_else(|| phases.iter().find_map(|phase| *phase))
                    .map(|phase| {
                        let shape = lfo_shape_from_index(state.params().lfo_shape.index() as i32);
                        naive_waveform_sample(shape, phase)
                    });
                let bases = ModDestination::ALL
                    .map(|destination| state.get_param(destination_parameter(destination)));
                for (row, modulation) in
                    knob_modulation(&routes, bases, lfo).into_iter().enumerate()
                {
                    if knob_mod_model.row_data(row).as_ref() != Some(&modulation) {
                        knob_mod_model.set_row_data(row, modulation);
                    }
                }
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

/// SVG path commands tracing one cycle of `waveform`, starting at normalized
/// `start_phase`, across a `width` by `height` box: 0 degrees after the start
/// on the left, 360 on the right, +1 at the top.
fn waveform_cycle_path(waveform: Waveform, start_phase: f32, width: f32, height: f32) -> String {
    let mut samples = [0.0_f32; CYCLE_PATH_SEGMENTS + 1];
    render_cycle(waveform, start_phase, &mut samples);
    cycle_samples_path(&samples, width, height)
}

/// SVG path commands tracing one LFO cycle of `waveform` across a `width` by
/// `height` box, laid out like `waveform_cycle_path`.
fn lfo_cycle_path(waveform: Waveform, width: f32, height: f32) -> String {
    let mut samples = [0.0_f32; CYCLE_PATH_SEGMENTS + 1];
    render_lfo_cycle(waveform, &mut samples);
    cycle_samples_path(&samples, width, height)
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

fn oscillator_parameter(id: i32) -> Option<SynthParamsParamId> {
    match id {
        0 => Some(SynthParamsParamId::OscPitch),
        1 => Some(SynthParamsParamId::OscLevel),
        _ => None,
    }
}

/// The knob parameter each modulation destination moves.
fn destination_parameter(destination: ModDestination) -> SynthParamsParamId {
    match destination {
        ModDestination::OscPitch => SynthParamsParamId::OscPitch,
        ModDestination::OscLevel => SynthParamsParamId::OscLevel,
        ModDestination::FilterCutoff => SynthParamsParamId::FilterCutoff,
        ModDestination::FilterQ => SynthParamsParamId::FilterQ,
    }
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

fn read_routes(state: &PluginContext<SynthParams>) -> [ModRoute; MOD_SLOTS] {
    std::array::from_fn(|slot| {
        let destination = state.get_param(MOD_DESTINATION_PARAMS[slot]);
        let index = (destination * ModDestination::ALL.len() as f32).round();
        let amount = state.get_param(MOD_AMOUNT_PARAMS[slot]);
        ModRoute {
            destination: mod_destination_from_index(index as u32),
            amount: (2.0 * amount - 1.0).clamp(-1.0, 1.0),
        }
    })
}

fn destination_to_normalized(destination: Option<ModDestination>) -> f64 {
    f64::from(mod_destination_index(destination)) / ModDestination::ALL.len() as f64
}

/// A bipolar route amount as its parameter's normalized value.
fn amount_to_normalized(amount: f32) -> f64 {
    ((f64::from(amount) + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Depth given to a new route: a semitone, a quarter of the level, an
/// octave of cutoff or a doubling of Q.
fn default_mod_amount(destination: ModDestination) -> f32 {
    match destination {
        ModDestination::OscPitch => 1.0 / (2.0 * MAX_PITCH_SEMITONES),
        ModDestination::OscLevel => 0.25,
        ModDestination::FilterCutoff => 1.0 / (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2(),
        ModDestination::FilterQ => 1.0 / (MAX_Q / MIN_Q).log2(),
    }
}

/// How far a route of `amount` swings `destination` at the LFO's peak, in the
/// destination's units.
fn format_mod_amount(destination: ModDestination, amount: f32) -> String {
    match destination {
        ModDestination::OscPitch => {
            format!("{:+.2} st", amount * 2.0 * MAX_PITCH_SEMITONES)
        }
        ModDestination::OscLevel => format!("{:+.0} %", amount * 100.0),
        ModDestination::FilterCutoff => {
            format!(
                "{:+.2} oct",
                amount * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).log2()
            )
        }
        ModDestination::FilterQ => {
            let ratio = (amount * (MAX_Q / MIN_Q).log2()).exp2();
            if ratio >= 1.0 {
                format!("×{ratio:.2}")
            } else {
                format!("÷{:.2}", ratio.recip())
            }
        }
    }
}

fn mod_route_slots(routes: &[ModRoute; MOD_SLOTS]) -> [ModRouteSlot; MOD_SLOTS] {
    routes.map(|route| ModRouteSlot {
        destination: mod_destination_index(route.destination) as i32,
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
fn knob_modulation(routes: &[ModRoute], bases: [f32; 4], lfo: Option<f32>) -> [KnobModulation; 4] {
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
        }
    })
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
    fn lfo_cycle_path_spans_the_box_and_follows_the_shape() {
        let points = points(&lfo_cycle_path(Waveform::Sawtooth, 200.0, 50.0));
        assert_eq!(points.len(), CYCLE_PATH_SEGMENTS + 1);
        assert_eq!(points[0], (0.0, 25.0));
        assert!((points[CYCLE_PATH_SEGMENTS].0 - 200.0).abs() < 0.01);

        let paths: Vec<_> = (0..LfoShapeType::variant_count() as i32)
            .map(|index| lfo_cycle_path(lfo_shape_from_index(index), 100.0, 40.0))
            .collect();
        for (index, path) in paths.iter().enumerate() {
            assert!(!paths[index + 1..].contains(path));
        }
        assert_eq!(lfo_shape_from_index(-1), Waveform::Sine);
        assert_eq!(lfo_shape_from_index(99), Waveform::Sawtooth);
    }

    #[test]
    fn lfo_dropdown_indices_map_to_normalized_values() {
        assert_eq!(lfo_shape_to_normalized(0), 0.0);
        assert_eq!(lfo_shape_to_normalized(3), 1.0);
        assert_eq!(lfo_mode_to_normalized(0), 0.0);
        assert_eq!(lfo_mode_to_normalized(1), 1.0);
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
            FilterSettings { mode, cutoff_hz, q },
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
        let used = routes(&[(Some(ModDestination::OscPitch), 0.1), (None, 0.3)]);
        assert_eq!(slot_for_new_route(&used, cutoff), Some(1));
        let routed = routes(&[(None, 0.0), (Some(cutoff), 0.1)]);
        assert_eq!(slot_for_new_route(&routed, cutoff), None);
        let full = routes(&[(Some(ModDestination::OscPitch), 0.1); MOD_SLOTS]);
        assert_eq!(slot_for_new_route(&full, cutoff), None);
    }

    #[test]
    fn knob_depth_edits_the_first_route_and_keeps_the_rest() {
        let level = ModDestination::OscLevel;
        let routes = routes(&[
            (Some(ModDestination::OscPitch), 0.5),
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
        let bases = [0.5, 1.0, 0.6, 0.2];

        let idle = knob_modulation(&routes, bases, None);
        assert!(idle[2].routed && !idle[2].live_visible);
        assert_eq!(idle[2].depth, 0.5);
        assert!(!idle[0].routed && idle[0].depth == 0.0);

        let running = knob_modulation(&routes, bases, Some(-0.5));
        assert!(running[2].live_visible);
        assert!((running[2].live - 0.35).abs() < 1e-6);
        assert!(!running[1].live_visible);
        assert_eq!(knob_modulation(&routes, bases, Some(1.0))[2].live, 1.0);
    }

    #[test]
    fn mod_amounts_read_in_destination_units() {
        let pitch = default_mod_amount(ModDestination::OscPitch);
        assert_eq!(
            format_mod_amount(ModDestination::OscPitch, pitch),
            "+1.00 st"
        );
        assert_eq!(format_mod_amount(ModDestination::OscLevel, -0.25), "-25 %");
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
        let slots = mod_route_slots(&routes(&[(Some(ModDestination::OscLevel), -0.5)]));
        assert_eq!(slots[0].destination, 2);
        assert_eq!(slots[0].amount, 0.25);
        assert_eq!(slots[0].amount_text, "-50 %");
        assert_eq!(slots[1].destination, 0);
        assert_eq!(slots[1].amount, 0.5);
        assert!(slots[1].amount_text.is_empty());
        assert_eq!(
            destination_to_normalized(Some(ModDestination::OscLevel)),
            0.5
        );
        assert_eq!(mod_target(3), Some(ModDestination::FilterQ));
        assert_eq!(mod_target(4), None);
        assert_eq!(mod_slot(-1), None);
    }

    #[test]
    fn cycle_path_starts_at_the_start_phase() {
        let points = points(&waveform_cycle_path(Waveform::Sine, 0.25, 200.0, 50.0));
        // A sine started at 90 degrees begins at its peak and ends there.
        assert!(points[0].1.abs() < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS / 2].1 - 50.0 < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS].1.abs() < 0.01);
    }
}
