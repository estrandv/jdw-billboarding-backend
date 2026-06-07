# NRT Recording Migration Plan

## Status: PLANNING — all tasks pending

## Overview

Port `nrt_scoring.py` (Score class) and `nrt_record()` from jdw-pycompose to Rust.
Unlike live play which sends to the sequencer (port 14441), NRT sends directly to
jdw-sc (port 13331/13339) with a different bundle format and a listener to wait
for completion.

## Code Reuse Between NRT and Live Play

The billboard pipeline is identical up to the branch point:

```
.bbd → macros → parse → Billboard
                              │
                ┌─────────────┴─────────────┐
                ▼                           ▼
         Live Play                      NRT Record
    send_full_queue_update            Score + nrt bundles
    → sequencer (14441)               → jdw-sc (13331/13339)
```

**What's shared:**
- SynthDef loading (`load_synthdefs`)
- Sample loading (`get_default_samples`)
- Billboard parsing (`full::parse`)
- Effect/drone creation (`send_effects_create`, `send_drones_create`)
- Command translation (`send_full_commands`)
- ElementConverter and external ID scheme

**What's NRT-specific:**
- Score class — timeline composition with group filter ordering
- NRT bundle format (`nrt_record` / `nrt_preload` tags instead of `update_queue`)
- Preload batching (`/clear_nrt`, filtered synthdefs/samples, timed setup messages)
- Listener — waits for `/nrt_record_finished` from jdw-sc

## Phase 1 — Score Class (src/score.rs)

The `Score` class composes tracks into a chronological timeline based on group
filters. It pads tracks to equal length so they render simultaneously.

### Data Types

```rust
/// A source track with its elements and group affiliation.
struct TrackSource {
    elements: Vec<ElementMessage>,
    group_name: String,
}

/// A single entry in the score timeline.
struct ScoreMessage {
    message: Option<ElementMessage>,  // None = silence padding
    time: f64,                         // beat position
}

/// The Score composes tracks into a synchronized timeline.
struct Score {
    track_sources: HashMap<String, TrackSource>,
    tracks: HashMap<String, Vec<ScoreMessage>>,
}
```

### Stage 1a — add_source (TEST FIRST)

Create a Score, add a source, verify elements are stored.

**Test:** `test_score_add_source` — add two tracks with different lengths,
verify sources are stored correctly.

### Stage 1b — extend_groups (TEST FIRST)

Walk group filters in order. For each filter set:
1. Find the longest matching track source → extend it fully into the score
2. Determine `goal_time` from the extended track's end time
3. Extend other matching tracks by their source length until remaining time < source length
4. Pad non-matching tracks with silence to match `goal_time`

**Test:** `test_score_extend_groups_single` — one filter, two tracks of same length
→ both should be fully extended, same total duration.

**Test:** `test_score_extend_groups_padding` — one filter, two tracks where one is
shorter → shorter track should be padded with silence.

**Test:** `test_score_extend_groups_multi_filter` — two filters, tracks from
filter 1 and filter 2 should be composed in order, with padding.

### Stage 1c — unpack_timed_tracks (TEST FIRST)

Convert score entries to timed OSC bundles for the NRT sequencer.
- Adjacent silence entries are compressed into preceding note durations
- `None` elements become `/empty_message` bundles

**Test:** `test_score_unpack_silence_compression` — notes with silence between
them should have extended durations.

**Test:** `test_score_unpack_empty_messages` — pure silence entries should
produce `/empty_message` bundles.

## Phase 2 — NRT Bundle Builders (src/osc.rs)

### Bundle Formats

#### `create_nrt_preload_bundle`
```
[/bundle_info, "nrt_preload"]
[untagged inner bundle containing timed setup messages]
```

#### `create_nrt_record_bundle`
```
[/bundle_info, "nrt_record"]
[/nrt_record_info, <bpm: float>, <filename: str>, <end_beat: float>]
[untagged inner bundle containing all timed track messages]
```

### Stage 2a — create_nrt_preload_bundle (TEST FIRST)

**Test:** `test_nrt_preload_bundle_structure` — verify the bundle has
`/bundle_info "nrt_preload"` as first element and a nested bundle as second.

### Stage 2b — create_nrt_record_bundle (TEST FIRST)

**Test:** `test_nrt_record_bundle_structure` — verify the bundle has:
1. `/bundle_info "nrt_record"`
2. `/nrt_record_info [bpm, filename, end_beat]` with correct typed args
3. A nested bundle containing timed messages

### Stage 2c — get_nrt_record_bundles (TEST FIRST)

Full bundle construction for a billboard:
1. Create Score, add all track sources, extend by group filters
2. Collect commands (ALL + UPDATE context) as t=0.0 timed messages
3. Collect effect/drone create as t=0.0 timed messages
4. Build preload messages: `/clear_nrt`, needed synthdefs, needed samples
5. Build preload bundles from timed setup messages (batched by 10)
6. Build main nrt_record bundle with score messages

