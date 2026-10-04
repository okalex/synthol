# Synthol Architecture

## Goals

Synthol is currently a small Truce instrument: MIDI drives one sine oscillator,
and a Slint editor controls output gain. The intended product is a modular
synthesizer where users can add and connect oscillators, envelopes, LFOs,
filters, effects, and other modules.

The architecture should support that growth without making each module know
about the plugin host, UI, or other modules. It should also keep the audio
callback predictable: graph editing, validation, allocation, and destruction
belong off the real-time audio thread.

## Current implementation scope

The code is being migrated incrementally. The current foundation separates
the Truce adapter, Slint editor bridge, engine, MIDI event type, sine
oscillator, and reusable ADSR envelope. Engine construction compiles a fixed
typed oscillator-to-output-envelope-to-output graph once; processing follows
its prepared order without compiling or allocating in the audio callback.
Output gain is applied after the envelope. This fixed graph is an internal
representation only: it is not user-editable or serialized, and graph
publication, patch persistence, additional node types, and a patching UI are
future work. This keeps the current instrument's behavior unchanged while
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

The present engine is monophonic and last-note-wins. Preserve that behavior
while extracting its oscillator and MIDI handling; add a voice manager as a
separate feature rather than making the initial graph abstraction depend on
polyphony prematurely.

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
