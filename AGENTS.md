# AGENTS.md — jdw-billboarding-backend

## Source Structure

```
src/
  lib.rs        # Public API: parse_billboard, parse_billboard_file
  shuttle.rs    # Hand-written Shuttle Notation parser + expander
  billboard.rs  # Mini-billboard line parser
  osc.rs        # OSC message conversion + send helpers
```

## Mini-Billboard Format

```text
# Comment — track is disabled
trackname:synthname arg=val,arg=val  (c4 d4 e4)*4:amp0.5
drums:SP_Roland808  14 14 26 32
```

- `#` prefix = disabled track
- `<name>:<synth>` — name is the sequencer alias, synth is the scsynth instrument
- `arg=val` — track-level default args (use `=` not `:`)
- Shuttle notation follows after args

## OSC Protocol

All messages sent to OSC router at `127.0.0.1:13339`:

- Queue updates: `/bundle_info` + timed bundles for sequencer
- Stop: `/hard_stop`, `/wipe_on_finish`
- Setup: `/read_scd` for each synth
