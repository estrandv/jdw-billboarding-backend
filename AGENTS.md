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

### 🔴 BLOCKING: `Preloaded nrt packets: 0` → `/nrt_done` never arrives

**This is NOT a timeout problem.** Timeouts are an error scenario. The ONLY acceptable
outcome is receiving `/nrt_done` from sclang. The `set_read_timeout` fix (57fc9c2)
in jdw-sc only makes the hang visible by unblocking `recv_from` — it does NOT fix
why `/nrt_done` never arrives.

**Pattern**: Tracks where jdw-sc logs `Preloaded nrt packets: 0` are the ones
that hang. The main bundle has plenty of notes (168-640 timed messages). The SCD
file is generated correctly (brackets balanced, verified). jdw-sc sends it to sclang
via `/read_scd_file`. sclang appears to render it (CPU activity, `nextOSCPacket`
logs). But `/nrt_done` never comes back.

**What we know for certain:**
1. SCD files for hanging tracks are syntactically valid (verified bracket balance)
2. `send_to_sclang` is called for every track (line 371)
3. `await_internal_response("/nrt_done")` is called for every track (line 382)
4. The timeout was previously broken (commented-out `set_read_timeout`), meaning
   the hang was truly infinite, not just slow
5. Sample filtering reduced SCD size 10x — didn't fix it
6. Increasing timeouts 30s→120s→300s didn't fix it — not a slowness issue

**Theories (ordered by likelihood):**
1. **Score row causing sclang runtime error/hang**: `o.sendMsg` is the same for
   every SCD (it's in the template, not per-track). Since most tracks work, the
   problem is in the failing track's specific score rows. Something in those rows
   causes sclang to abort or hang before reaching the `action:` callback. Compare
   the Python SCD score rows with Rust's for the same track — find the difference.
2. **Empty preloads cause a different code path in jdw-sc**: Despite tracing
   showing the same lines execute, maybe `self.nrt_preloads.is_empty()` triggers
   different behavior elsewhere (e.g., `nrt_sample_pack_dict` state).
3. **sclang parse failure silently ignored**: The SCD might parse but have a
   runtime error that prevents the action from firing, with no visible error.

**Investigation plan (tomorrow):**
1. **Compare Python SCD vs Rust SCD for the same track** — find what's different
   in the generated SuperCollider script that would cause the action to not fire.
   Python works, Rust doesn't. Diff them.
2. **Check if Python also has empty-preload tracks** — if Python's tracks also
   have 0 preloads but work fine, the issue is definitely in the SCD generation.
3. **Test with a minimal track** — create a 1-note .bbd file with no commands/effects.
   Does NRT work for it? If yes, the issue is specific to certain arena.bbd tracks.
4. **Add /nrt_done logging on the sclang side** — verify whether sclang even
   attempts to send it. The `nextOSCPacket` logs suggest sclang IS running the
   score, but maybe the action never fires.

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
