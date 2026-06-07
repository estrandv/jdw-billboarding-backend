pub mod shuttle;
pub mod billboard;
pub mod full;
pub mod macros;
pub mod config;
pub mod note_utils;
pub mod osc;
pub mod synthdefs;

pub use full::{
    Billboard, BillboardCommand, CommandContext, EffectDefinition, SynthHeader, SynthSection,
    TrackDefinition,
};
pub use osc::{dump_queue_update, dump_setup, dump_commands, OscConfig};
pub use synthdefs::{load_synthdefs, SynthDefMessage};
pub use macros::{compile_macros, load_and_expand};

/// Parse a billboard file from a string (raw, no macro expansion).
pub fn parse_billboard(source: &str) -> Billboard {
    full::parse(source)
}

/// Parse from source with macro expansion.
pub fn parse_billboard_with_macros(source: &str, supplied_defs: &[String]) -> Result<Billboard, String> {
    let expanded = macros::compile_macros(source, supplied_defs)?;
    Ok(full::parse(&expanded))
}

/// Read a billboard file, expand macros (including sibling `common_macros.txt`), and parse.
pub fn parse_billboard_file(path: &str) -> Result<Billboard, String> {
    let expanded = macros::load_and_expand(path)?;
    Ok(full::parse(&expanded))
}
