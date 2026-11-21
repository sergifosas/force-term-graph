use graph_core::physics::kinetic_energy;
use graph_core::{Graph, Layout, PhysicsParams, Simulation, Vec2};

/// Two 6-cliques joined by a single bridge edge.
fn two_clusters() -> Graph {
    let mut g = Graph::new();
    for c in 0..2 {
        for i in 0..6 {
            g.add_node(&format!("{c}-{i}"), &format!("{c}-{i}"), &format!("c{c}"));
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

fn settle(g: &Graph, layout: &mut Layout) -> Simulation {
    let mut sim = Simulation::new(PhysicsParams::default());
    sim.run_until_settled(g, layout, 5_000);
    sim
}

#[test]
fn simulation_settles_and_stays_finite() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    let sim = settle(&g, &mut layout);
    assert!(
        sim.is_settled(),
        "alpha {} never dropped below alpha_min",
        sim.alpha()
    );
    assert!(layout.pos.iter().all(|p| p.is_finite()));
}

#[test]
fn coincident_nodes_do_not_produce_nan() {
    let mut g = Graph::new();
    for i in 0..10 {
        g.add_node(&i.to_string(), "", "x");
    }
    g.link("0", "1");
    let mut layout = Layout::for_graph(&g, 20.0);
    for p in &mut layout.pos {
        *p = Vec2::new(5.0, 5.0);
    }
    let mut sim = Simulation::new(PhysicsParams::default());
    for _ in 0..1_000 {
        sim.step(&g, &mut layout);
    }
    assert!(layout.pos.iter().all(|p| p.is_finite()));
    // And they actually separated.
    assert!((layout.pos[0] - layout.pos[2]).length() > 1.0);
}

#[test]
fn energy_decreases_as_alpha_cools() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    let mut sim = Simulation::new(PhysicsParams::default());
    let mut early = 0.0;
    for _ in 0..30 {
        sim.step(&g, &mut layout);
        early += kinetic_energy(&layout);
    }
    for _ in 0..300 {
        sim.step(&g, &mut layout);
    }
    let mut late = 0.0;
    for _ in 0..30 {
        sim.step(&g, &mut layout);
        late += kinetic_energy(&layout);
    }
    assert!(late < early * 0.05, "early={early} late={late}");
}

#[test]
fn connected_nodes_end_up_closer_than_unconnected() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    settle(&g, &mut layout);

    let (mut conn, mut conn_n, mut unconn, mut unconn_n) = (0.0, 0, 0.0, 0);
    for a in g.node_ids() {
        for b in g.node_ids().filter(|b| *b > a) {
            let d = (layout.position(a) - layout.position(b)).length();
            if g.neighbors(a).contains(&b) {
                conn += d;
                conn_n += 1;
            } else {
                unconn += d;
                unconn_n += 1;
            }
        }
    }
    let (conn, unconn) = (conn / conn_n as f32, unconn / unconn_n as f32);
    assert!(
        conn < unconn * 0.7,
        "connected {conn} vs unconnected {unconn}"
    );
}

#[test]
fn same_input_gives_identical_layout() {
    let g = two_clusters();
    let mut a = Layout::for_graph(&g, 20.0);
    let mut b = Layout::for_graph(&g, 20.0);
    settle(&g, &mut a);
    settle(&g, &mut b);
    assert_eq!(a.pos, b.pos);
}

#[test]
fn pinned_nodes_do_not_move() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    let id = g.id_of("0-3").unwrap();
    layout.set_position(id, Vec2::new(500.0, 500.0));
    layout.set_pinned(id, true);
    settle(&g, &mut layout);
    assert_eq!(layout.position(id), Vec2::new(500.0, 500.0));
}

#[test]
fn interpolation_blends_between_the_last_two_steps() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    let mut sim = Simulation::new(PhysicsParams::default());
    assert!(sim.interpolated(&layout).is_none(), "nothing to blend yet");

    let before = layout.pos.clone();
    sim.step(&g, &mut layout); // accumulator stays 0 → t = 0 → previous positions
    let at_start = sim.interpolated(&layout).unwrap();
    assert_eq!(at_start, before);

    // Half a step of time accumulated (below the 0.9 threshold) → midpoint.
    assert!(!sim.advance(&g, &mut layout, 0.5 / 60.0));
    let mid = sim.interpolated(&layout).unwrap();
    for i in 0..layout.len() {
        let expect = before[i] + (layout.pos[i] - before[i]) * 0.5;
        assert!((mid[i] - expect).length() < 1e-3);
    }
}

#[test]
fn advance_uses_fixed_timestep_and_stops_when_settled() {
    let g = two_clusters();
    let mut layout = Layout::for_graph(&g, 20.0);
    let mut sim = Simulation::new(PhysicsParams::default());
    // Less than one step's worth of time: nothing runs.
    assert!(!sim.advance(&g, &mut layout, 0.001));
    assert!(sim.advance(&g, &mut layout, 0.02));
    sim.run_until_settled(&g, &mut layout, 5_000);
    assert!(
        !sim.advance(&g, &mut layout, 1.0),
        "settled sim must not request redraws"
    );
    sim.reheat(0.5);
    assert!(sim.advance(&g, &mut layout, 0.02), "reheat resumes motion");
}
