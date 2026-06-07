pub mod shuttle;
pub mod billboard;
pub mod osc;

pub use billboard::{Billboard, Track};
pub use osc::OscConfig;

/// Parse a mini-billboard file from a string.
pub fn parse_billboard(source: &str) -> Result<Billboard, String> {
    billboard::parse(source)
}

/// Read and parse a mini-billboard file from the given path.
pub fn parse_billboard_file(path: &str) -> Result<Billboard, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path, e))?;
    parse_billboard(&content)
}
