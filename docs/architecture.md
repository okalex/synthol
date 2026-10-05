# Synthol Architecture

## Goals

Synthol is currently a small Truce instrument: MIDI drives up to eight
oscillator voices (sine, square, triangle, or sawtooth) through an ordered chain
of up to 32 two-pole filters. A Slint editor controls output gain, oscillators,
the effects chain, ADSR and LFO modulators that can modulate oscillator pitch
and level and each filter's cutoff, Q, and mix, and the polyphony limit.
The intended product is a modular synthesizer where users
can add and connect oscillators, envelopes, LFOs, filters, effects, and other
modules.

The architecture should support that growth without making each module know
about the plugin host, UI, or other modules. It should also keep the audio
callback predictable: graph editing, validation, allocation, and destruction
belong off the real-time audio thread.

## Current implementation scope

The code is being migrated incrementally. The current foundation separates
the Truce adapter, Slint editor bridge, engine, MIDI event type,
multi-waveform oscillator, biquad filter, and reusable ADSR envelope.
`engine/effects.rs` represents the current serial effects topology as a bounded
list of stable slot identities, replacing the original fixed three-node graph.
At each block boundary the adapter derives the chain from enabled/order host
parameters. Processing mixes oscillators, then visits effects in chain order;
there is no allocation, locking, or graph compilation in the audio callback.
ADSRs are control sources, not audio nodes; ENV 1 defaults to multiplying
OSC 1's level. Output gain is applied after the chain. The filter (`engine/node/filter.rs`)
is an RBJ-cookbook biquad in low-pass, high-pass, or band-pass mode. Because
LFO modulation can move cutoff and Q per note, each voice owns a `Filter` per slot that
recomputes its coefficients only when its modulated settings change, plus its
own filter state, cleared when the voice goes silent. The editor's response
plot evaluates the same coefficients' magnitude response for the unmodulated
settings. The editor's full-width Effects section is a horizontally scrollable
row of compact filter cards. Each card's type
selector sits above the response plot, with cutoff, Q, and mix controls below.
The editor can add/delete filters and drag their titles to reorder them.
Horizontal scrolling, an explicit scrollbar, and edge scrolling during a
drag make all 32 slots reachable without interfering with knob drags.
Reordering retains settings, modulation targets, automation IDs, and per-slot
filter memory; insertion/removal clears the changed slot's memory.
Deleting a filter also clears routes targeting it. The first filter retains
its original parameter IDs. Additional filters use explicit nested parameter
bases. Route destinations retain the original 12-value enum and normalized
automation mapping; a separate per-route selector supplies the filter identity.
Adding future effect types should extend the slot runtime and card dispatch
rather than introduce a parallel audio path.
Each voice owns an LFO (`engine/node/lfo.rs`), and the engine owns
one more shared, free-running LFO for Sync mode; `engine/modulation.rs` routes
them to destinations. See "MIDI, notes, voices, and
modulation" below. A general connection graph,
graph publication, graph-document persistence, additional node types, and a
patching UI are future work. Sound patches are currently flat host-parameter
snapshots (see "Persistence and compatibility"). This keeps the current instrument's behavior unchanged while
establishing the boundaries for later stages.

## High-level structure

Keep the plugin adapter thin and put the reusable instrument engine in
host-independent Rust modules:

```text
DAW / Truce
    |
    +-- plugin adapter
    |     MIDI, host parameters, transport, buses, state, editor lifecycle
    |
    +-- synth engine
          |
          +-- graph model       serializable nodes and connections
          +-- graph compiler    validation, routing, execution schedule
          +-- graph runtime     prepared nodes, buffers, event/control flow
          +-- DSP modules       oscillators, envelopes, LFOs, filters, effects
          +-- parameter model  stable IDs, values, modulation destinations

Slint editor --> UI/view-model --> control commands --> graph builder/compiler
                                    |
                                    +--> host parameter API
```

A possible source layout as the project grows:

