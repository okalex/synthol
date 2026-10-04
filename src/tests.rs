use crate::Plugin;

#[test]
fn plugin_metadata_is_valid() {
    truce_test::assert_valid_info::<Plugin>();
}

#[test]
fn editor_can_be_created() {
    truce_test::assert_has_editor::<Plugin>();
}

#[test]
fn stays_silent_without_midi() {
    use std::time::Duration;
    use truce_test::{assertions, driver};

    let result = driver!(Plugin).duration(Duration::from_millis(12)).run();
    assertions::assert_silence(&result);
}

#[test]
fn note_off_stops_the_oscillator() {
    use std::time::Duration;
    use truce_test::{assertions, driver};

    let result = driver!(Plugin)
        .duration(Duration::from_millis(1_040))
        .script(|script| {
            script.note_on(69, 1.0);
            script.wait_ms(8);
            script.note_off(69);
            script.wait_ms(1_022);
        })
        .run();

    assertions::assert_nonzero(&result);
    assertions::assert_nonzero_after(&result, Duration::from_millis(9));
    assertions::assert_silence_after(&result, Duration::from_millis(1_020));
    assertions::assert_no_nans(&result);
}

#[test]
fn attack_parameter_changes_the_rendered_envelope() {
    use std::time::Duration;
    use truce_test::driver;

    let render_with_attack = |normalized_attack| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(crate::plugin::SynthParamsParamId::Attack, normalized_attack)
            .script(|script| script.note_on(69, 1.0))
            .run()
    };

    let fast_attack = render_with_attack(0.0);
    let slow_attack = render_with_attack(1.0);
    let fast_peak = fast_attack.output[0]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    let slow_peak = slow_attack.output[0]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);

    assert!(fast_peak > 0.5, "fast attack peak was {fast_peak}");
    assert!(slow_peak < 0.02, "slow attack peak was {slow_peak}");
}

#[test]
fn decay_parameter_changes_the_rendered_envelope() {
    use std::time::Duration;
    use truce_test::driver;

    let render_with_decay = |normalized_decay| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(crate::plugin::SynthParamsParamId::Decay, normalized_decay)
            .script(|script| script.note_on(69, 1.0))
            .run()
    };

    let fast_decay = render_with_decay(0.0);
    let slow_decay = render_with_decay(1.0);
    let fast_peak = fast_decay.output[0][3_528..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    let slow_peak = slow_decay.output[0][3_528..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);

    assert!(fast_peak < 0.8, "fast decay peak was {fast_peak}");
    assert!(slow_peak > 0.9, "slow decay peak was {slow_peak}");
}

