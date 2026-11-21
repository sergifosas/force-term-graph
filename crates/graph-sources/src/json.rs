//! JSON source. The schema is [`GraphData`] itself:
//!
//! ```json
//! { "nodes": [{"id": "rust", "label": "Rust", "category": "lang", "meta": {}}],
//!   "edges": [{"source": "rust", "target": "wezterm", "weight": 1.0}] }
//! ```
//!
//! A path of `-` reads from stdin, so the tool composes with other programs.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use graph_core::GraphData;

use crate::Source;

pub struct JsonSource {
    path: PathBuf,
}

impl JsonSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn is_stdin(&self) -> bool {
        self.path == Path::new("-")
    }

    pub fn parse(text: &str) -> anyhow::Result<GraphData> {
        Ok(serde_json::from_str(text)?)
    }
}

impl Source for JsonSource {
    fn name(&self) -> &str {
        "json"
    }

    fn load(&self) -> anyhow::Result<GraphData> {
        let text = if self.is_stdin() {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading JSON from stdin")?;
            s
        } else {
            std::fs::read_to_string(&self.path)
                .with_context(|| format!("reading {}", self.path.display()))?
        };
        Self::parse(&text).with_context(|| format!("parsing {} as graph JSON", self.path.display()))
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        if self.is_stdin() {
            Vec::new()
        } else {
            vec![self.path.clone()]
        }
    }
}