```text
src/
  lib.rs                    Truce entry point and plugin adapter
  plugin/
    params.rs               Host-visible parameters
    state.rs                Project/plugin state serialization
  engine/
    graph/
      model.rs              Node and connection definitions
      compile.rs            Validation and execution-plan construction
      runtime.rs            Prepared graph execution
      ports.rs              Audio, control, and event port types
    node/
      mod.rs                Node interfaces and registry
      oscillator.rs
      envelope.rs
      lfo.rs
      filter.rs
      effect.rs
      utility.rs
    midi.rs                 MIDI normalization and note tracking
    modulation.rs           Modulation sources and destination mapping
    parameter.rs            Engine parameter identity/value handling
  editor/
    mod.rs                  UI setup and engine command bridge
    view_model.rs           UI-facing graph/parameter state
ui/
  main.slint
```

This is a target organization, not a requirement to split every item into its
own file immediately. Extract a module when it has a clear responsibility or
independent tests.

## Graph model and compiled graph

Represent the user's patch as a serializable **graph document**. It contains
stable node IDs, each node's type and settings, and directed connections
between typed ports. Keep this document separate from the live DSP objects:
the document is editable and persists across sessions; the runtime is prepared
for one sample rate, channel layout, and maximum block size.

An abstract model could look like:

```text
Node {
    id: NodeId,
    kind: NodeKind,
    parameters: ...
}

Connection {
    from: PortAddress,
    to: PortAddress
}
```

Ports should distinguish at least:

* **Audio**: sample streams, with an explicit channel layout.
* **Control**: scalar or per-sample values used for modulation and parameter
  control.
* **Events**: timestamped MIDI/note/gate events.

The graph compiler runs away from the audio thread. It checks node and port
types, connection counts, required inputs, channel compatibility, resource
limits, and cycles. It then produces an execution plan, allocates or sizes
intermediate buffers, and prepares each node for the current audio
configuration. A typical acyclic audio/control graph can be scheduled in
topological order. Do not silently accept a cycle: either reject it with a
useful diagnostic or require an explicit delay/feedback node that defines the
cycle's sample or block delay.

The runtime executes a precomputed schedule and reuses its buffers. It must
not allocate, lock, perform file I/O, compile a graph, or destroy a large graph
on the audio thread. Render in blocks where possible; nodes that need
sample-accurate MIDI or modulation can process event offsets or per-sample
segments within a block.

Initially, a straightforward typed graph with preallocated buffers is more
valuable than a highly generic execution framework. Profile before adding
specialized scheduling, buffer aliasing, SIMD, or graph fusion.

## Module/node contract

Treat each DSP module as a reusable unit with:

1. A static description: kind/version, ports, parameter descriptors, and
   display metadata.
2. Editable settings that belong to the graph document.
3. Prepared runtime state that belongs only to the audio engine.
4. A preparation step for sample rate, maximum block size, and channel
   configuration, called off the audio thread.
5. A real-time process step that reads inputs and writes outputs using
   caller-owned/preallocated memory.
6. A reset/transport-change path for discontinuities and host state changes.

The node should not read UI widgets or call Truce APIs. The plugin adapter
translates host MIDI and parameter changes into engine inputs; the editor
submits user intent through a control layer. This keeps DSP reusable in tests,
a future standalone host, or another plugin format.

Node kinds should be registry-backed so the editor can enumerate available
modules and the loader/compiler can resolve serialized kinds. Prefer explicit
typed node implementations or a small common trait over making every node
perform dynamic string lookups in the audio callback. Resolve IDs to prepared
node indices during compilation.

## MIDI, notes, voices, and modulation

Normalize host MIDI into an engine event representation with sample offsets.
The engine owns note lifecycle and voice allocation, not individual
oscillators. A later polyphonic design can have a voice manager create or
recycle per-voice node state while shared effects and output routing remain
outside the voice. Keep voice policy (polyphony limit, stealing, mono/legato)
explicit rather than burying it in oscillator code.

Envelopes and LFOs are graph nodes. An envelope can consume note/gate events;
an LFO can run freely or synchronize to transport. Their outputs connect to
typed control inputs or parameter modulation destinations. Define modulation
combination rules deliberately (for example, additive offsets versus
multiplicative scaling) and clamp/convert values at the destination's
parameter domain. Do not make modulation mutate the saved base value of a
parameter.