**Test:** `test_nrt_record_bundles_basic` — parse a minimal billboard with
one synth track, verify bundle structure, preload messages, and track messages.

**Test:** `test_nrt_record_bundles_with_bpm` — verify `/nrt_record_info` has
the BPM from a `/set_bpm` command.

**Test:** `test_nrt_record_bundles_filtered_samples` — verify only samples
used by the track are preloaded (not all samples).

## Phase 3 — Listener (src/listener.rs)

Port Python's `Listener` class — a blocking wait for NRT completion.

**This is new infrastructure.** The Rust codebase currently only *sends* OSC;
this requires *receiving* it. The Listener spawns a background UDP OSC server on port
13456 that subscribes to `/nrt_record_finished` responses from jdw-sc (routed
back via the osc-router at 13339). After the NRT render completes, jdw-sc sends
`/nrt_record_finished "SUCCESS" <filename>` which the Listener receives and
unblocks `wait_for()`.

Key design points:
- Uses a background thread with a blocking UDP socket (not async)
- Only needs to handle ONE response type (`/nrt_record_finished`)
- Must time out (5s default) to prevent hanging indefinitely
- The OSC router must be configured to forward `/nrt_record_finished` to
  the listener's port (via `/subscribe` message sent during setup)

### Stage 3a — Listener structure (TEST FIRST)

**Test:** `test_listener_start_stop` — create listener, verify it binds to
a port, can be stopped.

### Stage 3b — wait_for (TEST FIRST)

Block until a specific OSC address is received, with timeout.

**Test:** `test_listener_wait_for_timeout` — waiting for an address that
never arrives should timeout after N seconds.

**Test:** `test_listener_wait_for_receives` — sending the expected address
should unblock wait_for.

## Phase 4 — Wire into jdw-suite

### Stage 4a — NRT entry point

Add `jdw nrt <file> <output>` command:
1. Parse billboard
2. Load synthdefs + samples
3. Get NRT bundles via `get_nrt_record_bundles`
4. For each bundle:
   a. Send preload messages one-by-one
   b. Send preload bundle batches
   c. Send main nrt_record bundle
   d. Wait for `/nrt_record_finished`
5. Report results

### Stage 4b — Config

Add NRT config keys to `JdwConfig`:
- `nrt_output_dir` (default: `~/jdw_output/`)
- `nrt_listener_port` (default: 13456)

## Key Differences to Remember

| Aspect | Live Play | NRT |
|--------|-----------|-----|
| Bundle tags | `update_queue`, `batch_update_queues` | `nrt_record`, `nrt_preload` |
| Target | Sequencer (14441) | jdw-sc (13331 via router 13339) |
| SynthDef loading | Once during setup | Per-track, filtered to needed synths |
| Sample loading | All samples during setup | Per-track, filtered to used samples |
| Timing | Real-time sequencer loop | Beat-based, converted to seconds in jdw-sc |
| Completion | None (continuous) | Listener waits for `/nrt_record_finished` |
| External IDs | Same | Same (both use `{nodeId}`) |
| Effects/drones | Created during setup | Sent as t=0 timed messages in preload |

## Phase 5 — OSC Comparison and Regression Tests

Before declaring NRT complete, verify the Rust output matches Python's NRT
output for arena.bbd (same approach as the live play comparison).

### Stage 5a — Capture Python NRT dump

Extend `capture_compare.sh` with a `--nrt` phase:
```bash
./capture_compare.sh --nrt arena.bbd
```
Captures `python3 run.py --nrt arena.bbd` on port 13339.

### Stage 5b — Rust NRT dump

Extend `dump_osc` example with `--phase nrt` that calls `dump_nrt_bundles()`.

### Stage 5c — Comparison

Same diff workflow as play phase:
- Compare bundle types (`nrt_preload`, `nrt_record`)
- Compare `/nrt_record_info` args (BPM, filename, end_beat)
- Compare timed message counts per track
- Compare external IDs and arg values

### Stage 5d — Regression test suite

Based on the verified comparison, add high-level integration tests:
- `test_nrt_arena_bundle_count` — correct number of tracks x bundles
- `test_nrt_arena_bpm` — BPM from command propagated correctly
- `test_nrt_arena_preload_messages` — right synthdefs and samples preloaded
- `test_nrt_arena_timed_messages` — note counts match Python per track

## Implementation Order (updated)

1. **Phase 1 (Score)** — pure logic, no OSC, easy to test
2. **Phase 2 (NRT Bundles)** — builds on Score + existing OSC infrastructure
3. **Phase 3 (Listener)** — blocking UDP listener
4. **Phase 4 (jdw-suite)** — CLI integration
5. **Phase 5 (Comparison)** — capture Python NRT, diff, regression tests

Each phase has test-first stages. No stage should be marked complete until its
tests pass.
