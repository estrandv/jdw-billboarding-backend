# NRT Overview

Non-real-time recording — renders arena.bbd (any .bbd) to individual WAV files
via SuperCollider's NRT server. Port of Python's `jdw-pycompose/billboard_running.py`
and `jdw-billboarding-lib/jdw_billboarding/lib/nrt_scoring.py`.

## Pipeline

```
.bbd file
  → macros::load_and_expand     expand $macros
  → full::parse                 classify, group, build Billboard
  → osc::get_nrt_record_bundles build Score, extend groups, produce bundles
  → client.rs                   send preload_msgs → preload_bundles → nrt_bundle
  → jdw-sc                      receive, build SCD, run scsynth NRT
  → ~/jdw_output/track_*.wav    individual track WAVs
```

## Score + extend_groups

The `Score` (`src/score.rs`) composes a timeline from track sources using group
filters (`>>> group1 group2` lines in the .bbd file).

```
score.add_source(track_name, group, durations)   ← raw element durations per track

for filter_set in group_filters:                  ← e.g. ["apad", "moog", "blip"]
    score.extend_groups(filter_set)
        1. find longest matching source → extend it (push all elements)
        2. goal_time = sum of longest track
        3. for all other tracks:
            - matching: repeat until near goal_time, then pad
            - non-matching: pad to goal_time

score.unpack_timed_entries()   → (BigDecimal, Option<source_index>) per track
```

Uses `BigDecimal` for all beat arithmetic (matching Python's `Decimal`) to
avoid floating-point precision issues across 12 filter set iterations.

## Bundle Format

Three phases sent sequentially per track:

### 1. preload_messages (individual UDP sends)
- `/clear_nrt` — reset jdw-sc state
- `/create_synthdef` — synthdefs needed by this track (instrument + router + sampler+ + effects)
- `/load_sample` — samples needed by this track (filtered by category + tone_index)

### 2. preload_bundles (batched OSC bundles)
Combined setup + timed note packets, split into chunks of 100 to stay under
UDP size limits. Each chunk wrapped in a `/bundle_info "nrt_preload"` bundle
with individual `/timed_msg_info [beat_duration]` entries.

Setup packets (at beat 0):
- Commands converted: `/create_router` → `/note_on "router"`, `/create_effect` → `/note_on`
- Drones from all drone sections → `/note_on` with amp=0
- Effects for this track's section → `/note_on`

Timed packets:
- Note playback: `/s_new` (synths), `/play_sample` (samplers)
- Silence padding: `/empty_message`

### 3. nrt_bundle (single metadata-only OSC bundle)
```
/bundle_info "nrt_record"
/nrt_record_info [bpm, output_path, end_time]
Bundle {}   ← empty inner bundle (jdw-sc parser expects 3 children)
```

## Diff Tool

Compare old method (Python) SCDs against Rust output:

```bash
# Batch compare all 23 pairs (PASS 1: structural, PASS 2: detail with --full)
python3 scripts/compare_scds.py

# Single pair with full detail
python3 scripts/compare_scds.py --full old.scd new.scd
```

PASS 1 checks: synthdef set, entry counts, command breakdown, duration.
Any difference is substantial. PASS 2 shows entry times and `/s_new` args.

## Lessons Learned

### Router synthdef must always be loaded
`/create_router` commands produce `/note_on "router"` entries. If the `router`
synthdef isn't in `/d_recv`, scsynth can't create router synths → no audio
routing to bus 0 → most tracks silent. Always include `router` in needed synthdefs.

### Preload bundles need batching
Large sampler tracks can have 600+ timed packets. A single OSC bundle with all
of them exceeds UDP packet limits. Split into chunks (100 packets per bundle).

### f64 → BigDecimal conversion is critical
Even tiny floating-point errors compound across 12 `extend_groups` iterations.
Use `BigDecimal::from_str(&format!("{}", f))` which preserves needed precision.
Previously the extend_groups `goal_time` was consistently -1 beat due to a
shuttle notation parsing difference, not a precision issue.

### SCD comparison must check synthdefs
Entry counts alone are insufficient. Old vs new diff must verify that the
same set of synthdefs are loaded via `/d_recv` at beat 0. A missing `router`
synthdef silently breaks all audio routing.

## Tests

```bash
cargo test                              # 159 library tests
cargo test --test integration           # 5 integration tests
```
