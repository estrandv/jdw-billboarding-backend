# jdw-billboarding-backend

Rust-native billboard notation parser and OSC converter for JackDAW.

Port of `jdw-billboarding-lib` (Python) and `jdw-pycompose`. Consumed as a
library by `jdw-suite`, which orchestrates the full ecosystem.

## Status: Fully Working

- Live play: 100% match vs Python for setup/update/play (arena.bbd)
- NRT recording: 23/23 tracks match Python SCD output (arena.bbd)
- 167 tests passing (162 lib + 5 integration)

## Usage

```rust
use jdw_billboarding_backend;

// Parse a .bbd file
let bb = jdw_billboarding_backend::parse_billboard_file("song.bbd")?;

// Live play — send to sequencer
jdw_billboarding_backend::send_full_setup(&synthdefs, &config)?;
jdw_billboarding_backend::send_full_queue_update(&bb, &config)?;

// NRT — render to WAV files
let bundles = jdw_billboarding_backend::get_nrt_record_bundles(
    &bb, &synthdefs, &samples, "~/jdw_output",
);
```

## Docs

| File | Content |
|---|---|
| `AGENTS.md` | Architecture, pipeline, test commands |
| `docs/NRT_OVERVIEW.md` | NRT pipeline, Score/extend_groups, bundle format, diff tool |
| `OSC_COMPARISON_PLAN.md` | Live play OSC comparison (all resolved) |
| `scripts/compare_scds.py` | SCD diff tool — two-pass structural comparison |
