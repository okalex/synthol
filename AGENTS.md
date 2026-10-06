# Synthol VST Agentic Coding Guide

## Project Overview

Synthol is a polyphonic VST3 synthesizer plugin built with Rust and Truce. The architecture separates the Truce plugin adapter, Slint UI bridge, and a modular real-time DSP engine.

**Repository:** Synthol VST
**Language:** Rust 2024 edition
**Framework:** Truce 6.3.0 (VST3 wrapper) + Slint 1.15.1 (UI)
**Build:** `cargo truce build --vst3` (release) or `cargo truce install --shell --vst3` (hot-reload development)

---

## High-Level Architecture

```
DAW (Ableton, etc.)
  ↓
Truce VST3 Adapter (src/lib.rs)
  ├─ MIDI input handling
  ├─ Host parameter binding
  ├─ Audio callback (real-time, predictable)
  │
  ├─ Synth Engine (src/engine/)
  │   ├─ Voice Manager: polyphony (up to 8 voices)
  │   ├─ Oscillators: sine/square/triangle/sawtooth with shape modulation
  │   ├─ Effects Chain: serial filter slots with stable IDs
  │   ├─ Modulators: ADSR envelopes and LFOs (up to 4 of each)
  │   └─ Modulation Routing: 4 slots per modulator to any destination
  │
  └─ Slint Editor (ui/*.slint)
      ├─ Main layout: oscillators (left), modulators (right), effects (bottom)
      ├─ Control knobs: compact 33px dials with live modulation display
      └─ Patch browser: save/load JSON patches
```

---

## Key Modules

| Module | Purpose | File(s) |
|--------|---------|---------|
| **Plugin Adapter** | Truce entry point, host integration | `src/lib.rs` |
| **Voice Manager** | Polyphony, note stealing, gate control | `src/engine/voice.rs` |
| **Oscillators** | Multi-waveform synthesis with PolyBLEP | `src/engine/oscillator.rs` |
| **Effects Chain** | Slot-based serial filters, routing | `src/engine/effects.rs` |
| **Modulators** | ADSR envelopes, LFOs, routing | `src/engine/modulation.rs` |
| **Filters** | RBJ biquad low/high/band-pass | `src/engine/node/filter.rs` |
| **Editor UI** | Slint layout, controls, waveform plots | `ui/main.slint`, `ui/controls.slint` |
| **Parameters** | Host-automatable parameter definitions | `src/params.rs` (inferred) |

---

## Critical Constraints for Agentic Work

### 1. **Backward Compatibility is Not Required** ℹ️
The plugin is in active development and has not been publicly released. Backward compatibility with saved patches and host automation is **not a constraint**. Feel free to refactor parameter IDs, audio architecture, or UI layout if it improves code organization, consistency, or user experience. However:
- Document breaking changes in commits and pull requests
- Update patch migration logic if needed
- Consider whether new architecture would benefit from a clean slate approach

### 2. **Real-Time Audio Safety** ⚠️
Audio callback must remain predictable:
- **No allocation, locking, or blocking I/O in audio thread**
- **All graph changes happen off-thread** (when host calls parameter update)
- Parameter smoothing is sample-accurate; smoothed values are computed in the audio callback
- Filter coefficients are recomputed only when cutoff/Q/type changes (per voice)
- Voice allocation and destruction happen between audio blocks, never mid-callback

### 3. **Voice Stealing Logic**
When all 8 voices are busy and a new note arrives:
1. Steal the oldest **releasing** voice (gentler cutoff)
2. If none releasing, steal the oldest **held** voice
3. This prevents abrupt cutoff of actively playing notes

### 4. **Filter Memory & Reordering**
- Each voice owns a filter state per effects slot
- When effects are **reordered** (via drag): settings, automation IDs, modulation targets are preserved
- When a filter is **deleted**: its modulation routes are cleared
- When a filter is **added**: memory is initialized to zero

### 5. **Patch Persistence**
- Patches are JSON: `{ "format": "synthol-patch", "version": 1, "parameters": {...} }`
- Stored per-file in `~/Library/Application Support/Synthol/Patches/<name>.synthol`
- Loads reset missing parameters to defaults, ignore unknown IDs
- Rejects files from newer format versions (breaks on format change without migration)

---

## Development Workflow

### Build Commands

```bash
# Quick type check
cargo check

# Release build (static plugin)
cargo truce build --vst3

# Hot-reload development (keep Ableton open)
cargo truce install --shell --vst3
cargo truce build --shell --vst3  # Rebuilds dylib watched by shell

# Tests
cargo test
```

### Hot-Reload Workflow
1. Install shell once: `cargo truce install --shell --vst3`
2. Open Ableton, load Synthol
3. Edit code (DSP, MIDI, or UI)
4. Run `cargo truce build --shell --vst3`
5. Wait 1+ second, close/reopen editor in Ableton
6. Audio dropout during swap is expected; normal reload cycle