#[test]
fn sustain_parameter_changes_the_sustained_level() {
    use std::time::Duration;
    use truce_test::driver;

    let render_with_sustain = |normalized_sustain| {
        driver!(Plugin)
            .duration(Duration::from_millis(700))
            .set_param(
                crate::plugin::SynthParamsParamId::Sustain,
                normalized_sustain,
            )
            .script(|script| script.note_on(69, 1.0))
            .run()
    };

    let quiet_sustain = render_with_sustain(0.0);
    let loud_sustain = render_with_sustain(1.0);
    let quiet_peak = quiet_sustain.output[0][26_460..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    let loud_peak = loud_sustain.output[0][26_460..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);

    assert!(quiet_peak < 0.002, "quiet sustain peak was {quiet_peak}");
    assert!(loud_peak > 0.9, "loud sustain peak was {loud_peak}");
}

#[test]
fn release_parameter_changes_the_release_duration() {
    use std::time::Duration;
    use truce_test::driver;

    let render_with_release = |normalized_release| {
        driver!(Plugin)
            .duration(Duration::from_millis(300))
            .set_param(
                crate::plugin::SynthParamsParamId::Release,
                normalized_release,
            )
            .script(|script| {
                script.note_on(69, 1.0);
                script.wait_ms(100);
                script.note_off(69);
            })
            .run()
    };

    let short_release = render_with_release(0.0);
    let long_release = render_with_release(1.0);
    let short_peak = short_release.output[0][8_820..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    let long_peak = long_release.output[0][8_820..]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);

    assert!(short_peak < 0.002, "short release peak was {short_peak}");
    assert!(long_peak > 0.1, "long release peak was {long_peak}");
}

#[test]
fn envelope_times_reach_zero_milliseconds() {
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    let params = SynthParams::default();
    for id in [
        SynthParamsParamId::Attack,
        SynthParamsParamId::Decay,
        SynthParamsParamId::Release,
    ] {
        params.set_normalized(id.into(), 0.0);
        assert_eq!(params.get_plain(id.into()), Some(0.0));
    }
}

#[test]
fn voices_parameter_spans_one_to_eight() {
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    let params = SynthParams::default();
    assert_eq!(params.voices.value(), 8);
    for (normalized, voices) in [(0.0, 1.0), (3.0 / 7.0, 4.0), (1.0, 8.0)] {
        params.set_normalized(SynthParamsParamId::Voices.into(), normalized);
        assert_eq!(
            params.get_plain(SynthParamsParamId::Voices.into()),
            Some(voices)
        );
    }
}

#[test]
fn voices_parameter_controls_polyphony() {
    use std::time::Duration;
    use truce_test::driver;

    let render_chord = |normalized_voices| {
        driver!(Plugin)
            .duration(Duration::from_millis(50))
            .set_param(crate::plugin::SynthParamsParamId::Voices, normalized_voices)
            .script(|script| {
                script.note_on(60, 1.0);
                script.note_on(64, 1.0);
                script.note_on(67, 1.0);
            })
            .run()
    };

    let peak = |result: &truce_test::DriverResult<Plugin>| {
        result.output[0]
            .iter()
            .copied()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    };
    let mono_peak = peak(&render_chord(0.0));
    let poly_peak = peak(&render_chord(1.0));

    assert!(mono_peak <= 1.0, "mono peak was {mono_peak}");
    assert!(poly_peak > 1.5, "poly peak was {poly_peak}");
}

#[test]
fn oscillator_parameter_lists_each_waveform() {
    use crate::plugin::{OscillatorType, SynthParams};
    use truce::prelude::*;

    let params = SynthParams::default();
    assert_eq!(params.oscillator.index(), 0);
    assert_eq!(
        OscillatorType::variant_names(),
        ["Sine", "Square", "Triangle", "Sawtooth"]
    );

    for index in 0..4 {
        params.set_normalized(
            crate::plugin::SynthParamsParamId::Oscillator.into(),
            index as f64 / 3.0,
        );
        assert_eq!(params.oscillator.index(), index);
    }
}

#[test]
fn oscillator_parameter_changes_rendered_waveform() {
    use std::time::Duration;
    use truce_test::driver;

    let render = |normalized_waveform| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(
                crate::plugin::SynthParamsParamId::Oscillator,
                normalized_waveform,
            )
            .set_param(crate::plugin::SynthParamsParamId::Attack, 0.0)
            .script(|script| script.note_on(57, 1.0))
            .run()
    };

    // Crest factor (peak / RMS): ~1.0 square, ~1.41 sine, ~1.73 triangle and
    // sawtooth.
    let crest = |result: &truce_test::DriverResult<Plugin>| {
        let tail = &result.output[0][result.output[0].len() / 2..];
        let peak = tail.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        peak / rms
    };
    let sine = crest(&render(0.0));
    let square = crest(&render(1.0 / 3.0));
    let triangle = crest(&render(2.0 / 3.0));
    let sawtooth = render(1.0);

    assert!(square < 1.2, "square crest was {square}");
    assert!((1.3..1.55).contains(&sine), "sine crest was {sine}");
    assert!(triangle > 1.6, "triangle crest was {triangle}");
    let sawtooth_crest = crest(&sawtooth);
    assert!(sawtooth_crest > 1.6, "sawtooth crest was {sawtooth_crest}");
    // Unlike the symmetric triangle, the sawtooth has a single rising ramp:
    // most consecutive samples increase.
    let tail = &sawtooth.output[0][sawtooth.output[0].len() / 2..];
    let rising = tail.windows(2).filter(|w| w[1] > w[0]).count();
    assert!(
        rising * 10 > tail.len() * 9,
        "sawtooth rose on {rising} samples"
    );
}

