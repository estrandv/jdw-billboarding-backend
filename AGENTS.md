# AGENTS.md — jdw-billboarding-backend

## Architecture

**This crate is library-only.** It has no binary, no CLI, no install.sh.
It is consumed as a git dependency by `jdw-suite` (the end-user `jdw` binary).

The `jdw` CLI lives at `/home/estrandv/programming/jdw-suite/` and calls
into this crate's public API via `client.rs`.

## Source Structure

```
src/
  lib.rs        # Public API: parse_billboard, parse_billboard_file
  shuttle.rs    # Hand-written Shuttle Notation parser + expander
  billboard.rs  # Mini-billboard line parser (legacy)
  full.rs       # Full billboard parser: classify, group, build, resolve
  macros.rs     # $macro template expansion system
  note_utils.rs # Scale/key math, MIDI→frequency conversion
  config.rs     # TOML config loader (~/.config/jdw.toml + config.toml)
  osc.rs        # OSC message conversion, ElementConverter, send helpers
```

## Pipeline

```
.bbd file
  → macros::load_and_expand  (expand $macros, load common_macros.txt)
  → full::parse              (classify lines, group sections, build Billboard)
  → osc::send_*              (setup/update/commands/stop/quiet)
```

## Completed Work

### Core Pipeline
- **Macros** (`macros.rs`) — Full `$macro` expansion with `$name(args)`, `$:arg` placeholders, `common_macros.txt` loading
- **Parser** (`full.rs`) — Line classification, section grouping, synth headers, track definitions, effect definitions, commands, DEFAULT, arg inheritance with operators; bare `/address` commands classified per grammar
- **Shuttle** (`shuttle.rs`) — Tokenizer + parser for shuttle notation: notes (`c4`, `14`), sections `( ... )`, alternations `/`, repeats `*N`, args `:key=val`, loop markers `§`, rest/silence `x`, `@` mod, `$` drone
- **IMPORTANT: Tree-sitter grammar is the single source of truth** for .bbd syntax. Our `full.rs` approximates the grammar at `/home/estrandv/programming/tree-sitter-jdw-billboarding/grammar.js`. When in doubt, reference the grammar.

### OSC / Protocol
- **ElementConverter** (`osc.rs`) — Stateful converter with `{nodeId}` external ID scheme (format: `{index}_{name}_{counter}{elemIdx}_{counter}_{nodeId}`)
- **Instrument routing** — `InstrumentType::Sampler` → `/play_sample`, `::Synth` → `/note_on_timed`, `::Drone` → `/note_on`/`/note_modify`
- **Suffix handlers** — `@` (mod), `$` (drone on), `x` (silence), `.` (ignore), `§` (loop marker)
- **Note utils** (`note_utils.rs`) — Scale/key generation (maj/min), MIDI note resolution, frequency conversion
- **Protocol** — `/note_on_timed`, `/note_modify`, `/play_sample`, `/note_on`, `/free_notes`, `/hard_stop`, `/wipe_on_finish`, `/read_scd`, `/jdw_sc_event_trigger`
- **Bundle hierarchy** — `batch_update_queues` → `update_queue` → `timed_msg` (matches sequencer protocol)

### ElementConverter Wired into Pipeline
- `track_to_timed_packets` uses `ElementConverter::resolve_message()` per element
- `send_full_queue_update` creates per-track `ElementConverter` instances with correct `InstrumentType` and `ScaleData`
- `extract_scale_data` helper extracts `/set_scale` from billboard commands
- Drone tracks get `external_id_override` for shared drone IDs
- `to_note_mod`/`to_note_on`/`to_note_on_timed`/`to_play_sample` match Python reference (correct external_id logic, `&mut self` for `resolve_external_id`)
- Old ad-hoc OSC generation removed

### jdw-suite Integration
- **jdw-billboarding-backend is library-only** — no binary; consumed by `jdw-suite` as a git dependency
- **jdw-suite** (`/home/estrandv/programming/jdw-suite/`) provides the `jdw` CLI: `jdw play`, `setup`, `stop`, `quiet`, `terminate`
- `client.rs` in jdw-suite calls our public API (`send_full_queue_update`, `send_full_setup`, `send_full_commands`, `send_stop`, `send_silence_drones`)
- **Config** (`config.rs`) — Two-layer TOML loader (`~/.config/jdw.toml` + `./config.toml`), used by jdw-suite via `OscConfig`

### Bug Fixes
- Shuttle tokenizer: `.` in args like `amp0.5` now correctly tokenizes as `Ident("amp0") Number(".5")` instead of splitting on the dot (was peeking at the same character instead of advancing past it)
- **Args precedence** (`osc.rs:585`): `track_to_timed_packets` now resolves args in correct Python order — DEFAULT → header → element inline → track overrides. Previously, header args used `or_insert` after defaults, making them unable to override. This fixed `amp` values (e.g. blip amp=1→0.08, aPad amp=1→0.8).
- **Rest/silence filtering** (`osc.rs:67`): `is_symbol` now checks `prefix` in addition to `suffix`. Shuttle parser puts `x` in prefix (not suffix), so rest elements were falling through to `/play_sample` instead of `/empty_msg`.
- **Deterministic arg ordering** (`osc.rs:88`): `args_as_osc` now sorts HashMap keys alphabetically for consistent OSC argument order between runs.
- **OSC output verified** against Python reference for `arena.bbd`: 94/94 messages matched by external ID, 0 Python-only, 1 trivial difference (extra empty_msg).

### OSC Comparison Tools
- `parse_osc_dump.py` — Python script that parses tcpdump pcap captures into human-readable OSC. Supports `--compact` mode for one-line-per-message output.
- `normalize_rust_dump.py` — Converts `dump_osc` example output to compact format for diffing.
- Comparison workflow: `sudo tcpdump -i lo -U -w /tmp/dump.pcap udp port 13339` → `python3 run.py song.bbd` → parse with `parse_osc_dump.py --compact` → diff against `cargo run --example dump_osc`.

## Remaining Work

### NRT recording pipeline (deferred)
Port `nrt_scoring.py` (Score class) and `listener.py` (OSC response listener):
- Chronological score composition from group filters
- Preload batching (`/clear_nrt`, synthdefs, samples)
- `/nrt_record_info` with BPM, filename, end time
- Wait for `/nrt_record_finished` response

### Minor OSC discrepancies (arena.bbd verified, 94/94 match)
- 1 extra `/empty_msg` on Rust side (harmless)
- `jdw all` idempotency: kill existing scsynth/sclang before re-launch

## Tests

```
cargo test                          # 128 library tests
cargo test --test integration       # 5 integration tests (16 .bbd files)
```

All 133 tests passing.