The engine owns a fixed pool of `MAX_VOICES` (8) voices in
`engine/voice.rs`, allocated up front so note handling never allocates on the
audio thread. Each voice holds its own oscillator, per-effect filter, and
envelope state and runs the shared effects order; voice outputs are summed
before output gain. The
host-visible `Voices` parameter (1-8) limits how many voices new notes may
use. Allocation policy, in `SynthEngine::allocate_voice`, is: retrigger a voice
already sounding the same note, else take a free voice, else steal the oldest
releasing voice, else steal the oldest held voice. With one voice this
reproduces the earlier monophonic last-note-wins behavior. Lowering the limit
releases held voices above it and lets them finish their release.

The oscillator (`engine/node/oscillator.rs`) keeps a normalized 0-1 phase and
renders a `Waveform` (sine, PolyBLEP square, triangle, or PolyBLEP sawtooth).
The host-visible `Oscillator` enum parameter is mapped to the engine `Waveform` once per block
and applied to every voice; switching takes effect immediately, including on
sounding notes, without resetting phase.

The oscillator also has a pitch offset in semitones (cached as a frequency
ratio) and a 0-1 output level, applied per sample from the smoothed `Osc Pitch`
and `Osc Level` parameters.

A bipolar shape (-1 to 1, from the smoothed `Osc Shape` parameter's
-100%..100%) bends each waveform in `waveform_sample`. Shape 0 takes the
original `plain_waveform_sample` path, so unshaped output is bit-for-bit
unchanged. Sine and triangle split the cycle into a rise and fall whose ratio
follows the shape (half-cosine or linear segments), ending at falling and
rising saws. Square uses the shape as its duty cycle, with DC-free levels
scaled so the larger one is full scale and PolyBLEP steps scaled to the jump.
Sawtooth plays the plain PolyBLEP saw inside a window `1 - |shape|` of the
cycle wide, at the start for negative shapes and the end for positive ones,
and outputs zero outside it. Extreme segments are limited to `MIN_SEGMENT`
(2% of the cycle) or a few samples at high pitches to bound aliasing.
`render_cycle` takes the same shape so the editor's display matches.

Each oscillator preallocates 20 independent phases for unison. Its
`UnisonSettings` selects 1-20 subvoices, a symmetric pitch spread
(0-50 cents), and stereo width (0-1). Subvoices share the oscillator's
other controls and note lifecycle. Pitch offsets are evenly spaced across
the spread and converted to frequency ratios with `2^(cents / 1200)`,
with equal left/right groups and a centered subvoice
for odd counts. Linear pan gains and averaging keep zero-detune output
at the original level, independent of voice count and width. Newly enabled
subvoices start at the running primary phase; note-on resets every phase
to the configured start phase. Zero detune aligns phases each sample.
The voice mix and engine output are stereo, and each filter slot owns
separate left/right biquad memory while sharing coefficients. The adapter
writes both stereo channels or averages them for a mono host bus.
Unison parameters use new stable IDs; oscillator removal shifts them
alongside existing controls. Flat patch and host-state snapshots include
them automatically, with defaults preserving older patches.

The `Lfo` node (`engine/node/lfo.rs`) reuses the oscillator's waveform
functions without PolyBLEP. Its phase is kept
in `f64` because at 0.01 Hz the per-sample increment is below `f32`
resolution. Each voice owns one: the voice's note-on (including retrigger and
voice stealing) restarts it at phase 0, and it stops when the voice goes
silent, so notes have independent LFOs. `SynthEngine` also owns a shared
`sync_lfo` that free-runs while the mode is `Sync`. The engine's
`lfo_positions()` reports what to display: in `Trigger`, each sounding voice's
phase plus the most recently started voice; in `Envelope`, each still-running
voice's phase until its first cycle completes; in `Sync`, only the shared LFO.
The `LFO Shape`, `LFO Rate`, and `LFO Mode` parameters are read once per
block. After each block the plugin publishes the positions through Truce
meter slots `lfo_position_0` to `lfo_position_7` (encoded as `1 + phase`, with
`0` meaning stopped, because unwritten meters read `0`) and `lfo_newest`
(slot index plus one). The editor's per-frame sync decodes them into
`PositionMarker` rows: solid for the newest note and faint for the others.
Meter slots are display-only and not saved with presets.