#[test]
fn phase_parameter_sets_where_notes_start() {
    use std::time::Duration;
    use truce_test::driver;

    let first_sample = |normalized_phase| {
        let result = driver!(Plugin)
            .duration(Duration::from_millis(10))
            .set_param(crate::plugin::SynthParamsParamId::Phase, normalized_phase)
            .set_param(crate::plugin::SynthParamsParamId::Attack, 0.0)
            .script(|script| script.note_on(57, 1.0))
            .run();
        let output = &result.output[0];
        let start = output
            .iter()
            .position(|sample| *sample != 0.0)
            .expect("the note should sound");
        output[start]
    };

    // A sine starting at 0 degrees rises from zero; at 90 degrees it starts
    // at its peak and at 270 degrees at its trough.
    let at_0 = first_sample(0.0);
    let at_90 = first_sample(0.25);
    let at_270 = first_sample(0.75);
    assert!(at_0.abs() < 0.05, "0 degrees started at {at_0}");
    assert!(at_90 > 0.5, "90 degrees started at {at_90}");
    assert!(at_270 < -0.5, "270 degrees started at {at_270}");
}

#[test]
fn filter_type_parameter_lists_each_mode() {
    use crate::plugin::{FilterType, SynthParams};
    use truce::prelude::*;

    let params = SynthParams::default();
    assert_eq!(params.filter_type.index(), 0);
    assert_eq!(
        FilterType::variant_names(),
        ["Low Pass", "High Pass", "Band Pass"]
    );
    for index in 0..3 {
        params.set_normalized(
            crate::plugin::SynthParamsParamId::FilterType.into(),
            index as f64 / 2.0,
        );
        assert_eq!(params.filter_type.index(), index);
    }
}

#[test]
fn filter_parameters_shape_the_oscillator_output() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    // A 220 Hz sawtooth through the filter; returns the steady-state RMS.
    let rms = |filter_type: f64, cutoff: f64| {
        let result = driver!(Plugin)
            .duration(Duration::from_millis(200))
            .set_param(SynthParamsParamId::Oscillator, 1.0)
            .set_param(SynthParamsParamId::Attack, 0.0)
            .set_param(SynthParamsParamId::FilterType, filter_type)
            .set_param(SynthParamsParamId::FilterCutoff, cutoff)
            .script(|script| script.note_on(57, 1.0))
            .run();
        let tail = &result.output[0][result.output[0].len() / 2..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    };

    let open = rms(0.0, 1.0);
    assert!(open > 0.1, "open filter RMS was {open}");
    // Low pass at 20 Hz and high pass at 20 kHz both remove almost everything.
    let low_passed = rms(0.0, 0.0);
    let high_passed = rms(0.5, 1.0);
    assert!(low_passed < open * 0.05, "low pass RMS was {low_passed}");
    assert!(high_passed < open * 0.1, "high pass RMS was {high_passed}");
    // Band pass around the fundamental keeps it but drops the harmonics.
    let band_passed = rms(1.0, (220.0_f64 / 20.0).ln() / 1_000.0_f64.ln());
    assert!(
        (open * 0.3..open * 0.95).contains(&band_passed),
        "band pass RMS was {band_passed} of {open}"
    );
}

