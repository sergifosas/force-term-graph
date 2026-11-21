//! Data sources. Every source produces the same [`GraphData`], so they can be
//! combined without the engine knowing where data came from.

use std::path::PathBuf;

use graph_core::GraphData;

pub mod filesystem;
pub mod json;
pub mod markdown;

pub use filesystem::FsSource;
pub use json::JsonSource;
pub use markdown::MarkdownSource;

pub trait Source {
    /// Short name, also used as the key namespace (`json`, `md`, `fs`).
    fn name(&self) -> &str;

    fn load(&self) -> anyhow::Result<GraphData>;

    /// Paths to watch for live reload (Phase 4).
    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Loads every source and merges them, namespacing keys by source name.
pub fn load_all(sources: &[Box<dyn Source>]) -> anyhow::Result<graph_core::Graph> {
    let mut graph = graph_core::Graph::new();
    for s in sources {
        let data = s.load()?;
        data.namespaced(s.name()).apply_to(&mut graph);
    }
    Ok(graph)
}
