//! Force-directed layout: a Fruchterman-Reingold / d3-force hybrid with a
//! fixed timestep and *alpha cooling*, so the simulation actually comes to
//! rest and frontends can stop redrawing.
//!
//! Forces (all scaled by `alpha`):
//! - repulsion between every pair: `repulsion / max(d, min_distance)`
//!   (Fruchterman-Reingold 1/d falloff: long-ranged enough to pull clusters
//!   apart, unlike 1/d², which leaves nodes stuck on top of edges)
//! - spring along each edge: `link_strength · weight · (d − link_distance)`,
//!   divided by the smaller endpoint degree so hubs don't explode
//! - gravity toward the origin: `−gravity · pos`
//!
//! Velocities are damped by `friction` each step and clamped to `max_velocity`.

use serde::{Deserialize, Serialize};

use crate::layout::Layout;
use crate::math::Vec2;
use crate::model::Graph;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysicsParams {
    pub repulsion: f32,
    pub min_distance: f32,
    pub link_strength: f32,
    pub link_distance: f32,
    pub gravity: f32,
    /// Fraction of velocity removed per step (0..1).
    pub friction: f32,
    /// Maximum displacement per step, in world units.
    pub max_velocity: f32,
    /// Below this alpha the simulation is considered settled.
    pub alpha_min: f32,
    /// Per-step alpha decay toward `alpha_target`.
    pub alpha_decay: f32,
    /// Steps per second of the fixed timestep.
    pub steps_per_second: f32,
    /// Spacing used for initial placement.
    pub initial_spacing: f32,
}

