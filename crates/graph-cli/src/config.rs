//! Layered configuration: embedded defaults < user config file < `--config`.
//! Layers are merged key-by-key, so user files only need the keys they change.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use graph_core::{PhysicsParams, Style};
use serde::Deserialize;

const DEFAULT_TOML: &str = include_str!("../../../config/default.toml");

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub physics: PhysicsParams,
    pub style: Style,
    pub render: RenderConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderConfig {
    /// Cap on the full-quality frame's longest side, in pixels.
    pub max_dimension: u32,
    /// Frame-rate cap while animating.
    pub fps: f32,
    /// Resolution factor while animating/interacting.
    pub interactive_scale: f32,
    /// "auto", "kitty" or "iterm2".
    pub protocol: String,
    /// "auto" (pane in an image-capable terminal, else window), "pane" or
    /// "window".
    pub frontend: String,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            max_dimension: 4096,
            fps: 60.0,
            interactive_scale: 1.0,
            protocol: "auto".into(),
            frontend: "auto".into(),
        }
    }
}

pub fn user_config_path() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("graph").join("config.toml"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("graph").join("config.toml"))
    }
}

pub fn load(explicit: Option<&Path>) -> Result<Config> {
    let mut merged: toml::Table = DEFAULT_TOML
        .parse()
        .context("embedded default.toml is invalid")?;
    if let Some(p) = user_config_path().filter(|p| p.is_file()) {
        merge(&mut merged, read_table(&p)?);
    }
    if let Some(p) = explicit {
        merge(&mut merged, read_table(p)?);
    }
    Config::deserialize(toml::Value::Table(merged)).context("invalid configuration")
}

fn read_table(path: &Path) -> Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parsing {}", path.display()))
}

/// Deep-merges `over` into `base`: tables merge recursively, other values replace.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_match_code_defaults() {
        let c = load(None).unwrap();
        assert_eq!(c.physics, PhysicsParams::default());
        assert_eq!(c.style, Style::default());
        assert_eq!(c.render, RenderConfig::default());
    }

    #[test]
    fn partial_override_keeps_other_keys() {
        let mut base: toml::Table = DEFAULT_TOML.parse().unwrap();
        merge(&mut base, "[physics]\ngravity = 0.5".parse().unwrap());
        let c = Config::deserialize(toml::Value::Table(base)).unwrap();
        assert_eq!(c.physics.gravity, 0.5);
        assert_eq!(c.physics.repulsion, PhysicsParams::default().repulsion);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let mut base: toml::Table = DEFAULT_TOML.parse().unwrap();
        merge(&mut base, "[physics]\ngravvity = 0.5".parse().unwrap());
        assert!(Config::deserialize(toml::Value::Table(base)).is_err());
    }
}
