//! `GraphData`: the plain, serde-friendly exchange format every data source
//! produces. It is also the schema of the JSON source.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{EdgeKind, Graph};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphData {
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
    #[serde(default)]
    pub edges: Vec<EdgeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub source: String,
    pub target: String,
    #[serde(default = "default_weight")]
    pub weight: f32,
    #[serde(default)]
    pub kind: EdgeKind,
}

fn default_weight() -> f32 {
    1.0
}

impl GraphData {
    /// Prefixes every id with `namespace:` so several sources can merge
    /// without key collisions.
    pub fn namespaced(mut self, namespace: &str) -> Self {
        let ns = |id: &mut String| *id = format!("{namespace}:{id}");
        for n in &mut self.nodes {
            ns(&mut n.id);
        }
        for e in &mut self.edges {
            ns(&mut e.source);
            ns(&mut e.target);
        }
        self
    }

    /// Merges this data into `graph`. Edges referencing unknown ids create
    /// ghost nodes rather than being dropped.
    pub fn apply_to(&self, graph: &mut Graph) {
        for n in &self.nodes {
            let label = n.label.as_deref().unwrap_or_else(|| strip_namespace(&n.id));
            let category = n.category.as_deref().unwrap_or("default");
            let id = graph.add_node(&n.id, label, category);
            graph.node_mut(id).meta.extend(n.meta.clone());
        }
        for e in &self.edges {
            let s = graph.ensure_node(&e.source);
            let t = graph.ensure_node(&e.target);
            graph.add_edge(s, t, e.weight, e.kind);
        }
    }

    pub fn into_graph(self) -> Graph {
        let mut g = Graph::new();
        self.apply_to(&mut g);
        g
    }
}

fn strip_namespace(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, rest)| rest)
}
