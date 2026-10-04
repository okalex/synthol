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
