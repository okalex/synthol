# Synthol Development Roadmap

## Phase 1: Foundation (Completed)
- [x] Basic polyphonic synthesizer (8 voices)
- [x] Oscillators with waveform shaping
- [x] Serial effects chain (up to 32 filters)
- [x] ADSR and LFO modulators
- [x] Patch save/load
- [x] Unison (multi-voice per note)

## Phase 2: More Effects (In Progress)

### Saturator
- Goal: Soft clipping with tone control
- Complexity: Medium (similar to filter)
- Status: Not started
- Test: Compare saturation curves with industry standard

### Delay
- Goal: Configurable feedback delay
- Complexity: Medium (requires circular buffer management)
- Status: Not started
- Constraint: Must not exceed 10s max delay (memory)

### Compressor
- Goal: Dynamic range compression
- Complexity: High (envelope follower, gain reduction scheduling)
- Status: Not started
- Architecture: Track RMS level, apply gain reduction per voice

### Reverb
- Goal: Schroeder reverb or similar
- Complexity: High (state management, tuning)
- Status: Not started
- Challenge: Avoid excessive CPU; consider fixed-size buffer pool

## Phase 3: Per-Oscillator Effects Chain
- Goal: Route individual oscillators through effects
- Status: Not started
- Architecture: Extend current serial chain to support branching
- Note: Requires significant refactor of effects.rs

## Phase 4: MIDI Effects
- Goal: Arpeggiator with configurable patterns
- Status: Not started
- Constraint: Must not introduce latency in audio path
- Architecture: Prepare note sequences off-thread, apply in audio callback

## Phase 5: Component Bypass
- Goal: Add bypass toggle to filters/effects without removing them
- Status: Not started
- Preserve parameter state when bypassed
- Benefit: Easy A/B comparison without re-tuning

## Known Issues / Blockers
- None currently

## Testing Checklist
- [ ] Load plugin in Ableton
- [ ] Play a chord, verify correct polyphony limit (8 voices)
- [ ] Trigger 9th voice, verify oldest is stolen without audio glitch
- [ ] Drag effects to reorder, verify automation is preserved
- [ ] Save/load a patch, verify all parameters match
- [ ] Change cutoff/Q rapidly, verify no audio zippering
- [ ] Edit oscillator shape knob, verify waveform plot updates in real-time