Modulation (`engine/modulation.rs`) works in normalized knob space. Each
`ModDestination` (oscillator pitch, level, and shape, filter cutoff and Q) maps its
value to and from 0-1 along the same taper as its host parameter: linear for
pitch, level, and shape, logarithmic for cutoff and Q. The plugin exposes `MOD_SLOTS`
(4) routing slots as `LFO Route N Destination` and `LFO Route N Amount`
parameters. Every sample, `SynthEngine::set_modulation` sums the slots' bipolar
amounts per destination into `ModDepths`, and the engine passes the base
settings, depths, and (in Sync mode) the shared LFO's value to each voice as
`VoiceControls`. A voice advances its own LFO first, then offsets each
destination's normalized value by `depth * lfo` and clamps it to the range. A
zero offset leaves a value bit-for-bit unchanged, so unrouted settings cost
nothing extra and sound the same as before. Voice LFOs keep running in Sync
mode for display, but only the shared value modulates.

`ModDestination::ALL` and the host-visible `ModDestinationType` enum are
indexed by position, and saved routes store those indices, so new
destinations are appended: the `OscShape` destinations come after every
effect destination (`SHAPE_BASE`), and `Osc N Shape` after `Filter Mix` in the
host enum.

The editor reads the routing parameters each frame. It draws each routed
knob's depth arc and, from the newest LFO position, its live modulated value.
Dropping the drag handle on a knob fills the first empty slot unless that
destination is already routed. Alt-dragging a knob sets the total depth on
that destination by adjusting the first slot that targets it.

Each voice also owns up to `MAX_ENVELOPES` (4) independent ADSRs. They start
on note-on and release on note-off, and their active count is controlled by
the host's `Envelopes` parameter. ENV 1 retains the original `attack`, `decay`,
`sustain`, and `release` parameter IDs; the remaining envelopes and each
envelope's four routing slots have separate host parameters. Count, settings,
and routes are preset data; level meters are not.
The Modulators tabs select either an ADSR or an LFO, sharing routing, drag/drop,
depth edits, and destination-remapping on oscillator removal. Adding/removing
an envelope resets/compacts its parameter slots like an LFO.

ADSR values are advanced before audio generation. After the summed bipolar
LFO offset, each envelope applies its destination depth in envelope order.
Oscillator levels are multiplied by `1 - abs(depth) + abs(depth) * shaped`,
where `shaped` is the envelope for positive depth and its inverse for negative
depth. Other destinations receive `depth * envelope` normalized offsets.
The default ENV 1 route is OSC 1 Level at 100%; additional envelopes are unrouted.
There is no output-wide envelope stage. A releasing voice survives until all
its ADSRs finish; without ADSRs it stops on note-off. Oscillators without a
nonzero active ADSR depth on their Level are gated by the
held-note state, so they cannot sound during another oscillator's release tail.
This rectangular gate leaves their configured level and other modulation
unchanged while the note is held. After each block, the
newest sounding voice's envelope levels are published as `1 + level`, with 0
meaning no active voice, for the knob's live modulation marker.

## Parameters and host integration

Keep host-visible Truce parameters stable and separately identifiable from
graph-node parameters. Host parameters are valuable for automation, MIDI
learn, and DAW display; individual module settings need stable node-scoped
IDs for patch persistence. A module parameter can optionally be exposed to the
host, but the mapping must remain stable when graph nodes are reordered.

Use stable identifiers, not UI labels or array positions, for nodes, ports,
and parameters. Parameter metadata should define range, units, scaling,
smoothing, and whether the value is host-automatable. Apply smoothing in the
engine at a suitable point; host automation and modulation should not create
competing unsynchronized copies of the same value.

The plugin adapter is responsible for translating Truce's parameter/MIDI
interfaces to engine commands and reporting latency/tail. The core graph
engine should not depend on the plugin wrapper.

## Safe live graph edits

Do not edit the active graph in place from the UI or another control thread.
Use a control-side builder/compiler to create a complete candidate graph:

1. Apply an edit to the graph document.
2. Validate and compile the candidate off the audio thread.
3. Report compile errors to the editor and leave the current sound running.
4. Publish the prepared graph to the audio thread through a bounded,
   non-blocking handoff.
5. Swap graphs at an audio-block boundary.
6. Reclaim the retired graph on a non-audio thread.