### When Full Reinstall Needed
Parameter definitions, plugin identity, or audio bus layout changes require:
```bash
cargo truce install --shell --vst3  # With Ableton closed
# Then restart Ableton and rescan plugins
```

---

## UI Architecture

### Slint Files
- `ui/main.slint` - Main layout, top-level structure
- `ui/controls.slint` - Reusable knob, slider, dropdown primitives
- `ui/effects.slint` - Effects chain card UI
- `ui/patches.slint` - Patch browser dialog

### Keyboard Event Handling
- **Patched `truce-slint`** in `vendor/truce-slint` enables selective keyboard capture
- `KeyboardCapture` handle allows Ableton's MIDI keyboard to work after clicking editor
- **Exception:** Patch dialogs capture all keys (except Esc for cancel)
- **Re-apply patch when upgrading Truce versions**

### Editor Dimensions
- Opens at **980 × 1016 logical pixels**
- Knobs: 33px dials with full-size text labels
- Effects chain: horizontally scrollable row of compact filter cards
- Content scrolls if exceeds window height

---

## Common Gotchas & Solutions

### Gotcha: Real-Time Audio Thread Safety (Critical)
```rust
// ❌ WRONG: allocation in audio thread
fn process() {
  let stolen = find_voice_to_steal();  // Can allocate/lock
}

// ✅ CORRECT: prepare off-thread, apply in callback
fn note_on() {
  let victim = find_victim();  // Off-thread
  schedule_steal(victim);  // Queued for callback
}
fn audio_callback() {
  apply_queued_steals();  // Only simple state changes
}
```

### Gotcha: Filter Coefficient Recomputation
```rust
// ✅ CORRECT: only recompute when parameters change
fn on_parameter_change(cutoff: f32, q: f32) {
  if cutoff != last_cutoff || q != last_q {
    coeffs = compute_coeffs(cutoff, q);  // Off-thread
    mark_dirty();
  }
}
fn audio_callback() {
  if is_dirty {
    apply_coeffs(coeffs);  // Apply prepared coeffs
    is_dirty = false;
  }
}
```

### Gotcha: Modifying Voice Stealing Logic
Voice stealing strategy is: oldest releasing voice first, then oldest held voice. Changes here should be tested with:
- All 8 voices playing a note each
- Triggering a 9th note immediately
- Verifying no audio glitches during the steal
- Checking that release tails don't get abruptly cut off

---

## Testing Strategy

```bash
cargo test
```

Tests cover:
- Pitch accuracy and MIDI note-to-Hz mapping
- Note-off handling and envelope release tails
- Editor state persistence (patches)
- Voice allocation and stealing

### Manual Testing Checklist
- [ ] Load plugin in Ableton
- [ ] Play a chord, verify correct polyphony limit
- [ ] Trigger 9th voice, verify oldest is stolen
- [ ] Drag effects to reorder, verify automation is preserved
- [ ] Save/load a patch, verify all parameters match
- [ ] Change cutoff/Q rapidly, verify no audio glitches
- [ ] Edit oscillator shape knob, verify waveform plot updates in real-time

---

## File Organization Reference

```
synthol-vst/
├── src/
│   ├── lib.rs                    # Truce adapter, plugin entry point
│   ├── engine/
│   │   ├── voice.rs             # Polyphony, note stealing, gates
│   │   ├── oscillator.rs        # Multi-waveform synthesis
│   │   ├── effects.rs           # Serial effects chain
│   │   ├── modulation.rs        # LFO/envelope routing
│   │   └── node/
│   │       ├── filter.rs        # RBJ biquad filter
│   │       ├── lfo.rs           # LFO oscillator
│   │       └── envelope.rs      # ADSR envelope
│   └── params.rs                # Host parameter definitions (inferred)
├── ui/
│   ├── main.slint               # Top-level layout
│   ├── controls.slint           # Knob/slider/dropdown primitives
│   ├── effects.slint            # Effects card UI
│   └── patches.slint            # Patch browser
├── build.rs                     # Slint build script
├── Cargo.toml
├── docs/
│   ├── architecture.md          # Long-form design doc
│   ├── todo.md                  # Roadmap
│   └── ...
├── AGENTS.md                    # This file
└── kilo.json                    # Kilo project config
```

---

## When to Escalate to Expert Review

Before committing significant changes:
- [ ] Audio callback logic changes → verify no allocations/locks
- [ ] Real-time data structure changes → check thread safety
- [ ] Major architecture refactors → discuss design approach first
- [ ] Voice stealing or polyphony changes → manual testing (all 8 voices, 9th note, no glitches)
- [ ] New DSP nodes or effects → verify CPU overhead and numerical stability
- [ ] UI keyboard handling → verify Esc and dialog behavior still works


---

## Quick Links

- **README.md** - User-facing plugin documentation
- **docs/architecture.md** - Detailed design rationale
- **docs/todo.md** - Development roadmap
- **Cargo.toml** - Dependencies and build profiles
- **Truce Docs:** https://github.com/truce-audio/truce
