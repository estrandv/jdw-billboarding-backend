# AGENTS.md — jdw-billboarding-backend

## Architecture

**This crate is library-only.** It has no binary, no CLI, no install.sh.
It is consumed as a git dependency by `jdw-suite` (the end-user `jdw` binary).

The `jdw` CLI lives at `/home/estrandv/programming/jdw-suite/` and calls
into this crate's public API via `client.rs`.

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
  osc.rs          # OSC message conversion, ElementConverter, send/dump helpers
  synthdefs.rs    # SynthDef template compiler and loader (port of compile_scd.py)
  sample_loader.rs# Sample pack scanner + /load_sample message builder
```

## Pipeline

```
.bbd file
  → macros::load_and_expand  (expand $macros, load common_macros.txt)
  → full::parse              (classify lines, group sections, build Billboard)
  → osc::send_*              (setup → commands → effects → drones → queue)
```

Setup order (matches Python, SC bus order is strict):
```
send_samples → send_full_setup (synthdefs) → send_effects_clear
→ send_full_commands (routers) → send_effects_create → send_drones_create
```

## Completed Work

### Core Pipeline
- **Macros** (`macros.rs`) — Full `$macro` expansion with `$name(args)`, `$:arg` placeholders, `common_macros.txt` loading
- **Parser** (`full.rs`) — Line classification, section grouping, synth headers, track definitions, effect definitions, commands, DEFAULT, arg inheritance with operators; bare `/address` commands classified per grammar
- **Shuttle** (`shuttle.rs`) — Tokenizer + parser for shuttle notation: notes (`c4`, `14`), sections `( ... )`, alternations `/`, repeats `*N`, args `:key=val`, loop markers `§`, rest/silence `x`, `@` mod, `$` drone
- **IMPORTANT: Tree-sitter grammar is the single source of truth** for .bbd syntax. Our `full.rs` approximates the grammar at `/home/estrandv/programming/tree-sitter-jdw-billboarding/grammar.js`. When in doubt, reference the grammar.

### OSC / Protocol
- **ElementConverter** (`osc.rs`) — Stateful converter with `{nodeId}` external ID scheme
- **Instrument routing** — `Sampler` → `/play_sample`, `Synth` → `/note_on_timed`, `Drone` → `/note_on`/`/note_modify`
- **Suffix handlers** — `@` (mod), `$` (drone on), `x` (silence → `/empty_msg`), `.` (legacy x), `§` (loop marker)
- **Protocol** — `/note_on_timed`, `/note_modify`, `/play_sample`, `/note_on`, `/free_notes`, `/hard_stop`, `/wipe_on_finish`, `/create_synthdef`, `/load_sample`, `/jdw_sc_event_trigger`
- **Bundle hierarchy** — `batch_update_queues` → `update_queue` → `timed_msg`
- **Command translation** — `/create_router` → `/note_on "router"`, `/create_effect` → `/note_on` + `/note_modify`

### Setup / Configure (verified working for arena.bbd)
- **Sample loading** (`sample_loader.rs`) — Scans `~/sample_packs/`, assigns buffer_index (100+), tone_index (per-pack), categorizes by filename keyword. Sends `/load_sample` during setup.
- **Effects** — `send_effects_clear` → `/free_notes "^effect_(.*)"`; `send_effects_create` → `/note_on` per `€`-defined effect (inherits section header args for bus routing)
- **Drones** — `send_drones_create` → `/note_on` per drone track (amp=0, inherits section header args)
- **SynthDef reload** — `update()` resends synthdefs, matching Python configure
- **Config** (`config.rs`) — `sample_pack_dir`, `first_buffer_index` in `JdwConfig`

### jdw-suite Integration
- **jdw-billboarding-backend is library-only** — no binary; consumed by `jdw-suite` as a git dependency
- **jdw-suite** (`/home/estrandv/programming/jdw-suite/`) provides the `jdw` CLI: `jdw setup`, `update`, `play`, `stop`, `quiet`, `terminate`
- `client.rs` in jdw-suite calls all OSC functions: `send_samples`, `send_full_setup`, `send_effects_clear`, `send_full_commands`, `send_effects_create`, `send_drones_create`, `send_full_queue_update`, `send_stop`, `send_silence_drones`

### OSC Comparison & Debugging
- **Verified against Python** for `arena.bbd` — all 3 phases (setup, update, play) captured and compared
- `parse_osc_dump.py` — parses tcpdump pcap into human-readable OSC (`--compact` for diffing)
- `normalize_rust_dump.py` — converts Rust dump output to comparable format
- `capture_compare.sh` — single-command, non-interactive script: captures all 3 Python phases with one sudo, auto-splits by OSC sentinel markers, dumps Rust equivalents, prints comparison table
- `dump_osc` example — `--phase setup|commands|play|all` for per-phase Rust dumps

### Bug Fixes
- **Shuttle decimal** — `.` in `amp0.5` tokenized correctly (was splitting on dot)
- **Args precedence** — DEFAULT → header → element → track_overrides. Header must override default (was using `or_insert` both ways); element inline > header; track operators apply last. Fixed `amp` values (blip 1→0.08, aPad 1→0.8).
- **Rest/silence** — `is_symbol` now checks `prefix` (not just suffix); shuttle puts `x` in prefix. Rest elements now produce `/empty_msg`, not `/play_sample`.
- **Deterministic args** — `args_as_osc` sorts HashMap keys alphabetically.
- **Effect args** — Effects now inherit section header args (`out`, `relT`, etc.) via `build_billboard`. Without this, effects routed to bus 0.
- **Creation order** — Commands (routers) now sent BEFORE effects/drones. SC bus chain: router → effect → notes. Effects created before routers had no bus to target.
- **Router arg types** — `/create_router` now sends `Float` (not `Int`) for in/out. jdw-sc's `/note_on` rejects Int custom args.
- **Group filter** — only tracks in `billboard.filters.last()` are included in queue update.
- **Filter collection** — all `>>>` lines collected (matching Python), not just first chain.

## Remaining Work

### NRT recording pipeline
Port `nrt_scoring.py` (Score class) and `listener.py` (OSC response listener):
- Chronological score composition from group filters
- Preload batching (`/clear_nrt`, synthdefs, samples)
- `/nrt_record_info` with BPM, filename, end time
- Wait for `/nrt_record_finished` response via OSC listener

### Minor
- `jdw all` idempotency: kill existing scsynth/sclang before re-launch
- Effect modulation during update (`/note_modify` for existing effects, not recreate)

## Tests

```
cargo test                          # 128 library tests
cargo test --test integration       # 5 integration tests (16 .bbd files)
```

All 133 tests passing.
