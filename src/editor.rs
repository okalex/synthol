use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use truce::prelude::*;
use truce_slint::{PluginContext, SlintEditor, SyncFn};

use crate::engine::node::filter::{BiquadCoefficients, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ};
use crate::engine::node::oscillator::render_cycle;
use crate::engine::{FilterMode, FilterSettings, MAX_VOICES, Waveform};
use crate::plugin::{FilterType, OscillatorType, SynthParams, SynthParamsParamId};

slint::include_modules!();

pub fn create(params: Arc<SynthParams>) -> Box<dyn Editor> {
    SlintEditor::new(
        params.clone(),
        (720, 780),
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
                ui.set_filter_type(state.params().filter_type.index() as i32);
                ui.set_filter_cutoff(state.get_param(SynthParamsParamId::FilterCutoff));
                ui.set_filter_cutoff_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::FilterCutoff),
                ));
                ui.set_filter_q(state.get_param(SynthParamsParamId::FilterQ));
                ui.set_filter_q_text(slint::SharedString::from(
                    state.format_param(SynthParamsParamId::FilterQ),
                ));
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

    let mut commands = String::with_capacity(samples.len() * 16);
    for (index, sample) in samples.iter().enumerate() {
        let x = width * index as f32 / CYCLE_PATH_SEGMENTS as f32;
        let y = height * (1.0 - sample.clamp(-1.0, 1.0)) / 2.0;
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(commands, "{command}{x:.2} {y:.2} ");
    }
    commands
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
        let gain_db = 20.0 * coefficients.magnitude(frequency, RESPONSE_SAMPLE_RATE).max(1e-6).log10();
        let level = (RESPONSE_TOP_DB - gain_db) / (RESPONSE_TOP_DB - RESPONSE_BOTTOM_DB);
        let x = width * position;
        let y = height * level.clamp(0.0, 1.0);
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
        let x_of_peak = |points: &[(f32, f32)]| {
            points
                .iter()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
                .0
        };
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

    #[test]
    fn cycle_path_starts_at_the_start_phase() {
        let points = points(&waveform_cycle_path(Waveform::Sine, 0.25, 200.0, 50.0));
        // A sine started at 90 degrees begins at its peak and ends there.
        assert!(points[0].1.abs() < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS / 2].1 - 50.0 < 0.01);
        assert!(points[CYCLE_PATH_SEGMENTS].1.abs() < 0.01);
    }
}
