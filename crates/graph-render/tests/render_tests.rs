use graph_core::{Anchor, Color, Scene, Shape, Vec2};
use graph_render::{PngSpeed, Renderer};

fn scene() -> Scene {
    Scene {
        width: 200,
        height: 100,
        background: Color::rgb(10, 10, 10),
        shapes: vec![
            Shape::Line {
                a: Vec2::new(20.0, 50.0),
                b: Vec2::new(180.0, 50.0),
                width: 2.0,
                color: Color::rgb(80, 80, 80),
            },
            Shape::Circle {
                center: Vec2::new(50.0, 50.0),
                radius: 10.0,
                fill: Color::rgb(255, 0, 0),
                stroke: None,
            },
            Shape::Text {
                pos: Vec2::new(150.0, 10.0),
                text: "Hi".into(),
                size: 20.0,
                color: Color::rgb(255, 255, 255),
                anchor: Anchor::TopCenter,
            },
        ],
    }
}

#[test]
fn paints_background_shapes_and_text() {
    let mut r = Renderer::new().unwrap();
    let pm = r.render(&scene()).unwrap();
    assert_eq!((pm.width(), pm.height()), (200, 100));

    let px = |x: u32, y: u32| pm.pixel(x, y).unwrap();
    assert_eq!(
        (px(5, 95).red(), px(5, 95).alpha()),
        (10, 255),
        "opaque background"
    );
    assert_eq!(px(50, 50).red(), 255, "circle centre is red");

    // Some bright text pixels in the label area.
    let bright = (135..165)
        .flat_map(|x| (10..35).map(move |y| (x, y)))
        .filter(|&(x, y)| px(x, y).green() > 200)
        .count();
    assert!(bright > 20, "expected text pixels, got {bright}");
}

#[test]
fn scaled_render_halves_size_and_keeps_geometry() {
    let mut r = Renderer::new().unwrap();
    let pm = r.render_scaled(&scene(), 0.5).unwrap();
    assert_eq!((pm.width(), pm.height()), (100, 50));
    assert_eq!(
        pm.pixel(25, 25).unwrap().red(),
        255,
        "circle moved with the scale"
    );
}

#[test]
fn encodes_valid_png() {
    let mut r = Renderer::new().unwrap();
    let png = r.render_png(&scene(), PngSpeed::Fast).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(
        png.len() < 20_000,
        "dark simple scene should compress well: {} bytes",
        png.len()
    );
}
