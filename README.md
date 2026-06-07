# jdw-billboarding-backend

Rust-native billboard notation parser and OSC converter for JackDAW.

Based on a combination of `jdw-billboarding-lib` (base billboarding language parsing MVP) and `jdw-pycompose` (Extemsions like command targets and macro support). 

Key usage is as an imported library in `jdw-suite`, which manages the ecosystem in which this parsing can be used for music making. 

## Status: Faulty

`jdw-pycompose` setup/update/send commands produce different output than what we currently have here for an example song file. 

See: OSC_COMPARISON_PLAN.md. 

## Usage

```rust
use jdw_billboarding_backend;

let bb = jdw_billboarding_backend::parse_billboard_file("song.txt")?;
```
