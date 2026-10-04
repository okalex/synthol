# Synthol VST

A minimal polyphonic instrument plugin built with Rust and
[Truce](https://github.com/truce-audio/truce). It accepts MIDI note-on and
note-off events, uses MIDI note numbers for pitch, and scales amplitude by
note-on velocity. Each voice mixes its oscillators and runs them through a filter, then a
reusable ADSR envelope shapes it at the output stage (10 ms attack, 500 ms decay, -6 dB sustain, 1 s release) before output
gain. Up to eight voices can sound at once; a **Voices** dropdown in the editor
(also a host-automatable parameter) sets the limit from Mono to 8. When all
voices are busy, a new note steals the oldest releasing voice, or else the
oldest held one. Voices are summed without normalization, so large chords can
exceed 0 dBFS; lower the output gain if needed.

The **Oscillators** section starts with one oscillator and shows them in a
tabbed panel, one tab per oscillator. **+ Add** next to the heading adds
another (up to four) and opens its tab; each tab's × button removes that
oscillator (the last one can't be removed), and later oscillators move up to
fill the gap. The number
of oscillators is the host-automatable **Oscillators** parameter, and each
oscillator has its own host-automatable type, phase, pitch, and level
parameters ("Osc 1 Type" through "Osc 4 Level"). Every voice plays all active
oscillators on its note and sums them, unnormalized, before the filter, so
several oscillators at full level can exceed 0 dBFS; lower their levels if
needed.

Each oscillator's tab has a type dropdown that selects the
waveform: sine, square, triangle, or sawtooth. The square and sawtooth waves
are band-limited with PolyBLEP; the triangle is generated directly and has
some mild aliasing at high pitches. Square is full-scale, so it sounds louder
than the other shapes. A **Phase** knob (0° to 360°, also host-automatable)
sets where in the cycle each new note starts; sounding notes are unaffected.
Phases other than 0° start mid-cycle, so short attacks may click. A **Pitch**
knob transposes that oscillator by ±24 semitones and a **Level** knob (0% to
100%) sets its volume before the filter; both are host-automatable and
smoothed. Next to the
dropdown, a waveform display plots one cycle (0° to 360°) of the selected
shape as it plays from the chosen start phase, updating live while the knob
turns. It is computed from the oscillator's own sample function rather than
drawn by hand.

The **Filter** row selects a 12 dB/octave (two-pole) low-pass, high-pass, or
band-pass filter, with **Cutoff** (20 Hz to 20 kHz) and **Q** (0.1 to 20)
knobs, plus a **Mix** knob (0% to 100%); all are host-automatable. Mix defaults
to 100%, preserving existing sounds. At 0% the filter is exactly bypassed;
100% uses the original filter. Intermediate values morph the filter's
power response, rather than summing separate dry and phase-shifted wet
signals: `|H_mix|² = (1 - mix) + mix * |H_filter|²`, where mix is 0 to 1.
The morph uses a single minimum-phase biquad with no added latency, avoiding
the cancellation notches that a conventional dry/wet sum can introduce.
It still has the natural phase response of a minimum-phase filter; it is
not a linear-phase filter. Mix reduces resonance as well as attenuation,
and is smoothed over 20 ms. The filters are RBJ-cookbook biquads:
low and high pass resonate above Q 0.707 (Butterworth, -3 dB at the cutoff),
and band pass peaks at 0 dB at the cutoff, narrowing as Q rises. Every voice
has its own filter memory. Cutoff and Q are smoothed so sweeps don't zipper.
The defaults (low pass, 20 kHz, Q 0.707) leave the sound nearly unfiltered. A
response display plots gain (+24 dB to -48 dB) from 20 Hz to 20 kHz on a log
axis, computed from the same coefficients the DSP uses and updating live as
the controls move. The editor doesn't know the host sample rate, so the plot
assumes 48 kHz; near Nyquist the real response can differ slightly.

The tabbed **Modulators** group starts empty. **+ Add** creates an LFO and opens
its tab, up to four LFOs. Each tab's × button removes that LFO, including the
last one; later LFOs and their routes move up to fill the gap. Newly added LFOs
start with default controls and no routes. The host-automatable **LFOs**
parameter controls the active count (0 to 4).

Each LFO has independent controls and four routing slots. A dropdown selects its shape (sine, square, triangle, or sawtooth, without
band-limiting), a **Rate** knob sets 0.01 Hz to 30 Hz on a log taper, and a
**Mode** dropdown selects **Trigger** or **Sync**; all three are
host-automatable. In Trigger mode every note gets its own LFO, which starts
from 0° when that note is pressed and stops when that note's release ends, so
a note played later starts a fresh cycle while earlier notes' LFOs keep
running. In Sync mode a single shared LFO runs continuously and ignores notes.
A plot in the selected tab shows one cycle of its shape. Vertical lines mark the current
positions, updated every editor frame from the audio thread: the most
recently pressed note's line is solid and older notes' lines are faint (up to
one per voice). Because the position comes from audio processing, a
Sync LFO only advances while the host is processing the plugin.

Each LFO can modulate any oscillator's **Pitch** and **Level** and filter
**Cutoff**, **Q**, and **Mix**. Drag the amber **MOD** handle from the LFO Routing row onto one of
those knobs (they light up while you drag) to route the LFO to it; to target
another oscillator, select its tab first. A routed
knob shows an amber arc: the faint arc covers the full swing around the knob's
value, the bright arc shows the direction and depth of a positive LFO peak,
and while a note plays a dot shows the current modulated value. Alt-drag a
routed knob up or down to change the depth without touching the knob's value;
hold Shift for fine control. The routing list below the handle shows the four
routing slots, each a host-automatable **Destination** and bipolar **Amount**
(-100% to 100% of the destination's knob range). Each slot has a destination
dropdown (listing the pitch and level of every active oscillator), an amount slider (Shift-drag for fine control, double-click to reset),
the depth in the destination's units, and a × button that clears the slot.
The selected Modulators tab determines which LFO the handle, routing list,
and Alt-drag depth edits control. Arcs show that LFO's depth; the live dot
includes the combined modulation of all active LFOs. A
new route starts at a modest depth: 1 semitone, 25% level, 1 octave of cutoff,
×2 Q, or 25 percentage points of Mix. The amounts are measured along each knob's own taper, so pitch moves
in semitones, cutoff in octaves, and Q multiplicatively (shown as ×/÷).
Modulated values stay within each knob's range, and several slots targeting the
same destination add up, including routes from different LFOs, before clamping.
Every note is modulated by its own LFO in Trigger mode
and by the shared LFO in Sync mode, and each voice has its own filter, so
modulating the cutoff doesn't affect other notes. Removing an oscillator clears
the routes that target it, and routes to later oscillators follow them as they
move up.

Existing oscillator and first-LFO parameter IDs are retained. Sessions saved
before the LFO count parameter existed load with no active LFOs; add the first
LFO by setting the host's **LFOs** parameter to 1 to reactivate its saved routes
without resetting them.

The synth has no effects yet. The Slint editor
currently provides a reusable ADSR graph/knob panel for the output envelope,
an output-gain slider, a voice-count dropdown, and the oscillator, filter, and
LFO controls. Attack, decay, and release each range from 0 ms to 10 s
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
