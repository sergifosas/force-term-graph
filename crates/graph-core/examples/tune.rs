//! Physics tuning harness: layout quality for a few parameter sets.
//! `cargo run --release -p graph-core --example tune`
//!
//! Metrics (lower is better): edge crossings, and the ratio of mean
//! connected distance to mean unconnected distance.
use graph_core::{Graph, Layout, PhysicsParams, Simulation, Vec2};

fn two_clusters() -> Graph {
    let mut g = Graph::new();
    for c in 0..2 {
        for i in 0..6 {
            g.add_node(&format!("{c}-{i}"), "", "");
        }
        for i in 0..6 {
            for j in (i + 1)..6 {
                g.link(&format!("{c}-{i}"), &format!("{c}-{j}"));
            }
        }
    }
    g.link("0-0", "1-0");
    g
}

/// Deterministic pseudo-random tree, like a source tree (~120 nodes).
fn tree() -> Graph {
    let mut g = Graph::new();
    g.add_node("0", "", "");
    let mut x: u64 = 12345;
    for i in 1..120u64 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Bias toward recent nodes → deep-ish, bushy tree.
        let parent = i - 1 - (x >> 33) % i.min(12);
        g.link(&parent.to_string(), &i.to_string());
    }
    g
}

/// Sparse mesh: ring + chords, like a small vault.
fn mesh() -> Graph {
    let mut g = Graph::new();
    let n = 40;
    for i in 0..n {
        g.link(&i.to_string(), &((i + 1) % n).to_string());
        if i % 3 == 0 {
            g.link(&i.to_string(), &((i * 7 + 5) % n).to_string());
        }
    }
    g
}

fn crossings(g: &Graph, l: &Layout) -> usize {
    let seg = |e: &graph_core::EdgeData| (l.position(e.source), l.position(e.target));
    let cross = |(a, b): (Vec2, Vec2), (c, d): (Vec2, Vec2)| {
        let o = |p: Vec2, q: Vec2, r: Vec2| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
        o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
    };
    let edges = g.edges();
    let mut n = 0;
    for i in 0..edges.len() {
        for j in (i + 1)..edges.len() {
            let (e, f) = (&edges[i], &edges[j]);
            let shared = e.source == f.source
                || e.source == f.target
                || e.target == f.source
                || e.target == f.target;
            if !shared && cross(seg(e), seg(f)) {
                n += 1;
            }
        }
    }
    n
}

fn ratio(g: &Graph, l: &Layout) -> f32 {
    let (mut c, mut cn, mut u, mut un) = (0.0, 0, 0.0, 0);
    for a in g.node_ids() {
        for b in g.node_ids().filter(|b| *b > a) {
            let d = (l.position(a) - l.position(b)).length();
            if g.neighbors(a).contains(&b) {
                c += d;
                cn += 1;
            } else {
                u += d;
                un += 1;
            }
        }
    }
    (c / cn as f32) / (u / un as f32)
}

fn main() {
    let d = PhysicsParams::default;
    let sets = [
        ("default", d()),
        (
            "friction .4",
            PhysicsParams {
                friction: 0.4,
                ..d()
            },
        ),
        (
            "slow cooling",
            PhysicsParams {
                alpha_decay: 0.0114,
                ..d()
            },
        ),
    ];
    for (gname, g) in [
        ("clusters", two_clusters()),
        ("tree", tree()),
        ("mesh", mesh()),
    ] {
        println!(
            "== {gname}: {} nodes, {} edges",
            g.node_count(),
            g.edge_count()
        );
        for (name, p) in &sets {
            {
                let mut l = Layout::for_graph(&g, p.initial_spacing);
                let mut sim = Simulation::new(*p);
                let steps = sim.run_until_settled(&g, &mut l, 20_000);
                println!(
                    "  {name:13} steps={steps:4} crossings={:4} ratio={:.2}",
                    crossings(&g, &l),
                    ratio(&g, &l)
                );
            }
        }
    }
}