#[test]
fn lfo_shape_and_mode_parameters_list_their_options() {
    use crate::plugin::{LfoModeType, LfoShapeType, SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    let params = SynthParams::default();
    assert_eq!(params.lfo_shape.index(), 0);
    assert_eq!(
        LfoShapeType::variant_names(),
        ["Sine", "Square", "Triangle", "Sawtooth"]
    );
    for index in 0..4 {
        params.set_normalized(SynthParamsParamId::LfoShape.into(), index as f64 / 3.0);
        assert_eq!(params.lfo_shape.index(), index);
    }

    assert_eq!(params.lfo_mode.index(), 0);
    assert_eq!(LfoModeType::variant_names(), ["Trigger", "Sync"]);
    params.set_normalized(SynthParamsParamId::LfoMode.into(), 1.0);
    assert_eq!(params.lfo_mode.index(), 1);
}

#[test]
fn lfo_rate_spans_a_hundredth_of_a_hertz_to_thirty_hertz() {
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    let params = SynthParams::default();
    let id: u32 = SynthParamsParamId::LfoRate.into();
    assert_eq!(params.get_plain(id), Some(1.0));
    params.set_normalized(id, 0.0);
    assert!((params.get_plain(id).unwrap() - 0.01).abs() < 1e-9);
    params.set_normalized(id, 1.0);
    assert!((params.get_plain(id).unwrap() - 30.0).abs() < 1e-6);

    for (value, text) in [(0.01, "0.01 Hz"), (2.5, "2.5 Hz"), (30.0, "30 Hz")] {
        assert_eq!(params.format_value(id, value).as_deref(), Some(text));
    }
}

#[test]
fn an_unrouted_lfo_does_not_affect_the_sound() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    let render = |shape: f64, rate: f64, mode: f64| {
        driver!(Plugin)
            .duration(Duration::from_millis(50))
            .set_param(SynthParamsParamId::LfoShape, shape)
            .set_param(SynthParamsParamId::LfoRate, rate)
            .set_param(SynthParamsParamId::LfoMode, mode)
            .script(|script| script.note_on(57, 1.0))
            .run()
            .output[0]
            .clone()
    };

    let reference = render(0.0, 0.5, 0.0);
    assert_eq!(render(1.0, 1.0, 0.0), reference);
    assert_eq!(render(1.0 / 3.0, 0.0, 1.0), reference);
}

#[test]
fn lfo_meters_round_trip_positions_and_newest_slot() {
    use crate::plugin::{
        decode_lfo_newest, decode_lfo_position, encode_lfo_newest, encode_lfo_position,
    };

    // A meter that was never written reads 0.0, which must mean "stopped".
    assert_eq!(decode_lfo_position(0.0), None);
    assert_eq!(decode_lfo_newest(0.0), None);
    assert_eq!(decode_lfo_position(encode_lfo_position(None)), None);
    assert_eq!(decode_lfo_newest(encode_lfo_newest(None)), None);

    for phase in [0.0, 0.25, 0.999] {
        let decoded = decode_lfo_position(encode_lfo_position(Some(phase))).unwrap();
        assert!((decoded - phase).abs() < 1.0e-6);
    }
    for slot in 0..crate::engine::MAX_VOICES {
        assert_eq!(decode_lfo_newest(encode_lfo_newest(Some(slot))), Some(slot));
    }
}

#[test]
fn modulation_ranges_match_the_destination_parameters() {
    use crate::engine::ModDestination;
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    // Route depths are fractions of each knob's range, so the engine's
    // mapping must agree with the parameter's.
    let params = SynthParams::default();
    let destinations = [
        (ModDestination::OscPitch, SynthParamsParamId::OscPitch, 1.0),
        (
            ModDestination::OscLevel,
            SynthParamsParamId::OscLevel,
            100.0,
        ),
        (
            ModDestination::FilterCutoff,
            SynthParamsParamId::FilterCutoff,
            1.0,
        ),
        (ModDestination::FilterQ, SynthParamsParamId::FilterQ, 1.0),
    ];
    for (destination, id, scale) in destinations {
        let id: u32 = id.into();
        for normalized in [0.0, 0.3, 0.5, 0.8, 1.0] {
            params.set_normalized(id, normalized);
            let plain = params.get_plain(id).unwrap() / scale;
            let engine = f64::from(destination.denormalize(normalized as f32));
            assert!(
                (plain - engine).abs() <= plain.abs() * 1e-4 + 1e-4,
                "{destination:?} at {normalized}: param {plain}, engine {engine}"
            );
        }
    }
}

