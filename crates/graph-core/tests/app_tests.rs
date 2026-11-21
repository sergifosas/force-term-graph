use graph_core::{
    AppEvent, AppState, Camera, Graph, MouseButton, PhysicsParams, Shape, Style, Vec2,
};

#[test]
fn camera_round_trips_and_zooms_around_cursor() {
    let mut c = Camera::new(800.0, 600.0);
    c.center = Vec2::new(10.0, -20.0);
    c.zoom = 2.0;
    let w = Vec2::new(33.0, 44.0);
    let back = c.screen_to_world(c.world_to_screen(w));
    assert!((back - w).length() < 1e-4);

    let cursor = Vec2::new(100.0, 500.0);
    let before = c.screen_to_world(cursor);
    c.zoom_at(cursor, 1.7);
    assert!((c.screen_to_world(cursor) - before).length() < 1e-3);
}

fn small_app() -> AppState {
    let mut g = Graph::new();
    g.add_node("a", "Alpha", "x");
    g.add_node("b", "Beta", "y");
    g.add_node("c", "Gamma", "y");
    g.link("a", "b");
    let mut app = AppState::new(
        g,
        PhysicsParams::default(),
        Style::default(),
        (800.0, 600.0),
    );
    app.settle(5_000);
    app
}

#[test]
fn scene_contains_edges_nodes_and_labels_for_small_graphs() {
    let app = small_app();
    let scene = app.scene();
    let count = |f: fn(&Shape) -> bool| scene.shapes.iter().filter(|s| f(s)).count();
    assert_eq!(count(|s| matches!(s, Shape::Line { .. })), 1);
    assert_eq!(count(|s| matches!(s, Shape::Circle { .. })), 3);
    assert_eq!(count(|s| matches!(s, Shape::Text { .. })), 3);
    assert_eq!((scene.width, scene.height), (800, 600));
    // Everything was fitted into the viewport.
    for s in &scene.shapes {
        if let Shape::Circle { center, .. } = s {
            assert!(center.x >= 0.0 && center.x <= 800.0 && center.y >= 0.0 && center.y <= 600.0);
        }
    }
}

#[test]
fn hover_and_drag_a_node() {
    let mut app = small_app();
    let a = app.graph.id_of("a").unwrap();
    let screen = app.camera.world_to_screen(app.layout.position(a));

    app.update(AppEvent::PointerMove(screen));
    assert_eq!(app.highlight.hovered, Some(a));

    app.update(AppEvent::PointerDown(screen, MouseButton::Left));
    assert_eq!(app.highlight.selected, Some(a));
    assert!(
        app.layout.pinned[a.index()],
        "dragged node is pinned while held"
    );

    let target = screen + Vec2::new(50.0, 0.0);
    app.update(AppEvent::PointerMove(target));
    let moved = app.camera.world_to_screen(app.layout.position(a));
    assert!((moved - target).length() < 1e-2);
    assert!(app.tick(0.05), "drag reheats physics");

    app.update(AppEvent::PointerUp(target, MouseButton::Left));
    assert!(!app.layout.pinned[a.index()]);
}

#[test]
fn grabbing_off_centre_does_not_snap_the_node() {
    let mut app = small_app();
    app.hit_slop = 12.0;
    let a = app.graph.id_of("a").unwrap();
    let centre = app.camera.world_to_screen(app.layout.position(a));
    let grab = centre + Vec2::new(0.0, 8.0); // inside slop, off-centre
    app.update(AppEvent::PointerDown(grab, MouseButton::Left));
    assert!(app.is_dragging());
    app.update(AppEvent::PointerMove(grab + Vec2::new(30.0, 0.0)));
    let now = app.camera.world_to_screen(app.layout.position(a));
    assert!((now - (centre + Vec2::new(30.0, 0.0))).length() < 1e-2);
}

#[test]
fn keyboard_pan_reheat_and_clear_selection() {
    let mut app = small_app();
    app.tick(0.0);
    let before = app.camera.center;
    app.update(AppEvent::PanBy(Vec2::new(-40.0, 0.0)));
    assert!(
        app.camera.center.x > before.x,
        "content moves left = view moves right"
    );

    assert!(!app.is_animating());
    app.update(AppEvent::Reheat);
    assert!(app.is_animating());

    app.highlight.selected = app.graph.id_of("b");
    app.update(AppEvent::ClearSelection);
    assert_eq!(app.highlight.selected, None);
}

#[test]
fn tick_reports_idle_once_settled() {
    let mut app = small_app();
    app.tick(0.0); // consume the initial dirty flag
    assert!(!app.tick(1.0));
    app.update(AppEvent::Scroll(Vec2::new(400.0, 300.0), 1.0));
    assert!(app.tick(0.0), "zoom requests one redraw");
    assert!(!app.tick(0.1));
}