The handoff must define backpressure: if a bounded queue is full, keep the
current graph and report that the update was not applied, or coalesce pending
edits on the control side. Do not fall back to blocking the audio thread.
Likewise, avoid dropping the last `Arc` or freeing a large graph from the
audio callback. A return queue, a proven reclamation scheme, or another
explicit deferred-destruction mechanism should own that responsibility.

For small initial versions, graph changes can be applied only while transport
is stopped if that meaningfully simplifies safe state transfer. If live
editing is a product requirement, design and test the handoff before the UI
depends on it.

## Persistence and compatibility

Serialize the editable graph document, not runtime buffers or Rust object
memory. Include a document format version, node IDs, node kind/version,
settings, connections, and relevant global engine settings. On load, migrate
older document versions explicitly and surface unknown nodes or invalid
connections as recoverable diagnostics; do not silently discard user work.

Until the graph document exists, `src/patch.rs` saves user patches as
versioned JSON snapshots of the non-read-only host parameters, keyed by stable
param ID and stored as plain values. Missing IDs load as defaults and unknown
IDs are ignored. Files use the `.synthol` extension and live in
`~/Library/Application Support/Synthol/Patches`. `src/editor/patches.rs` owns
the browser/save UI state and applies loads through host automation. The
loaded patch name is a `#[persist]` field so it survives session reloads.
Effect enabled/order parameters, filter controls, and route identity selectors
are included in the same snapshots and host state. Missing chain parameters
default to the original single filter, so version-1 patches remain compatible.
When the graph document lands, bump the patch format version and migrate
version-1 parameter snapshots explicitly.

Keep DSP state persistence distinct from patch serialization. Depending on
host behavior, a plugin state blob may include both the graph document and
selected runtime state, but runtime state must never be assumed to have a
stable binary layout. Truce's host-state integration should wrap/coordinate
with this format rather than becoming the graph's internal representation.

## UI boundary

The Slint editor is a client of the control layer, not the owner of the audio
graph. It displays a view of the graph document and submits commands such as
add node, remove node, connect ports, and set a parameter. The control layer
validates user edits, updates the document, requests compilation, and returns
success or actionable errors. UI widgets should not hold pointers to live DSP
nodes.

This boundary allows the graph engine to be tested without Slint and leaves
room for different editors or automation surfaces. UI selection, zoom,
position, and panel layout are editor state; they should not affect audio
rendering.

For real-time parameter display, use the shared Truce parameter store as the
source of truth rather than keeping independent knob, graph, and DSP copies.
During a drag, controls update the shared parameter store immediately. Host
automation edits are coalesced to the final value and sent by the editor's frame
sync callback after the gesture, rather than invoking host callbacks for every
pointer movement. The engine reads the shared parameter store during processing,
and the editor sync callback pushes current values and formatted labels back to
every view. Reusable controls expose normalized values and change callbacks,
while the adapter maps those values to stable parameter IDs. Host automation
uses the same parameter store.

## Testing strategy

Build confidence in layers:

* Unit-test each DSP node for signal behavior, edge cases, and parameter
  bounds.
* Test graph validation and compilation for port mismatches, missing inputs,
  cycles, and resource limits.
* Test graph output with deterministic inputs and tolerances appropriate for
  floating-point DSP.
* Test MIDI sample offsets, note lifecycle, voice allocation, and block
  boundaries.
* Test graph publication under audio/control concurrency, including full
  queues and deferred destruction.
* Test patch serialization, version migrations, and unknown node recovery.
* Keep Truce integration tests for plugin metadata, host parameters, editor
  construction, and end-to-end audio.

The existing pitch, note-on/off, silence, metadata, and editor tests provide a
useful baseline for the first extraction.

## Incremental migration

1. Extract the sine oscillator and MIDI note logic from `PluginLogic::process`
   into a small engine module without changing current behavior.
2. Introduce the graph document plus a fixed internal patch representing the
   current oscillator-to-output path; compile it during initialization/reset.
3. Add typed port definitions and graph validation before adding a visual
   patch editor.
4. Add envelopes, LFOs, filters, effects, and voice management as independent
   nodes with focused DSP tests.
5. Add the control-side edit/compile/publish flow and test safe graph swaps.
6. Build the patching UI on the graph document/control API, then add
   versioned persistence and migrations.

This sequence keeps the working instrument usable at each stage and avoids
building a visual graph editor before the engine's graph semantics and
real-time ownership rules are established.
