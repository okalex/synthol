# Synthol VST

A minimal polyphonic instrument plugin built with Rust and
[Truce](https://github.com/truce-audio/truce). It accepts MIDI note-on and
note-off events, uses MIDI note numbers for pitch, and scales amplitude by
note-on velocity. A reusable ADSR envelope shapes the oscillator at the output
stage (10 ms attack, 500 ms decay, -6 dB sustain, 1 s release) before output
gain. Up to eight voices can sound at once; a **Voices** dropdown in the editor
(also a host-automatable parameter) sets the limit from Mono to 8. When all
voices are busy, a new note steals the oldest releasing voice, or else the
oldest held one. Voices are summed without normalization, so large chords can
exceed 0 dBFS; lower the output gain if needed.

An **Oscillator** dropdown (also a host-automatable parameter) selects the
waveform: sine, square, triangle, or sawtooth. The square and sawtooth waves
are band-limited with PolyBLEP; the triangle is generated directly and has
some mild aliasing at high pitches. Square is full-scale, so it sounds louder
than the other shapes.

The synth has no modulation, filters, or effects yet. The Slint editor
currently provides a reusable ADSR graph/knob panel for the output envelope,
an output-gain slider, and a voice-count dropdown. Attack, decay, and release each range from 0 ms to 10 s
on a skewed taper that gives short times more of the knob. The controls use
Slint's software renderer and are drag-only. The editor passes every keyboard
event back to the host, so Ableton's computer MIDI keyboard remains available
after clicking the editor. This relies on a patched copy of `truce-slint` in
[`vendor/truce-slint`](vendor/truce-slint) (wired up via `[patch.crates-io]`
in `Cargo.toml`); upstream truce-slint captures all keys once the editor has
focus. Re-apply the patch when upgrading Truce.

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
