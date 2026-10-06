Install hot-reloadable shell for Ableton development:

```bash
cargo truce install --shell --vst3
```

Then keep Ableton open and rebuild with:
```bash
cargo truce build --shell --vst3
```

After building, wait 1+ second, then close and reopen the editor in Ableton to see changes. Audio dropout during reload is normal.