impl Default for PhysicsParams {
    fn default() -> Self {
        Self {
            repulsion: 60.0,
            min_distance: 12.0,
            link_strength: 0.45,
            link_distance: 60.0,
            gravity: 0.005,
            // Measured with examples/tune.rs: 0.15 halves edge crossings vs
            // 0.4 on trees and meshes, still settling in ~300 steps.
            friction: 0.15,
            max_velocity: 40.0,
            alpha_min: 0.001,
            // 1 - alpha_min^(1/300): settles in ~300 steps (5 s at 60 Hz).
            alpha_decay: 0.0228,
            steps_per_second: 60.0,
            initial_spacing: 20.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Simulation {
    pub params: PhysicsParams,
    alpha: f32,
    alpha_target: f32,
    accumulator: f32,
    forces: Vec<Vec2>,
    /// Positions before the latest step, for render interpolation.
    prev: Vec<Vec2>,
}

impl Simulation {
    pub fn new(params: PhysicsParams) -> Self {
        Self {
            params,
            alpha: 1.0,
            alpha_target: 0.0,
            accumulator: 0.0,
            forces: Vec::new(),
            prev: Vec::new(),
        }
    }

    /// Positions to *draw* when the display refreshes faster than the
    /// physics steps (e.g. a 144 Hz window over 60 Hz physics): a blend of
    /// the last two steps by how far we are into the next one, so motion is
    /// smooth instead of juddering. Pinned (dragged) nodes use their live
    /// position. `None` when there's nothing to blend.
    pub fn interpolated(&self, layout: &Layout) -> Option<Vec<Vec2>> {
        if self.is_settled() || self.prev.len() != layout.len() {
            return None;
        }
        let step = 1.0 / self.params.steps_per_second.max(1.0);
        let t = (self.accumulator / step).clamp(0.0, 1.0);
        Some(
            (0..layout.len())
                .map(|i| {
                    let cur = layout.pos[i];
                    if layout.pinned[i] {
                        cur
                    } else {
                        self.prev[i] + (cur - self.prev[i]) * t
                    }
                })
                .collect(),
        )
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn is_settled(&self) -> bool {
        self.alpha < self.params.alpha_min && self.alpha_target < self.params.alpha_min
    }

    /// Raises alpha to at least `alpha` (e.g. after data changes: 1.0,
    /// after a drag: ~0.3).
    pub fn reheat(&mut self, alpha: f32) {
        self.alpha = self.alpha.max(alpha);
    }

    /// While set, alpha won't decay below `target` — used during drags so
    /// the neighbourhood keeps reacting.
    pub fn set_alpha_target(&mut self, target: f32) {
        self.alpha_target = target;
        self.alpha = self.alpha.max(target);
    }

    /// Advances by wall-clock `dt` seconds using a fixed timestep. Returns
    /// `true` if any step ran (i.e. positions may have changed).
    pub fn advance(&mut self, graph: &Graph, layout: &mut Layout, dt: f32) -> bool {
        if self.is_settled() {
            self.accumulator = 0.0;
            return false;
        }
        let step = 1.0 / self.params.steps_per_second.max(1.0);
        // Never try to catch up more than a few frames after a stall.
        self.accumulator = (self.accumulator + dt).min(step * 8.0);
        let mut ran = false;
        // 10% tolerance: when frames and steps share a rate, a frame that
        // wakes a hair early still gets its step instead of skipping one
        // (the small debt is repaid by the next frame's time).
        while self.accumulator >= step * 0.9 {
            self.accumulator -= step;
            self.step(graph, layout);
            ran = true;
        }
        ran
    }

    /// Steps until settled or `max_steps` is reached; returns steps taken.
    pub fn run_until_settled(
        &mut self,
        graph: &Graph,
        layout: &mut Layout,
        max_steps: usize,
    ) -> usize {
        let mut n = 0;
        while n < max_steps && !self.is_settled() {
            self.step(graph, layout);
            n += 1;
        }
        n
    }

    /// One fixed simulation step.
    pub fn step(&mut self, graph: &Graph, layout: &mut Layout) {
        self.prev.clone_from(&layout.pos);
        let n = layout.len().min(graph.node_count());
        let p = &self.params;
        let alpha = self.alpha;
        self.forces.clear();
        self.forces.resize(n, Vec2::ZERO);
        let forces = &mut self.forces;
        let pos = &layout.pos;

        // Repulsion (brute force O(n²); Barnes-Hut replaces this for big graphs).
        for i in 0..n {
            for j in (i + 1)..n {
                let mut d = pos[i] - pos[j];
                let mut dist2 = d.length_squared();
                if dist2 < 1e-6 {
                    d = jitter(i, j);
                    dist2 = d.length_squared();
                }
                let dist = dist2.sqrt();
                let f = p.repulsion / dist.max(p.min_distance);
                let fv = d * (f / dist);
                forces[i] += fv;
                forces[j] -= fv;
            }
        }

        // Springs.
        for e in graph.edges() {
            let (s, t) = (e.source.index(), e.target.index());
            if s >= n || t >= n {
                continue;
            }
            let d = pos[t] - pos[s];
            let dist = d.length().max(1e-3);
            let bias = 1.0 / graph.degree(e.source).min(graph.degree(e.target)).max(1) as f32;
            let f = p.link_strength * e.weight * bias * (dist - p.link_distance);
            let fv = d * (f / dist);
            forces[s] += fv;
            forces[t] -= fv;
        }

        // Gravity + integration.
        let keep = 1.0 - p.friction.clamp(0.0, 1.0);
        for i in 0..n {
            if layout.pinned[i] {
                layout.vel[i] = Vec2::ZERO;
                continue;
            }
            let f = forces[i] - layout.pos[i] * p.gravity;
            let v = ((layout.vel[i] + f * alpha) * keep).clamp_length(p.max_velocity);
            layout.vel[i] = v;
            layout.pos[i] += v;
        }

        self.alpha += (self.alpha_target - self.alpha) * p.alpha_decay;
    }
}

/// Total kinetic energy (sum of squared speeds). Useful for tests and for
/// deciding when to stop animating.
pub fn kinetic_energy(layout: &Layout) -> f32 {
    layout.vel.iter().map(|v| v.length_squared()).sum()
}

/// Tiny deterministic displacement for coincident nodes (never zero).
fn jitter(i: usize, j: usize) -> Vec2 {
    let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    let angle = (h % 6283) as f32 / 1000.0;
    Vec2::new(angle.cos(), angle.sin()) * 0.01
}
