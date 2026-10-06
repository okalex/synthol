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
fn unison_parameters_have_independent_ranges_defaults_and_saved_values() {
    use crate::plugin::{OSCILLATOR_PARAMS, SynthParams};
    use truce::prelude::*;

    let params = SynthParams::default();
    for (index, ids) in OSCILLATOR_PARAMS.iter().enumerate() {
        assert_eq!(
            params
                .format_value(ids.unison_detune.into(), 50.0)
                .as_deref(),
            Some("50 cents")
        );
        for (id, default, maximum) in [
            (ids.unison_voices, 1.0, 20.0),
            (ids.unison_detune, 0.0, 50.0),
            (ids.unison_width, 0.0, 100.0),
        ] {
            assert_eq!(params.get_plain(id.into()), Some(default));
            params.set_normalized(id.into(), 1.0);
            assert_eq!(params.get_plain(id.into()), Some(maximum));
            params.set_normalized(id.into(), index as f64 / 3.0);
        }
    }
    let (ids, values) = params.collect_values();
    let restored = SynthParams::default();
    restored.restore_values(&ids.into_iter().zip(values).collect::<Vec<_>>());
    for ids in OSCILLATOR_PARAMS {
        for id in [ids.unison_voices, ids.unison_detune, ids.unison_width] {
            assert_eq!(restored.get_plain(id.into()), params.get_plain(id.into()));
        }
    }
}

#[test]
fn each_oscillator_unison_reaches_stereo_output_and_stops_on_note_off() {
    use crate::plugin::{OSCILLATOR_PARAMS, SynthParamsParamId};
    use std::time::Duration;
    use truce_test::driver;

    for (index, ids) in OSCILLATOR_PARAMS.iter().enumerate() {
        for voices in [2, 3, 20] {
            let render = |width, detune, mix| {
                let mut driver = driver!(Plugin)
                    .duration(Duration::from_millis(100))
                    .set_param(SynthParamsParamId::EnvCount, 0.0)
                    .set_param(SynthParamsParamId::OscCount, index as f64 / 3.0)
                    .set_param(SynthParamsParamId::FilterMix, mix)
                    .set_param(ids.unison_voices, (voices - 1) as f64 / 19.0)
                    .set_param(ids.unison_detune, detune)
                    .set_param(ids.unison_width, width);
                for (other, params) in OSCILLATOR_PARAMS.iter().enumerate() {
                    driver = driver.set_param(params.level, if other == index { 1.0 } else { 0.0 });
                }
                driver
                    .script(|script| {
                        script.note_on(69, 1.0);
                        script.wait_ms(70);
                        script.note_off(69);
                    })
                    .run()
            };
            let centered = render(0.0, 1.0, 0.0);
            assert_eq!(centered.output[0], centered.output[1]);
            let wide = render(1.0, 1.0, 0.0);
            assert!(
                wide.output[0]
                    .iter()
                    .zip(&wide.output[1])
                    .any(|(left, right)| (left - right).abs() > 0.1)
            );
            let boundary = (wide.sample_rate * 0.07) as usize;
            for channel in &wide.output {
                assert!(channel[boundary..].iter().all(|&sample| sample == 0.0));
                assert!(
                    channel
                        .iter()
                        .all(|sample| sample.is_finite() && sample.abs() < 1.00001)
                );
            }
            let no_detune = render(1.0, 0.0, 0.0);
            for (left, right) in no_detune.output[0].iter().zip(&no_detune.output[1]) {
                assert!((left - right).abs() < 1e-6);
            }
            let filtered = render(1.0, 1.0, 1.0);
            assert!(
                filtered.output[0]
                    .iter()
                    .zip(&filtered.output[1])
                    .any(|(left, right)| (left - right).abs() > 0.1),
                "filters must not collapse stereo"
            );
        }
    }
}

