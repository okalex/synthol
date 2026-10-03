# Synthol VST

A minimal monophonic sine-wave instrument plugin built with Rust and
[Truce](https://github.com/truce-audio/truce). It accepts MIDI note-on and
note-off events, uses MIDI note numbers for pitch, and scales amplitude by
note-on velocity. The most recent note-on takes over the single oscillator;
only a note-off for that active pitch stops it.

The synth has no envelopes, modulation, filters, or effects yet. The Slint
editor currently provides a single output-gain slider and uses Slint's
software renderer. The slider is drag-only and does not take keyboard focus,
so Ableton's computer MIDI keyboard remains available after adjusting it.

## Build

Install the Truce command-line tools once:

```sh
cargo install cargo-truce
```

Build the VST3 bundle:

```sh
cargo truce build --vst3
```

To install it for your DAW to scan:

```sh
cargo truce install --vst3
```

## Hot reload in Ableton

The standard install above builds a static plugin; it does not hot-reload.
For development, install Truce's reloadable VST3 shell once instead:

```sh
cargo truce install --shell --vst3
```

Then scan for plugins in Ableton and load **Synthol**. Keep Ableton open while
iterating. After editing the DSP, MIDI handling, or Slint UI, rebuild the logic dylib.
For the canonical shell-mode build (which also refreshes the sidecar and
development bundle), use:

```sh
cargo truce build --shell --vst3
```

This rebuilds the logic dylib watched by the installed shell. For this
project, `cargo build --release` also rebuilds that same dylib; the Truce
command is preferable because it keeps the shell bundle and sidecar in sync.
The watcher polls every 500 ms and waits for the file to settle before
reloading, so wait at least a second after the build completes, then close and
reopen the editor. A brief audio dropout during the swap is expected. You do
not need to reinstall the VST3, restart Ableton, or rescan for DSP, MIDI, or
Slint UI changes.

Use `cargo truce install --shell --vst3` with Ableton closed only when the
installed shell itself needs to be refreshed, such as after changing the UI
backend or plugin integration. Restart Ableton after that install so it loads
the refreshed shell binary. Ordinary UI/layout edits use the rebuild-and-
reopen loop above.

Changes to parameter definitions, plugin identity, or audio bus layout require
rebuilding/reinstalling the shell and rescanning in Ableton because the host
caches those details. The shell build is an experimental development workflow;
use the regular non-shell release build when preparing a plugin to ship.

The plugin can also be built directly with Cargo:

```sh
cargo build --release
```

Use `cargo test` to run the pitch, MIDI playback, note-off, and editor tests.
See [docs/architecture.md](docs/architecture.md) for the planned modular
synth engine, graph, and real-time update architecture.