#[test]
fn oscillator_pitch_and_level_parameters() {
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::{assertions, driver};

    let params = SynthParams::default();
    let pitch: u32 = SynthParamsParamId::OscPitch.into();
    let level: u32 = SynthParamsParamId::OscLevel.into();
    assert_eq!(params.get_plain(pitch), Some(0.0));
    assert_eq!(params.get_plain(level), Some(100.0));
    assert_eq!(params.format_value(pitch, 0.0).as_deref(), Some("+0.00 st"));
    assert_eq!(
        params.format_value(pitch, -7.0).as_deref(),
        Some("-7.00 st")
    );
    assert_eq!(params.format_value(level, 100.0).as_deref(), Some("100 %"));
    let amount: u32 = SynthParamsParamId::Mod1Amount.into();
    assert_eq!(params.format_value(amount, -40.0).as_deref(), Some("-40 %"));

    let muted = driver!(Plugin)
        .duration(Duration::from_millis(50))
        .set_param(SynthParamsParamId::OscLevel, 0.0)
        .script(|script| script.note_on(57, 1.0))
        .run();
    assertions::assert_silence(&muted);
}

#[test]
fn routing_slots_list_each_destination() {
    use crate::engine::ModDestination;
    use crate::plugin::{
        MOD_AMOUNT_PARAMS, MOD_DESTINATION_PARAMS, ModDestinationType, SynthParams,
        mod_destination_from_index, mod_destination_index,
    };
    use truce::prelude::*;

    assert_eq!(
        ModDestinationType::variant_names(),
        [
            "None",
            "Osc Pitch",
            "Osc Level",
            "Filter Cutoff",
            "Filter Q"
        ]
    );
    assert_eq!(mod_destination_from_index(0), None);
    for destination in ModDestination::ALL {
        let index = mod_destination_index(Some(destination));
        assert_eq!(mod_destination_from_index(index), Some(destination));
    }

    let params = SynthParams::default();
    for (destination, amount) in MOD_DESTINATION_PARAMS.into_iter().zip(MOD_AMOUNT_PARAMS) {
        assert_eq!(params.get_plain(destination.into()), Some(0.0));
        assert_eq!(params.get_plain(amount.into()), Some(0.0));
        params.set_normalized(amount.into(), 0.0);
        assert_eq!(params.get_plain(amount.into()), Some(-100.0));
    }
}

#[test]
fn a_routed_lfo_modulates_the_sound() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    // A 20 Hz LFO on the oscillator level.
    let render = |destination: f64, amount: f64| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(SynthParamsParamId::Attack, 0.0)
            .set_param(SynthParamsParamId::LfoRate, 0.9)
            .set_param(SynthParamsParamId::Mod2Destination, destination)
            .set_param(SynthParamsParamId::Mod2Amount, amount)
            .script(|script| script.note_on(57, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let osc_level = 0.5;
    // Amounts are normalized: 0.5 is 0 %, 0.0 is -100 %.
    let reference = render(0.0, 0.5);
    // A routed slot with no depth, or depth with no destination, is inert.
    assert_eq!(render(osc_level, 0.5), reference);
    assert_eq!(render(0.0, 0.0), reference);

    let modulated = render(osc_level, 0.0);
    assert_ne!(modulated, reference);
    let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
    assert!(energy(&modulated) < energy(&reference) * 0.8);
}