#[test]
fn unison_detune_uses_cents_at_the_host_output() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    for cents in [10.0, 50.0] {
        let result = driver!(Plugin)
            .duration(Duration::from_millis(1200))
            .set_param(SynthParamsParamId::EnvCount, 0.0)
            .set_param(SynthParamsParamId::FilterMix, 0.0)
            .set_param(SynthParamsParamId::Osc1UnisonVoices, 1.0 / 19.0)
            .set_param(SynthParamsParamId::Osc1UnisonDetune, cents / 50.0)
            .set_param(SynthParamsParamId::Osc1UnisonWidth, 1.0)
            .script(|script| script.note_on(69, 1.0))
            .run();
        let start = (result.sample_rate * 0.2) as usize;
        for (channel, sign) in [(0, -1.0), (1, 1.0)] {
            let cycles = result.output[channel][start..]
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let expected_hz = 440.0 * 2.0_f64.powf(sign * cents / 1200.0);
            assert!(
                (cycles as f64 - expected_hz).abs() <= 1.0,
                "{cents} cents, channel {channel}: {cycles} Hz vs {expected_hz} Hz"
            );
        }
    }
}

#[test]
fn oscillator_pan_parameters_default_centered_and_move_stereo_output() {
    use crate::plugin::{OSCILLATOR_PARAMS, SynthParams, SynthParamsParamId};
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::driver;

    let params = SynthParams::default();
    let pan = OSCILLATOR_PARAMS[0].pan;
    assert_eq!(params.get_plain(pan.into()), Some(0.0));
    assert_eq!(params.format_value(pan.into(), 0.0).as_deref(), Some("C"));
    assert_eq!(
        params.format_value(pan.into(), -50.0).as_deref(),
        Some("L 50 %")
    );
    assert_eq!(
        params.format_value(pan.into(), 100.0).as_deref(),
        Some("R 100 %")
    );
    params.set_normalized(pan.into(), 0.0);
    assert_eq!(params.get_plain(pan.into()), Some(-100.0));
    params.set_normalized(pan.into(), 1.0);
    assert_eq!(params.get_plain(pan.into()), Some(100.0));

    for (index, ids) in OSCILLATOR_PARAMS.iter().enumerate() {
        let render = |pan| {
            let mut driver = driver!(Plugin)
                .duration(Duration::from_millis(100))
                .set_param(SynthParamsParamId::EnvCount, 0.0)
                .set_param(SynthParamsParamId::OscCount, index as f64 / 3.0)
                .set_param(SynthParamsParamId::FilterMix, 0.0)
                .set_param(ids.pan, pan);
            for (other, params) in OSCILLATOR_PARAMS.iter().enumerate() {
                driver = driver.set_param(params.level, if other == index { 1.0 } else { 0.0 });
            }
            driver.script(|script| script.note_on(69, 1.0)).run()
        };
        let centered = render(0.5);
        assert_eq!(centered.output[0], centered.output[1]);
        let left = render(0.0);
        assert!(
            left.output[0]
                .iter()
                .zip(&left.output[1])
                .any(|(left, right)| left.abs() > right.abs() + 0.1)
        );
        let right = render(1.0);
        assert!(
            right.output[1]
                .iter()
                .zip(&right.output[0])
                .any(|(right, left)| right.abs() > left.abs() + 0.1)
        );
    }
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
fn mono_unison_output_is_the_average_of_stereo_after_filtering() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    let render = |channels| {
        driver!(Plugin)
            .channels(channels)
            .duration(Duration::from_millis(100))
            .set_param(SynthParamsParamId::Osc1UnisonVoices, 1.0)
            .set_param(SynthParamsParamId::Osc1UnisonDetune, 1.0)
            .set_param(SynthParamsParamId::Osc1UnisonWidth, 1.0)
            .set_param(SynthParamsParamId::FilterCutoff, 0.4)
            .script(|script| script.note_on(69, 1.0))
            .run()
    };
    let mono = render(1);
    let stereo = render(2);
    assert_eq!(mono.output.len(), 1);
    assert_eq!(mono.output[0].len(), stereo.output[0].len());
    for ((mono, left), right) in mono.output[0]
        .iter()
        .zip(&stereo.output[0])
        .zip(&stereo.output[1])
    {
        assert_eq!(*mono, (left + right) * 0.5);
    }
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
    assert_eq!(params.osc_1_type.index(), 0);
    assert_eq!(
        OscillatorType::variant_names(),
        ["Sine", "Square", "Triangle", "Sawtooth"]
    );

    for index in 0..4 {
        params.set_normalized(
            crate::plugin::SynthParamsParamId::Osc1Type.into(),
            index as f64 / 3.0,
        );
        assert_eq!(params.osc_1_type.index(), index);
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
                crate::plugin::SynthParamsParamId::Osc1Type,
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
            .set_param(
                crate::plugin::SynthParamsParamId::Osc1Phase,
                normalized_phase,
            )
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
            .set_param(SynthParamsParamId::Osc1Type, 1.0)
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
    assert_eq!(
        LfoModeType::variant_names(),
        ["Trigger", "Envelope", "Sync"]
    );
    params.set_normalized(SynthParamsParamId::LfoMode.into(), 1.0);
    assert_eq!(params.lfo_mode.index(), 2);
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
        (
            ModDestination::OscPitch(0),
            SynthParamsParamId::Osc1Pitch,
            1.0,
        ),
        (
            ModDestination::OscLevel(0),
            SynthParamsParamId::Osc1Level,
            100.0,
        ),
        (
            ModDestination::OscPitch(3),
            SynthParamsParamId::Osc4Pitch,
            1.0,
        ),
        (
            ModDestination::OscLevel(1),
            SynthParamsParamId::Osc2Level,
            100.0,
        ),
        (
            ModDestination::FilterCutoff,
            SynthParamsParamId::FilterCutoff,
            1.0,
        ),
        (ModDestination::FilterQ, SynthParamsParamId::FilterQ, 1.0),
        (
            ModDestination::FilterMix,
            SynthParamsParamId::FilterMix,
            100.0,
        ),
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
    let pitch: u32 = SynthParamsParamId::Osc1Pitch.into();
    let level: u32 = SynthParamsParamId::Osc1Level.into();
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
        .set_param(SynthParamsParamId::Osc1Level, 0.0)
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
            "Osc 1 Pitch",
            "Osc 1 Level",
            "Filter Cutoff",
            "Filter Q",
            "Osc 2 Pitch",
            "Osc 2 Level",
            "Osc 3 Pitch",
            "Osc 3 Level",
            "Osc 4 Pitch",
            "Osc 4 Level",
            "Filter Mix",
            "Osc 1 Shape",
            "Osc 2 Shape",
            "Osc 3 Shape",
            "Osc 4 Shape",
        ]
    );
    assert_eq!(mod_destination_from_index(0), None);
    // Saved routes from before multiple oscillators keep their meaning.
    assert_eq!(
        mod_destination_from_index(2),
        Some(ModDestination::OscLevel(0))
    );
    assert_eq!(mod_destination_from_index(4), Some(ModDestination::FilterQ));
    for destination in ModDestination::ALL
        .into_iter()
        .filter(|destination| destination.effect().is_none_or(|slot| slot == 0))
    {
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
            .set_param(SynthParamsParamId::LfoCount, 0.25)
            .set_param(SynthParamsParamId::LfoRate, 0.9)
            .set_param(SynthParamsParamId::Mod2Destination, destination)
            .set_param(SynthParamsParamId::Mod2Amount, amount)
            .script(|script| script.note_on(57, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let osc_level = 2.0 / 15.0;
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

#[test]
fn every_lfo_has_independent_host_parameters_and_routes() {
    use crate::engine::{MAX_LFOS, ModDestination};
    use crate::plugin::{
        LFO_PARAMS, ModDestinationType, SynthParams, SynthParamsParamId, mod_destination_index,
    };
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::driver;

    let params = SynthParams::default();
    assert_eq!(params.lfo_count.value_usize(), 0);
    for count in 0..=MAX_LFOS {
        params.set_normalized(
            SynthParamsParamId::LfoCount.into(),
            count as f64 / MAX_LFOS as f64,
        );
        assert_eq!(params.lfo_count.value_usize(), count);
    }
    let destination = f64::from(mod_destination_index(Some(ModDestination::OscLevel(0))))
        / (ModDestinationType::variant_count() - 1) as f64;
    for (index, ids) in LFO_PARAMS.iter().enumerate() {
        assert_eq!(params.get_plain(ids.rate.into()), Some(1.0));
        let render = |count, route_lfo: usize| {
            driver!(Plugin)
                .duration(Duration::from_millis(100))
                .set_param(SynthParamsParamId::Attack, 0.0)
                .set_param(SynthParamsParamId::LfoCount, count)
                .set_param(LFO_PARAMS[route_lfo].rate, 0.9)
                .set_param(LFO_PARAMS[route_lfo].destinations[0], destination)
                .set_param(LFO_PARAMS[route_lfo].amounts[0], 0.0)
                .script(|script| script.note_on(57, 1.0))
                .run()
                .output[0]
                .clone()
        };
        let reference = render(0.0, index);
        let active = render((index + 1) as f64 / MAX_LFOS as f64, index);
        let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
        assert!(
            energy(&active) < energy(&reference) * 0.8,
            "LFO {}",
            index + 1
        );
        if index > 0 {
            assert_eq!(render(index as f64 / MAX_LFOS as f64, index), reference);
        }
    }
}

#[test]
fn dynamic_lfos_round_trip_saved_parameter_values() {
    use crate::plugin::{LFO_PARAMS, SynthParams, SynthParamsParamId};
    use truce::prelude::*;

    let params = SynthParams::default();
    for (index, ids) in LFO_PARAMS.iter().enumerate() {
        params.set_normalized(ids.shape.into(), index as f64 / 3.0);
        params.set_normalized(ids.rate.into(), 0.2 * index as f64);
        params.set_normalized(ids.mode.into(), (index % 2) as f64);
        for slot in 0..crate::engine::MOD_SLOTS {
            params.set_normalized(ids.destinations[slot].into(), (slot + 1) as f64 / 15.0);
            params.set_normalized(ids.amounts[slot].into(), 0.1 * (index + slot) as f64);
        }
    }
    for count in 0..=crate::engine::MAX_LFOS {
        params.set_normalized(SynthParamsParamId::LfoCount.into(), count as f64 / 4.0);
        let (ids, values) = params.collect_values();
        let stored: Vec<_> = ids.into_iter().zip(values).collect();
        let restored = SynthParams::default();
        restored.restore_values(&stored);
        assert_eq!(restored.lfo_count.value_usize(), count);
        for ids in LFO_PARAMS {
            for id in ids.all() {
                assert_eq!(restored.get_plain(id.into()), params.get_plain(id.into()));
            }
        }
    }
    for (id, expected) in [
        (SynthParamsParamId::Osc1Type, 8_919_309),
        (SynthParamsParamId::LfoShape, 9_677_462),
        (SynthParamsParamId::Mod1Destination, 10_410_336),
        (SynthParamsParamId::OscCount, 4_815_296),
        (SynthParamsParamId::Osc4Level, 7_313_262),
    ] {
        assert_eq!(
            u32::from(id),
            expected,
            "existing parameter IDs must stay stable"
        );
    }
}

#[test]
fn envelopes_round_trip_saved_parameter_values_and_default_to_oscillator_one() {
    use crate::engine::{MAX_ENVELOPES, ModDestination};
    use crate::plugin::{ENV_PARAMS, SynthParams, SynthParamsParamId, mod_destination_index};
    use truce::prelude::*;

    let params = SynthParams::default();
    assert_eq!(params.env_count.value_usize(), 1);
    assert_eq!(
        params.get_plain(ENV_PARAMS[0].destinations[0].into()),
        Some(f64::from(mod_destination_index(Some(
            ModDestination::OscLevel(0)
        ))))
    );
    assert_eq!(
        params.get_plain(ENV_PARAMS[0].amounts[0].into()),
        Some(100.0)
    );
    assert_eq!(
        ENV_PARAMS[0].adsr,
        [
            SynthParamsParamId::Attack,
            SynthParamsParamId::Decay,
            SynthParamsParamId::Sustain,
            SynthParamsParamId::Release
        ]
    );
    for (index, ids) in ENV_PARAMS.iter().enumerate() {
        for (offset, id) in ids.all().into_iter().enumerate() {
            params.set_normalized(id.into(), ((index + offset) % 10) as f64 / 10.0);
        }
    }
    for count in 0..=MAX_ENVELOPES {
        params.set_normalized(
            SynthParamsParamId::EnvCount.into(),
            count as f64 / MAX_ENVELOPES as f64,
        );
        let (ids, values) = params.collect_values();
        let restored = SynthParams::default();
        restored.restore_values(&ids.into_iter().zip(values).collect::<Vec<_>>());
        assert_eq!(restored.env_count.value_usize(), count);
        for ids in ENV_PARAMS {
            for id in ids.all() {
                assert_eq!(restored.get_plain(id.into()), params.get_plain(id.into()));
            }
        }
    }
}

#[test]
fn unrouted_oscillator_stops_at_note_off_while_env_one_releases() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    let result = driver!(Plugin)
        .duration(Duration::from_millis(100))
        .set_param(SynthParamsParamId::OscCount, 1.0 / 3.0)
        .set_param(SynthParamsParamId::Osc1Level, 0.0)
        .set_param(SynthParamsParamId::FilterMix, 0.0)
        .set_param(SynthParamsParamId::Osc2Phase, 0.25)
        .script(|script| {
            script.note_on(69, 1.0);
            script.wait_ms(20);
            script.note_off(69);
        })
        .run();
    let boundary = (result.sample_rate * 0.02) as usize;
    assert!(
        result.output[0][..boundary]
            .iter()
            .any(|sample| sample.abs() > 0.9)
    );
    assert!(
        result.output[0][boundary..]
            .iter()
            .all(|&sample| sample == 0.0)
    );
}

#[test]
fn envelope_one_does_not_gate_unrouted_oscillators_at_the_output() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    let render = |envelopes| {
        driver!(Plugin)
            .duration(Duration::from_millis(50))
            .set_param(SynthParamsParamId::OscCount, 1.0 / 3.0)
            .set_param(SynthParamsParamId::Osc1Level, 0.0)
            .set_param(SynthParamsParamId::Osc2Phase, 0.25)
            .set_param(SynthParamsParamId::Attack, 1.0)
            .set_param(SynthParamsParamId::EnvCount, envelopes)
            .script(|script| script.note_on(69, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let ungated = render(0.0);
    assert_eq!(render(0.25), ungated);
    assert!(ungated.iter().any(|sample| sample.abs() > 0.5));
}

#[test]
fn a_second_envelope_controls_its_routed_oscillator() {
    use crate::engine::ModDestination;
    use crate::plugin::{ModDestinationType, SynthParamsParamId, mod_destination_index};
    use std::time::Duration;
    use truce::prelude::ParamEnum;
    use truce_test::driver;

    let destination = f64::from(mod_destination_index(Some(ModDestination::OscLevel(1))))
        / (ModDestinationType::variant_count() - 1) as f64;
    let render = |attack| {
        driver!(Plugin)
            .duration(Duration::from_millis(50))
            .set_param(SynthParamsParamId::OscCount, 1.0 / 3.0)
            .set_param(SynthParamsParamId::Osc1Level, 0.0)
            .set_param(SynthParamsParamId::EnvCount, 0.5)
            .set_param(SynthParamsParamId::Env2Attack, attack)
            .set_param(SynthParamsParamId::Env2Mod1Destination, destination)
            .set_param(SynthParamsParamId::Env2Mod1Amount, 1.0)
            .script(|script| script.note_on(69, 1.0))
            .run()
            .output[0]
            .iter()
            .copied()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    };
    assert!(render(0.0) > 0.5);
    assert!(render(1.0) < 0.02);
}

#[test]
fn filter_mix_parameter_defaults_to_wet_and_bypasses_at_zero() {
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::driver;

    let params = SynthParams::default();
    let id = SynthParamsParamId::FilterMix;
    assert_eq!(params.get_plain(id.into()), Some(100.0));
    assert_eq!(
        params.format_value(id.into(), 50.0).as_deref(),
        Some("50 %")
    );
    let render = |mode, cutoff, q, mix| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(SynthParamsParamId::Attack, 0.0)
            .set_param(SynthParamsParamId::FilterType, mode)
            .set_param(SynthParamsParamId::FilterCutoff, cutoff)
            .set_param(SynthParamsParamId::FilterQ, q)
            .set_param(id, mix)
            .script(|script| script.note_on(69, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let dry = render(0.0, 0.0, 1.0, 0.0);
    for mode in [0.0, 0.5, 1.0] {
        assert_eq!(render(mode, 1.0, 0.0, 0.0), dry);
        assert_eq!(render(mode, 0.0, 1.0, 0.0), dry);
    }
    let wet = render(0.0, 0.0, 0.5, 1.0);
    let half = render(0.0, 0.0, 0.5, 0.5);
    let energy = |samples: &[f32]| samples[2_000..].iter().map(|s| s * s).sum::<f32>();
    assert!(energy(&wet) < energy(&dry) * 0.01);
    assert!((energy(&half) / energy(&dry) - 0.5).abs() < 0.03);
    params.set_normalized(id.into(), 0.25);
    let (ids, values) = params.collect_values();
    let restored = SynthParams::default();
    restored.restore_values(&ids.into_iter().zip(values).collect::<Vec<_>>());
    assert_eq!(restored.get_plain(id.into()), Some(25.0));
}

#[test]
fn every_lfo_can_modulate_filter_mix() {
    use crate::engine::ModDestination;
    use crate::plugin::{
        LFO_PARAMS, ModDestinationType, SynthParamsParamId, mod_destination_index,
    };
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::driver;

    let destination = f64::from(mod_destination_index(Some(ModDestination::FilterMix)))
        / (ModDestinationType::variant_count() - 1) as f64;
    for ids in LFO_PARAMS {
        let render = |amount| {
            driver!(Plugin)
                .duration(Duration::from_millis(100))
                .set_param(SynthParamsParamId::Attack, 0.0)
                .set_param(SynthParamsParamId::FilterCutoff, 0.0)
                .set_param(SynthParamsParamId::LfoCount, 1.0)
                .set_param(ids.shape, 1.0 / 3.0)
                .set_param(ids.destinations[0], destination)
                .set_param(ids.amounts[0], amount)
                .script(|script| script.note_on(69, 1.0))
                .run()
                .output[0]
                .clone()
        };
        let wet = render(0.5);
        let bypass = render(0.0);
        let energy = |samples: &[f32]| samples[2_000..].iter().map(|s| s * s).sum::<f32>();
        assert!(energy(&bypass) > energy(&wet) * 100.0);
    }
}

#[test]
fn oscillator_count_parameter_adds_oscillators() {
    use crate::plugin::{OSCILLATOR_PARAMS, SynthParams, SynthParamsParamId};
    use std::time::Duration;
    use truce::prelude::*;
    use truce_test::driver;

    let params = SynthParams::default();
    let count: u32 = SynthParamsParamId::OscCount.into();
    assert_eq!(params.get_plain(count), Some(1.0));
    for oscillator in OSCILLATOR_PARAMS {
        assert_eq!(params.get_plain(oscillator.level.into()), Some(100.0));
        assert_eq!(params.get_plain(oscillator.pitch.into()), Some(0.0));
    }

    let render = |oscillators: f64, osc_2_level: f64| {
        driver!(Plugin)
            .duration(Duration::from_millis(50))
            .set_param(SynthParamsParamId::Attack, 0.0)
            .set_param(SynthParamsParamId::OscCount, oscillators)
            .set_param(SynthParamsParamId::Osc1Level, 0.0)
            .set_param(SynthParamsParamId::Osc2Level, osc_2_level)
            .script(|script| script.note_on(57, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
    // With oscillator 1 muted, only an active oscillator 2 is heard.
    assert_eq!(energy(&render(0.0, 1.0)), 0.0);
    assert!(energy(&render(1.0 / 3.0, 1.0)) > 0.0);
    assert_eq!(energy(&render(1.0 / 3.0, 0.0)), 0.0);
}

#[test]
fn an_lfo_can_modulate_a_later_oscillator() {
    use crate::plugin::SynthParamsParamId;
    use std::time::Duration;
    use truce_test::driver;

    let render = |destination: f64, amount: f64| {
        driver!(Plugin)
            .duration(Duration::from_millis(100))
            .set_param(SynthParamsParamId::Attack, 0.0)
            .set_param(SynthParamsParamId::OscCount, 1.0 / 3.0)
            .set_param(SynthParamsParamId::Osc1Level, 0.0)
            .set_param(SynthParamsParamId::LfoCount, 0.25)
            .set_param(SynthParamsParamId::LfoRate, 0.9)
            .set_param(SynthParamsParamId::Mod1Destination, destination)
            .set_param(SynthParamsParamId::Mod1Amount, amount)
            .script(|script| script.note_on(57, 1.0))
            .run()
            .output[0]
            .clone()
    };
    let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
    let reference = render(0.0, 0.5);
    // Osc 2 Level keeps destination index 6.
    let osc_2_level = 6.0 / 15.0;
    let modulated = render(osc_2_level, 0.0);
    assert!(energy(&modulated) < energy(&reference) * 0.8);
    // A route to an inactive oscillator (Osc 3 Level) changes nothing.
    assert_eq!(render(8.0 / 15.0, 0.0), reference);
}

#[test]
fn patch_name_persists_with_host_state() {
    use truce::params::Params;
    let params = crate::SynthParams::default();
    assert_eq!(params.patch_name(), "");
    params.set_patch_name("Warm Pad");
    let saved = params.serialize_persist();

    let restored = crate::SynthParams::default();
    restored.load_persist(&saved);
    assert_eq!(restored.patch_name(), "Warm Pad");
}

#[test]
fn custom_lfo_shapes_persist_with_host_state() {
    use crate::engine::{LfoPoint, LfoShape};
    use truce::params::Params;
    let params = crate::SynthParams::default();
    assert_eq!(params.custom_lfo_shape(1), None);
    let shape = LfoShape::new(
        &[
            LfoPoint::curved(0.0, -1.0, 3.5),
            LfoPoint::new(0.4, 0.75),
            LfoPoint::new(0.9, 0.0),
        ],
        true,
    );
    params.set_custom_lfo_shape(1, Some(shape));
    assert_eq!(params.lfo_shape(1), shape);
    let saved = params.serialize_persist();

    let restored = crate::SynthParams::default();
    restored.load_persist(&saved);
    assert_eq!(restored.custom_lfo_shape(0), None);
    assert_eq!(restored.custom_lfo_shape(1), Some(shape));
    assert_eq!(restored.lfo_shape(1), shape);
}
