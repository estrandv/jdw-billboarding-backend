# NRT SCD Diff Plan

Comparison target: `old_method` (Python, correct) → Rust `~/jdw_output/` (current).

## Reference Data

| Dir | Source | Files |
|-----|--------|-------|
| `~/tmp/nrt_bug_export/old_method/` | Python | 24 SCDs + WAVs, correct |
| `~/jdw_output/` | Rust | 23 SCDs + WAVs, current |

## Diff Tool

```bash
# Batch compare all 23 pairs (PASS 1: structural, PASS 2: detail with --full)
python3 scripts/compare_scds.py

# Single pair
python3 scripts/compare_scds.py old.scd new.scd
python3 scripts/compare_scds.py --full old.scd new.scd   # include detail
```

PASS 1 checks: synthdef set, entry counts, command breakdown, duration — any diff is substantial.
PASS 2 (--full): entry times, /s_new arg values.

## Name Mapping

| old_method | Rust |
|---|---|
| track_aPad_apad_0 | track_aPad_8_0 |
| track_blip_chorus_0 | track_blip_2_0 |
| track_eBass_cbass_1 | track_cbass |
| track_eBass_dbass_0 | track_eBass_4_0 |
| track_EMU_SP12_drumx_0 | track_EMU_SP12_13_0 |
| track_EMU_SP12_drumi_0 | track_EMU_SP12_3_0 |
| track_EMU_SP12_drumi_1 | track_EMU_SP12_13_1 |
| track_EMU_SP12_drumi_2 | track_EMU_SP12_13_2 |
| track_EMU_SP12_drumi_3 | track_EMU_SP12_13_3 |
| track_EMU_SP12_cdrum_1 | track_cdrum |
| track_experimental_brah_0 | track_experimental_1_0 |
| track_FMRhodes_rat_0 | track_FMRhodes_7_0 |
| track_FMRhodes_rhodesii_0 | track_FMRhodes_5_0 |
| track_gritBass_gritBass_0 | track_gritBass_0_0 |
| track_gritBass_gritBass_1 | track_gritBass_0_1 |
| track_karp_blip_0 | track_karp_10_0 |
| track_karp_blipii_0 | track_karp_9_0 |
| track_moogBass_moog_0 | track_moogBass_6_0 |
| track_pluck_pluck_3 | track_pluck_11_3 |
| track_pluck_vocbridge_0 | track_vocbridge |
| track_pluck_vocchorus_2 | track_vocchorus |
| track_pluck_vocverse_1 | track_vocverse |
| track_Roland808_drum_0 | track_Roland808_12_0 |

## Status — 2026-06-08

### Audio: 16/23 audible, 7 silent

| Audible (16) | Silent (7) | Reason |
|---|---|---|
| aPad, blip, cbass, cdrum | EMU_SP12_13_0 | 0 /s_new (sampler, extend_groups) |
| eBass, EMU_SP12_13_1-3 | experimental_1_0 | 10 /s_new (routers only) |
| EMU_SP12_3_0, FMRhodes_7_0 | FMRhodes_5_0 | 12 /s_new (routers+effects only) |
| karp_10_0, karp_9_0 | gritBass_0_0, gritBass_0_1 | 10 /s_new (routers only) |
| moogBass, vocbridge | pluck_11_3 | 10 /s_new (routers only) |
| vocchorus, vocverse | Roland808_12_0 | 10 /s_new (routers only) |

### SCD Diff: all 23 PASS 1 match except extra `samplerALT` (harmless)

`samplerALT` is an additional synthdef Rust always includes; old_method Python doesn't.
It's not referenced by any `/s_new` entries — jdw-sc uses it for sample playback conversion.

### Fixes Applied

- **Router entries in SCD**: `/create_router` commands now converted to `/note_on "router"` in preload bundle (matching Python)
- **Drones in SCD**: ALL drone sections included as `/note_on` at beat 0 (matching Python)
- **Command conversion**: `/create_effect` converted to `/note_on` + `/note_modify` (matching `send_full_commands`)
- **Synthdef filtering**: `router` always loaded, `sampler` loaded for sampler tracks
- **Score::get_end_time()**: global max across all tracks (matching Python)
- **Global end_beat**: all tracks use same recording duration (matching Python)
- **BigDecimal migration**: all beat arithmetic in `score.rs` uses `BigDecimal`

### Remaining: extend_groups timing divergence

The 7 silent tracks have correct SCD structure (synthdefs, routers, effects) but 0 actual
synth notes. The `extend_groups` `goal_time` diverges between Python (`Decimal`) and
Rust (`BigDecimal`) across 12 filter iterations, causing these tracks to never get
their source elements extended — they accumulate only silence/route setup.

**Fix**: Add step-by-step debug logging to both Python and Rust `extend_groups`,
trace `goal_time` across all 12 filter sets, find where it diverges.
