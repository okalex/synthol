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
        .duration(Duration::from_millis(16))
        .script(|script| {
            script.note_on(69, 1.0);
            script.wait_ms(8);
            script.note_off(69);
            script.wait_ms(8);
        })
        .run();

    assertions::assert_nonzero(&result);
    assertions::assert_silence_after(&result, Duration::from_millis(8));
    assertions::assert_no_nans(&result);
}
