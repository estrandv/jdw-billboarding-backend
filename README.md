# jdw-billboarding-backend

Rust-native billboard notation parser and OSC converter for JackDAW.

This is the Rust port of the Python `jdw-billboarding-lib`, starting with a
mini-billboard subset for MVP and converging toward full billboard support.

## Status: MVP

Currently implements a minimal subset of the billboard format:
- Simple `<trackname>:<synthname> <shuttle>` notation
- Comment support (`#` prefix disables track)
- Synth-level args (`key=val`)
- Shuttle Notation: atomic notes, sections, alternations, repeats, args

## Usage

```rust
use jdw_billboarding_backend;

let bb = jdw_billboarding_backend::parse_billboard_file("song.txt")?;
```
