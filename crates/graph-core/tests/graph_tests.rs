use graph_core::{EdgeKind, Graph, GraphData, Layout, Vec2};

#[test]
fn add_node_is_idempotent() {
    let mut g = Graph::new();
    let a = g.add_node("md:a", "A", "note");
    let a2 = g.add_node("md:a", "A renamed", "note");
    assert_eq!(a, a2);
    assert_eq!(g.node_count(), 1);
    assert_eq!(g.node(a).label, "A renamed");
}

#[test]
fn unknown_edge_targets_become_ghosts_and_can_be_resolved() {
    let mut g = Graph::new();
    g.add_node("md:a", "A", "note");
    assert!(g.link("md:a", "md:missing"));
    let ghost = g.id_of("md:missing").unwrap();
    assert!(g.node(ghost).ghost);
    assert_eq!(g.node(ghost).label, "missing");

    // Defining it later turns the ghost into a real node, same id.
    let real = g.add_node("md:missing", "Missing", "note");
    assert_eq!(real, ghost);
    assert!(!g.node(real).ghost);
}

#[test]
fn duplicate_and_self_edges_are_ignored() {
    let mut g = Graph::new();
    let a = g.add_node("a", "A", "x");
    let b = g.add_node("b", "B", "x");
    assert!(g.add_edge(a, b, 1.0, EdgeKind::Link));
    assert!(!g.add_edge(b, a, 1.0, EdgeKind::Link), "reverse duplicate");
    assert!(!g.add_edge(a, a, 1.0, EdgeKind::Link), "self loop");
    assert_eq!(g.edge_count(), 1);
    assert_eq!(g.degree(a), 1);
    assert_eq!(g.neighbors(b), &[a]);
}

#[test]
fn graph_data_round_trips_through_json_and_namespaces() {
    let json = r#"{
        "nodes": [{"id": "rust", "label": "Rust", "category": "lang"}, {"id": "wezterm"}],
        "edges": [{"source": "rust", "target": "wezterm"}, {"source": "rust", "target": "nowhere", "weight": 2}]
    }"#;
    let data: GraphData = serde_json::from_str(json).unwrap();
    let g = data.namespaced("json").into_graph();
    assert_eq!(g.node_count(), 3);
    assert_eq!(g.edge_count(), 2);
    let rust = g.id_of("json:rust").unwrap();
    assert_eq!(g.node(rust).label, "Rust");
    assert_eq!(g.node(g.id_of("json:wezterm").unwrap()).label, "wezterm");
    assert!(g.node(g.id_of("json:nowhere").unwrap()).ghost);
}

#[test]
fn seeded_layout_is_deterministic_and_has_no_coincident_points() {
    let a = Layout::seeded(200, 20.0);
    let b = Layout::seeded(200, 20.0);
    assert_eq!(a.pos, b.pos);
    for i in 0..a.len() {
        for j in (i + 1)..a.len() {
            assert!((a.pos[i] - a.pos[j]).length() > 1.0, "{i} and {j} overlap");
        }
    }
}

#[test]
fn remap_keeps_positions_of_surviving_nodes() {
    let mut old = Graph::new();
    old.add_node("a", "A", "x");
    old.add_node("b", "B", "x");
    let mut old_layout = Layout::for_graph(&old, 20.0);
    old_layout.set_position(old.id_of("b").unwrap(), Vec2::new(123.0, -45.0));

    // New graph: different order, one node removed, one added.
    let mut new = Graph::new();
    new.add_node("c", "C", "x");
    new.add_node("b", "B", "x");
    new.link("c", "b");

    let l = Layout::remap(&old, &old_layout, &new, 20.0);
    assert_eq!(l.len(), 2);
    assert_eq!(l.position(new.id_of("b").unwrap()), Vec2::new(123.0, -45.0));
    let c = l.position(new.id_of("c").unwrap());
    assert!(c.is_finite());
    assert!(
        (c - Vec2::new(123.0, -45.0)).length() < 50.0,
        "new node placed near its neighbour"
    );
}
