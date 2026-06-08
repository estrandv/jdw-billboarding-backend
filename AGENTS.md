# AGENTS.md — jdw-billboarding-backend

## Architecture

**This crate is library-only.** Consumed as a git dependency by `jdw-suite`.

## Source Structure

```
src/
  lib.rs          # Public API exports
  shuttle.rs      # Shuttle Notation tokenizer + parser
  billboard.rs    # Mini-billboard line parser (legacy)
  full.rs         # Full billboard parser: classify, group, build, resolve
  macros.rs       # $macro template expansion system
  note_utils.rs   # Scale/key math, MIDI→frequency conversion
  config.rs       # TOML config loader (~/.config/jdw.toml + config.toml)
  osc.rs          # OSC message conversion, ElementConverter, send/dump helpers, NRT bundles
  synthdefs.rs    # SynthDef template compiler and loader (port of compile_scd.py)
  sample_loader.rs# Sample pack scanner + /load_sample message builder
  score.rs        # NRT Score class — timeline composition with group filter ordering
  listener.rs     # NRT Listener — background OSC UDP server for /nrt_record_finished
scripts/
  compare_scds.py # SCD diff tool: old_method (correct Python) vs Rust output
docs/
  NRT_OVERVIEW.md  # NRT pipeline, Score/extend_groups, bundle format, diff tool, lessons learned
```

## Pipeline

```
.bbd file
  → macros::load_and_expand  (expand $macros, load common_macros.txt)
  → full::parse              (classify lines, group sections, build Billboard)
  → osc::send_*              (setup/update/play/nrt)
```

## Status — 2026-06-08

### Fully Working
- **Live play**: setup/update/play verified against Python (94/94 messages match)
- **NRT recording**: all 23 arena.bbd tracks render correctly, verified against Python SCDs
- **All 164 tests passing** (159 lib + 5 integration)
- **SCD diff tool**: `scripts/compare_scds.py` two-pass structural comparison

## NRT Diff Tool

```bash
# Batch compare all 23 pairs (old_method Python vs Rust)
python3 scripts/compare_scds.py

# Single pair
python3 scripts/compare_scds.py old.scd new.scd
```

The old_method reference SCDs are at `~/tmp/nrt_bug_export/old_method/`.
Rust output is at `~/jdw_output/`.

## Tests

```
cargo test                          # 159 library tests
cargo test --test integration       # 5 integration tests
```
All 164 tests passing.
