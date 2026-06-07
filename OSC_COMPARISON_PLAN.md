# OSC Comparison: Rust vs Python

## Goal
Capture and diff the OSC messages produced by the Python (`jdw-pycompose`) and Rust (`jdw-suite`) implementations for the same `.bbd` file to find discrepancies causing audio issues.

## Approach

### 1. Capture Python OSC Output

Patch Python `jdw-pycompose` to dump OSC packets instead of (or in addition to) sending them.

**Option C: tcpdump (no code change)**
```bash
sudo tcpdump -i lo -U -w /tmp/python_dump.pcap udp port 13339
```

### 2. Capture Rust OSC Output

The `dump_queue_update()` function in `jdw-billboarding-backend` already generates text output of all packets `jdw play` would send.

**Usage:**
```rust
use jdw_billboarding_backend::{parse_billboard_file, dump_queue_update};

let bb = parse_billboard_file("arena.bbd").unwrap();
let lines = dump_queue_update(&bb);
for l in lines { println!("{}", l); }
```

Or use the example binary:
```bash
cargo run --example dump_osc path/to/arena.bbd > rust_dump.txt
```

### 3. Diff the Outputs

```bash
diff python_dump.txt rust_dump.txt | less
```

Focus on:
- Missing or extra args per message
- Different arg values (especially `amp`, `freq`, `sus`, `time`)
- Missing tracks (group filter issue — now fixed)
- Wrong instrument name routing
- Wrong OSC address

## Current Known Issues (things to check)

1. ~~Group filter not applied~~ — **FIXED**: `send_full_queue_update` now respects `>>>` filters
2. ~~Default args not passed to elements~~ — **FIXED**: `track_to_timed_packets` merges default + header + track overrides + element inline args, matching Python precedence
3. `sus` gate time — check if Python vs Rust compute differently
4. `time` arg — check if internal scheduling args leak into OSC
5. Instrument names — verify SP_/DR_ prefix stripping matches Python
6. SynthDef loading — verify all synthdefs from config paths are loaded

## Setup/Update/Play Differences

| Command | Python | Rust | Equivalent? |
|---------|--------|------|-------------|
| `setup` | setup() + configure() + beep | `send_full_setup` + `send_full_commands` | Yes |
| `update` | configure() + beep | `send_full_commands` | Yes |
| `play` | update_queue() | `send_full_queue_update` | Yes (after fix) |
