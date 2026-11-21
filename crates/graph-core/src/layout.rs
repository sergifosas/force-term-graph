//! Where nodes *are*: positions, velocities, pins. Struct-of-arrays indexed by
//! [`NodeId`], kept separate from the model.

use crate::math::{Rect, Vec2};
use crate::model::{Graph, NodeId};

/// Golden angle in radians, for phyllotaxis placement.
const GOLDEN_ANGLE: f32 = 2.399_963_2;

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub pos: Vec<Vec2>,
    pub vel: Vec<Vec2>,
    pub pinned: Vec<bool>,
}

impl Layout {
    /// Deterministic initial placement on a phyllotaxis (sunflower) spiral:
    /// no randomness and no two nodes at the same point, so the same input
    /// always produces the same layout.
    pub fn seeded(n: usize, spacing: f32) -> Self {
        let mut l = Layout::default();
        for i in 0..n {
            l.push(spiral_point(i, spacing));
        }
        l
    }

    /// Spiral placement in node order. (Breadth-first slot assignment was
    /// measured with `examples/tune.rs` and produced *more* crossings, so
    /// the simple order stays.)
    pub fn for_graph(graph: &Graph, spacing: f32) -> Self {
        Self::seeded(graph.node_count(), spacing)
    }

    fn push(&mut self, p: Vec2) {
        self.pos.push(p);
        self.vel.push(Vec2::ZERO);
        self.pinned.push(false);
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    pub fn position(&self, id: NodeId) -> Vec2 {
        self.pos[id.index()]
    }

    pub fn set_position(&mut self, id: NodeId, p: Vec2) {
        self.pos[id.index()] = p;
        self.vel[id.index()] = Vec2::ZERO;
    }

    pub fn set_pinned(&mut self, id: NodeId, pinned: bool) {
        self.pinned[id.index()] = pinned;
        if pinned {
            self.vel[id.index()] = Vec2::ZERO;
        }
    }

    pub fn bounds(&self) -> Option<Rect> {
        Rect::from_points(self.pos.iter().copied())
    }

    /// Grows the layout to cover nodes added to `graph` since it was built.
    /// New nodes are placed next to an already-placed neighbour when one
    /// exists, otherwise on the spiral.
    pub fn extend_for(&mut self, graph: &Graph, spacing: f32) {
        while self.len() < graph.node_count() {
            let i = self.len();
            let id = NodeId(i as u32);
            let anchor = graph
                .neighbors(id)
                .iter()
                .find(|n| n.index() < i)
                .map(|n| self.pos[n.index()]);
            let p = match anchor {
                Some(a) => a + spiral_point(i % 13 + 1, spacing * 0.5),
                None => spiral_point(i, spacing),
            };
            self.push(p);
        }
    }

    /// Builds a layout for `new_graph`, keeping positions (and pins) of nodes
    /// whose key also exists in `old_graph`. Used for live reloads so the
    /// picture doesn't scramble.
    pub fn remap(old_graph: &Graph, old: &Layout, new_graph: &Graph, spacing: f32) -> Layout {
        let mut out = Layout::default();
        for id in new_graph.node_ids() {
            let key = &new_graph.node(id).key;
            match old_graph.id_of(key) {
                Some(old_id) if old_id.index() < old.len() => {
                    out.pos.push(old.pos[old_id.index()]);
                    out.vel.push(Vec2::ZERO);
                    out.pinned.push(old.pinned[old_id.index()]);
                }
                _ => {
                    // Placeholder; positioned below once neighbours are known.
                    out.pos.push(Vec2::new(f32::NAN, f32::NAN));
                    out.vel.push(Vec2::ZERO);
                    out.pinned.push(false);
                }
            }
        }
        for i in 0..out.len() {
            if !out.pos[i].is_finite() {
                let id = NodeId(i as u32);
                let anchor = new_graph
                    .neighbors(id)
                    .iter()
                    .map(|n| out.pos[n.index()])
                    .find(|p| p.is_finite());
                out.pos[i] = match anchor {
                    Some(a) => a + spiral_point(i % 13 + 1, spacing * 0.5),
                    None => spiral_point(i, spacing),
                };
            }
        }
        out
    }
}

fn spiral_point(i: usize, spacing: f32) -> Vec2 {
    let r = spacing * (i as f32 + 0.5).sqrt();
    let a = i as f32 * GOLDEN_ANGLE;
    Vec2::new(r * a.cos(), r * a.sin())
}
