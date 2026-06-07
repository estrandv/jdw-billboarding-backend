/// Configuration for jdw-suite.
///
/// Loaded from two-layer TOML:
///   1. Central: `~/.config/jdw.toml` (or `$JDW_CONFIG`)
///   2. Local:   `./config.toml` (overrides central)

use std::path::Path;
use std::fs;

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct JdwConfig {
    pub router_host: String,
    pub router_port: u16,
    pub delay_inter_message: f64,
    pub delay_configure: f64,
    pub delay_update: f64,
    pub delay_quiet: f64,
    pub delay_nrt_preload: f64,
    pub bbd_root: Option<String>,
}

impl Default for JdwConfig {
    fn default() -> Self {
        JdwConfig {
            router_host: "127.0.0.1".to_string(),
            router_port: 13339,
            delay_inter_message: 0.005,
            delay_configure: 0.001,
            delay_update: 0.005,
            delay_quiet: 0.005,
            delay_nrt_preload: 0.005,
            bbd_root: None,
        }
    }
}

impl JdwConfig {
    /// Load config from central + optional local TOML files.
    pub fn load(local_path: Option<&str>) -> Self {
        let mut cfg = JdwConfig::default();

        // Layer 1: central config
        if let Some(central) = load_toml_file(&central_config_path()) {
            cfg.merge(&central);
        }

        // Layer 2: local config
        if let Some(local) = local_path {
            if let Some(toml) = load_toml_file(local) {
                cfg.merge(&toml);
            }
        }

        cfg
    }

    fn merge(&mut self, data: &toml::Value) {
        if let Some(py) = data.get("pycompose").and_then(|v| v.as_table()) {
            if let Some(v) = py.get("router_host").and_then(|v| v.as_str()) {
                self.router_host = v.to_string();
            }
            if let Some(v) = py.get("router_port").and_then(|v| v.as_integer()) {
                self.router_port = v as u16;
            }
            if let Some(v) = py.get("bbd_root").and_then(|v| v.as_str()) {
                self.bbd_root = Some(v.to_string());
            }
            if let Some(delays) = py.get("delays").and_then(|v| v.as_table()) {
                if let Some(v) = delays.get("inter_message").and_then(|v| v.as_float()) {
                    self.delay_inter_message = v;
                }
                if let Some(v) = delays.get("configure").and_then(|v| v.as_float()) {
                    self.delay_configure = v;
                }
                if let Some(v) = delays.get("update").and_then(|v| v.as_float()) {
                    self.delay_update = v;
                }
                if let Some(v) = delays.get("quiet").and_then(|v| v.as_float()) {
                    self.delay_quiet = v;
                }
                if let Some(v) = delays.get("nrt_preload").and_then(|v| v.as_float()) {
                    self.delay_nrt_preload = v;
                }
            }
        }
    }

    /// Create an OscConfig from this config.
    pub fn to_osc_config(&self) -> crate::osc::OscConfig {
        crate::osc::OscConfig {
            router_addr: format!("{}:{}", self.router_host, self.router_port),
            ..Default::default()
        }
    }
}

fn central_config_path() -> String {
    if let Ok(path) = std::env::var("JDW_CONFIG") {
        if Path::new(&path).exists() {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.config/jdw.toml", home)
}

fn load_toml_file(path: &str) -> Option<toml::Value> {
    let content = fs::read_to_string(path).ok()?;
    content.parse::<toml::Value>().ok()
}
