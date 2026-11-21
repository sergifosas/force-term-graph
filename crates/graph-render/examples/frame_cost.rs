//! Measures per-frame render and PNG encode cost at the probe's pane size.
//! `cargo run --release -p graph-render --example frame_cost`
use std::time::Instant;

use graph_core::{AppState, Graph, PhysicsParams, Style};
use graph_render::Renderer;

fn graph() -> Graph {
    // ~300 nodes in a scale-free-ish tree with some cross links.
    let mut g = Graph::new();
    for i in 0..300u32 {
        g.add_node(&i.to_string(), &format!("note {i}"), &format!("c{}", i % 6));
        if i > 0 {
            g.link(&i.to_string(), &((i * 7 / 11) % i).to_string());
        }
        if i % 9 == 0 && i > 20 {
            g.link(&i.to_string(), &(i / 3).to_string());
        }
    }
    g
}

fn encode(rgb: &[u8], w: u32, h: u32, c: png::Compression, f: Option<png::Filter>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_compression(c);
    if let Some(f) = f {
        enc.set_filter(f);
    }
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(rgb).unwrap();
    wr.finish().unwrap();
    out
}

fn main() {
    let (w, h) = (2544u32, 1296u32);
    let mut app = AppState::new(
        graph(),
        PhysicsParams::default(),
        Style::default(),
        (w as f32, h as f32),
    );
    app.settle(5000);
    let scene = app.scene();
    let mut r = Renderer::new().unwrap();
    let n = 20;

    let t = Instant::now();
    let mut pm = None;
    for _ in 0..n {
        pm = Some(r.render(&scene).unwrap());
    }
    println!(
        "render: {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let pm = pm.unwrap();

    let t = Instant::now();
    let mut rgb = Vec::new();
    for _ in 0..n {
        rgb = pm
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect();
    }
    println!(
        "rgba→rgb: {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );

    let t = Instant::now();
    let mut bytes = 0;
    for _ in 0..n {
        bytes = graph_render::encode_png(&pm, graph_render::PngSpeed::Fast)
            .unwrap()
            .len();
    }
    println!(
        "encode_png(Fast) [rgba/sub, used live]: {:.2} ms  {} KB",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64,
        bytes / 1024
    );

    use png::{Compression as C, Filter as F};
    for (name, c, f) in [
        ("fast/adaptive", C::Fast, None),
        ("fast/none", C::Fast, Some(F::NoFilter)),
        ("fast/sub", C::Fast, Some(F::Sub)),
        ("fast/up", C::Fast, Some(F::Up)),
        ("fastest/up", C::Fastest, Some(F::Up)),
        ("fastest/none", C::Fastest, Some(F::NoFilter)),
    ] {
        let t = Instant::now();
        let mut bytes = 0;
        for _ in 0..n {
            bytes = encode(&rgb, w, h, c, f).len();
        }
        println!(
            "{name:14} {:.2} ms  {} KB",
            t.elapsed().as_secs_f64() * 1000.0 / n as f64,
            bytes / 1024
        );
    }
}
