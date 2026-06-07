# OSC Comparison: Rust vs Python — DONE

## Status: All 3 phases verified (setup, update, play) for arena.bbd

Play phase: 94/94 messages match by external ID.

## Automated Tool

```bash
cd jdw-billboarding-backend
./capture_compare.sh [song.bbd]
```

Single command, non-interactive:
1. `sudo -v` → validates sudo once
2. Starts tcpdump on port 13339 (Python osc-router)
3. Runs `python3 run.py --setup`, `--update`, then play in sequence
4. OSC sentinel markers split the pcap into per-phase files
5. Dumps Rust equivalents for all 3 phases
6. Prints comparison table + message type breakdowns

## Historical Issues (all resolved)

1. ~~Group filter not applied~~ — FIXED: `send_full_queue_update` respects `>>>` filters
2. ~~Default args not passed to elements~~ — FIXED: args precedence matches Python
3. ~~Rest/silence `x` sent as /play_sample~~ — FIXED: `is_symbol` checks prefix
4. ~~HashMap iteration non-deterministic~~ — FIXED: sorted keys
5. ~~Effect args missing section header overrides (out, relT, etc.)~~ — FIXED: merged in build_billboard
6. ~~Commands (routers) sent after effects~~ — FIXED: correct SC bus order
7. ~~Router in/out args as Int~~ — FIXED: Float for jdw-sc /note_on handler
8. ~~Samples not loaded~~ — FIXED: sample_loader.rs
9. ~~Drones not created~~ — FIXED: send_drones_create
10. ~~/create_router, /create_effect not translated~~ — FIXED: command translation
