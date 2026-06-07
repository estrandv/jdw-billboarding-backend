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
```

## Pipeline

```
.bbd file
  → macros::load_and_expand  (expand $macros, load common_macros.txt)
  → full::parse              (classify lines, group sections, build Billboard)
  → osc::send_*              (setup/update/play/nrt)
```

## Status — 2026-06-07

### Fully Working
- **Live play**: setup/update/play verified against Python for arena.bbd (94/94 messages match)
- **OSC comparison**: `capture_compare.sh` automates Python vs Rust diffing
- **All 159 tests passing** (lib) + 5 integration tests

### NRT — Mostly Working (with one known issue)

`jdw nrt arena.bbd` successfully renders ~20 of 23 tracks. Output WAV files match Python's.

Changes made today:
- **Score class** (`score.rs`): timeline composition, group filter walking, silence padding
- **NRT bundles** (`osc.rs`): `nrt_preload`/`nrt_record` bundle format, `get_nrt_record_bundles`
- **Listener** (`listener.rs`): background OSC UDP server, `wait_for_nrt()`
- **Sample filtering**: sampler tracks only load their own pack's samples (was all 173)
- **Synthdef filtering**: only needed synthdefs per track (was all 44)
- **Macro expansion**: fixed regex space bug in `$hpf(osc, 500)` args
- **SCD comments**: strip trailing `//` comments from between synthdefs
- **jdw-sc fixes**: `/nrt_record_finished` subscription, output dir creation, `/empty_message` no-op, `set_read_timeout` for `await_internal_response`

### Known Issue: `Preloaded nrt packets: 0` → tracks hang

**Observation**: Tracks where jdw-sc logs `Preloaded nrt packets: 0` sometimes hang.
The NRT CLI says "Timed out waiting for NRT completion" but jdw-sc eventually sends
`/nrt_record_finished "FAILURE"`. The track's main bundle DOES have notes (e.g., 168-640 timed messages).

**Arguments FOR the theory that empty preloads cause missing `/nrt_done`:**
1. The pattern is consistent — always the first track with `Preloaded nrt packets: 0` that hangs
2. Other tracks with non-empty preloads complete quickly
3. Increasing timeouts (30s→120s→300s) doesn't fix it — it's not a timing issue
4. jdw-sc's `await_internal_response` had a bug (commented-out `set_read_timeout`) that
   would cause infinite blocking if no intermediate messages arrive — this was fixed
   but MAY not have been the root cause

**Arguments AGAINST:**
1. Many tracks with `Preloaded nrt packets: 0` DO succeed (e.g., cdrum, 168 notes)
2. The main bundle carries 168-640 notes regardless of preload emptiness
3. jdw-sc merges `nrt_preloads` (empty) + bundle messages into the score — both paths
   lead to the same SCD generation and `await_internal_response` call
4. The `nrt_record` handler at line 274 runs the same code regardless of preload count

**What needs investigation tomorrow:**
1. Trace jdw-sc's `nrt_record` handler line-by-line when `nrt_preloads` is empty:
   - Does it still call `create_nrt_script`? (line 345) YES
   - Does it still call `send_to_sclang`? (line 371) YES  
   - Does it still call `await_internal_response`? (line 382) YES
   - Does the generated SCD file have valid bracket balance? YES (verified)
2. Check if the `set_read_timeout` fix (57fc9c2) actually resolves the hang
3. Compare Python's NRT behavior: does Python also have "empty preload" tracks?
4. Check if `/nrt_done` is being sent by sclang but not received by jdw-sc
5. Check if the `server_osc_socket_name` config ("o") is correct for NRT mode

### NRT Remaining Work
- **Phase 5**: OSC comparison dump — verify Rust NRT output matches Python's
- Filter `/load_sample` to only samples actually referenced by the track's notes
  (Python's `_filter_used_samples` checks element messages, not just instrument pack)
- NRT regression test suite

## Tests

```
cargo test                          # 159 library tests
cargo test --test integration       # 5 integration tests
```
All 164 tests passing.
