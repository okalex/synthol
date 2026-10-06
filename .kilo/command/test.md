Run tests for pitch accuracy, MIDI handling, note-off behavior, and patch persistence:

```bash
cargo test
```

Tests verify:
- MIDI note-to-Hz mapping
- Envelope release tails
- Voice allocation and stealing
- Patch save/load serialization
