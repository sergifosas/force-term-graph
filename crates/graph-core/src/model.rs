//! What the graph *is*: nodes, edges, adjacency. Positions live in
//! [`crate::layout::Layout`], so the model can be shared, diffed and merged
//! without touching the simulation.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Dense node index, stable for the lifetime of a [`Graph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// A reference from one item to another (wikilink, JSON relation).
    #[default]
    Link,
    /// Containment (folder → file).
    Contains,
    /// Item → tag.
    Tag,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeData {
    /// Namespaced unique key, e.g. `md:notes/rust.md`, `fs:/src`, `json:42`.
    pub key: String,
    pub label: String,
    pub category: String,
    pub meta: BTreeMap<String, serde_json::Value>,
    /// Referenced but never defined (an unresolved link).
    pub ghost: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeData {
    pub source: NodeId,
    pub target: NodeId,
    pub weight: f32,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    nodes: Vec<NodeData>,
    edges: Vec<EdgeData>,
    index: HashMap<String, NodeId>,
    adjacency: Vec<Vec<NodeId>>,
    edge_set: HashSet<(NodeId, NodeId)>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a defined node, or updates and returns the existing one with the
    /// same key. A ghost becomes a real node once it is defined.
    pub fn add_node(&mut self, key: &str, label: &str, category: &str) -> NodeId {
        if let Some(&id) = self.index.get(key) {
            let n = &mut self.nodes[id.index()];
            n.label = label.to_string();
            n.category = category.to_string();
            n.ghost = false;
            return id;
        }
        self.push_node(NodeData {
            key: key.to_string(),
            label: label.to_string(),
            category: category.to_string(),
            meta: BTreeMap::new(),
            ghost: false,
        })
    }

    /// Returns the node for `key`, creating a ghost if it doesn't exist yet.
    pub fn ensure_node(&mut self, key: &str) -> NodeId {
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let label = key.split_once(':').map_or(key, |(_, rest)| rest);
        self.push_node(NodeData {
            key: key.to_string(),
            label: label.to_string(),
            category: "ghost".to_string(),
            meta: BTreeMap::new(),
            ghost: true,
        })
    }

    fn push_node(&mut self, data: NodeData) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("more than u32::MAX nodes"));
        self.index.insert(data.key.clone(), id);
        self.nodes.push(data);
        self.adjacency.push(Vec::new());
        id
    }

    /// Adds an undirected-for-layout edge. Self loops and duplicates (in either
    /// direction) are ignored; returns whether an edge was added.
    pub fn add_edge(
        &mut self,
        source: NodeId,
        target: NodeId,
        weight: f32,
        kind: EdgeKind,
    ) -> bool {
        if source == target
            || source.index() >= self.nodes.len()
            || target.index() >= self.nodes.len()
        {
            return false;
        }
        let pair = if source < target {
            (source, target)
        } else {
            (target, source)
        };
        if !self.edge_set.insert(pair) {
            return false;
        }
        let weight = if weight.is_finite() && weight > 0.0 {
            weight
        } else {
            1.0
        };
        self.edges.push(EdgeData {
            source,
            target,
            weight,
            kind,
        });
        self.adjacency[source.index()].push(target);
        self.adjacency[target.index()].push(source);
        true
    }

    /// Convenience: adds an edge by key, creating ghost nodes for unknown keys.
    pub fn link(&mut self, source: &str, target: &str) -> bool {
        let s = self.ensure_node(source);
        let t = self.ensure_node(target);
        self.add_edge(s, t, 1.0, EdgeKind::Link)
    }

    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.index()]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.index()]
    }

    pub fn id_of(&self, key: &str) -> Option<NodeId> {
        self.index.get(key).copied()
    }

    pub fn nodes(&self) -> &[NodeData] {
        &self.nodes
    }

    pub fn edges(&self) -> &[EdgeData] {
        &self.edges
    }

    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + use<> {
        (0..self.nodes.len() as u32).map(NodeId)
    }

    pub fn neighbors(&self, id: NodeId) -> &[NodeId] {
        &self.adjacency[id.index()]
    }

    pub fn degree(&self, id: NodeId) -> usize {
        self.adjacency[id.index()].len()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
