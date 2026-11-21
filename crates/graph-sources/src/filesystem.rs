//! Filesystem source: one node per directory and file, `contains` edges from
//! each directory to its children. Respects `.gitignore` and skips hidden
//! entries by default (so `target/`, `node_modules/`, `.git/` stay out).
//!
//! Categories: `dir` for directories, otherwise the lowercase file extension
//! (or `file`), so colours group files by type.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use graph_core::{EdgeKind, EdgeSpec, GraphData, NodeSpec};
use ignore::WalkBuilder;

use crate::Source;

pub struct FsSource {
    root: PathBuf,
    /// Maximum depth below the root (`None` = unlimited).
    pub max_depth: Option<usize>,
    /// Include hidden files and ignore `.gitignore`.
    pub all: bool,
    /// Stop after this many nodes; layout cost grows quadratically.
    pub max_nodes: usize,
}

impl FsSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_depth: None,
            all: false,
            max_nodes: 2_000,
        }
    }
}

/// `.` for the root, else the `/`-separated relative path.
fn key_for(rel: &std::path::Path) -> String {
    let s = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if s.is_empty() { ".".into() } else { s }
}

impl Source for FsSource {
    fn name(&self) -> &str {
        "fs"
    }

    fn load(&self) -> Result<GraphData> {
        if !self.root.is_dir() {
            anyhow::bail!("{} is not a directory", self.root.display());
        }
        let mut walker = WalkBuilder::new(&self.root);
        walker
            .max_depth(self.max_depth)
            .hidden(!self.all)
            .git_ignore(!self.all)
            .git_global(!self.all)
            .git_exclude(!self.all)
            .ignore(!self.all)
            .parents(!self.all)
            .require_git(false)
            // Alphabetical, so the graph and any truncation are deterministic.
            .sort_by_file_path(|a, b| a.cmp(b));

        let mut data = GraphData::default();
        let mut truncated = false;
        for entry in walker.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue, // unreadable entry: skip, don't abort
            };
            if data.nodes.len() >= self.max_nodes {
                truncated = true;
                break;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path());
            let key = key_for(rel);
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            let label = if key == "." {
                std::fs::canonicalize(&self.root)
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| ".".into())
            } else {
                entry.file_name().to_string_lossy().into_owned()
            };
            let category = if is_dir {
                "dir".to_string()
            } else {
                rel.extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_else(|| "file".into())
            };
            if key != "." {
                let parent = rel.parent().map(key_for).unwrap_or_else(|| ".".into());
                data.edges.push(EdgeSpec {
                    source: parent,
                    target: key.clone(),
                    weight: 1.0,
                    kind: EdgeKind::Contains,
                });
            }
            data.nodes.push(NodeSpec {
                id: key,
                label: Some(label),
                category: Some(category),
                meta: BTreeMap::new(),
            });
        }
        if truncated {
            // Mark the root so frontends can say the view is partial.
            if let Some(root) = data.nodes.first_mut() {
                root.meta
                    .insert("truncated_at".into(), serde_json::json!(self.max_nodes));
            }
        }
        Ok(data)
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }
}
