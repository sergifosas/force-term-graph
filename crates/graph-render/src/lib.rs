//! CPU rasterizer for [`graph_core::Scene`]: anti-aliased shapes via
//! tiny-skia, text via fontdue with an embedded OFL font (Fira Sans).
//!
//! The output is used for terminal image protocols and `--png` export.

use std::collections::HashMap;

use anyhow::Context;
use graph_core::{Anchor, Color, Scene, Shape};
use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

const FONT_BYTES: &[u8] = include_bytes!("../assets/FiraSans-Regular.ttf");

/// The embedded label font (Fira Sans, SIL OFL), shared with other
/// frontends so every view uses the same typeface.
pub const LABEL_FONT: &[u8] = FONT_BYTES;

/// PNG compression trade-off. Terminal frames favour speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngSpeed {
    Fast,
    Small,
}

pub struct Renderer {
    font: fontdue::Font,
    /// (char, size in 1/4 px) → rasterized coverage.
    glyphs: HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)>,
}

impl Renderer {
    pub fn new() -> anyhow::Result<Self> {
        let font = fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("loading embedded font: {e}"))?;
        Ok(Self {
            font,
            glyphs: HashMap::new(),
        })
    }

    /// Paints `scene` into a new pixmap of the scene's size. The background
    /// is forced opaque, so the (premultiplied) pixel data is also valid
    /// straight-alpha RGBA.
    pub fn render(&mut self, scene: &Scene) -> anyhow::Result<Pixmap> {
        self.render_scaled(scene, 1.0)
    }

    /// Like [`render`](Self::render), but at `scale` × the scene size (e.g.
    /// 0.5 for cheap frames while animating; the terminal scales the image
    /// back up to fill the same cells).
    pub fn render_scaled(&mut self, scene: &Scene, scale: f32) -> anyhow::Result<Pixmap> {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let (w, h) = scaled_size(scene, scale);
        let mut pm = Pixmap::new(w, h).context("allocating pixmap")?;
        let bg = scene.background;
        pm.fill(tiny_skia::Color::from_rgba8(bg.r, bg.g, bg.b, 255));
        let tf = Transform::from_scale(scale, scale);

        for shape in &scene.shapes {
            match shape {
                Shape::Line { a, b, width, color } => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(a.x, a.y);
                    pb.line_to(b.x, b.y);
                    if let Some(path) = pb.finish() {
                        let stroke = Stroke {
                            // Keep hairlines visible when downscaled.
                            width: width.max(0.5 / scale),
                            line_cap: LineCap::Round,
                            ..Stroke::default()
                        };
                        pm.stroke_path(&path, &paint(*color), &stroke, tf, None);
                    }
                }
                Shape::Circle {
                    center,
                    radius,
                    fill,
                    stroke,
                } => {
                    if let Some(path) =
                        PathBuilder::from_circle(center.x, center.y, radius.max(0.5))
                    {
                        pm.fill_path(&path, &paint(*fill), FillRule::Winding, tf, None);
                        if let Some((w, c)) = stroke {
                            let s = Stroke {
                                width: *w,
                                ..Stroke::default()
                            };
                            pm.stroke_path(&path, &paint(*c), &s, tf, None);
                        }
                    }
                }
                Shape::Text {
                    pos,
                    text,
                    size,
                    color,
                    anchor,
                } => {
                    let (x, y, size) = (pos.x * scale, pos.y * scale, size * scale);
                    // Below ~5px glyphs are unreadable noise; skip them.
                    if size >= 5.0 {
                        self.draw_text(&mut pm, x, y, text, size, *color, *anchor);
                    }
                }
            }
        }
        Ok(pm)
    }

    pub fn render_png(&mut self, scene: &Scene, speed: PngSpeed) -> anyhow::Result<Vec<u8>> {
        self.render_png_scaled(scene, 1.0, speed)
    }

    pub fn render_png_scaled(
        &mut self,
        scene: &Scene,
        scale: f32,
        speed: PngSpeed,
    ) -> anyhow::Result<Vec<u8>> {
        let pm = self.render_scaled(scene, scale)?;
        encode_png(&pm, speed)
    }

    fn glyph(&mut self, c: char, size: f32) -> &(fontdue::Metrics, Vec<u8>) {
        let key = (c, (size * 4.0).round() as u32);
        let font = &self.font;
        self.glyphs
            .entry(key)
            .or_insert_with(|| font.rasterize(c, key.1 as f32 / 4.0))
    }

    /// Width of `text` at `size` px.
    pub fn measure(&mut self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.glyph(c, size).0.advance_width)
            .sum()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        pm: &mut Pixmap,
        x: f32,
        y: f32,
        text: &str,
        size: f32,
        color: Color,
        anchor: Anchor,
    ) {
        let ascent = self
            .font
            .horizontal_line_metrics(size)
            .map_or(size * 0.8, |m| m.ascent);
        let width = self.measure(text, size);
        let mut pen_x = match anchor {
            Anchor::TopCenter => x - width / 2.0,
            Anchor::TopLeft => x,
        };
        let baseline = y + ascent;
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let data = pm.data_mut();
        for c in text.chars() {
            let (m, cov) = self.glyph(c, size);
            let gx = (pen_x + m.xmin as f32).round() as i32;
            let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
            for row in 0..m.height as i32 {
                let py = gy + row;
                if py < 0 || py >= ph {
                    continue;
                }
                for col in 0..m.width as i32 {
                    let px = gx + col;
                    if px < 0 || px >= pw {
                        continue;
                    }
                    let coverage = cov[(row * m.width as i32 + col) as usize];
                    if coverage == 0 {
                        continue;
                    }
                    let a = coverage as f32 / 255.0 * color.a as f32 / 255.0;
                    let i = ((py * pw + px) * 4) as usize;
                    // Source-over on premultiplied RGBA.
                    for (ch, src) in [color.r, color.g, color.b].into_iter().enumerate() {
                        data[i + ch] =
                            (src as f32 * a + data[i + ch] as f32 * (1.0 - a)).round() as u8;
                    }
                    data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)).round() as u8;
                }
            }
            pen_x += m.advance_width;
        }
    }
}

/// Pixel size of a scene rendered at `scale` (at least 1×1).
pub fn scaled_size(scene: &Scene, scale: f32) -> (u32, u32) {
    let s = |v: u32| ((v as f32 * scale).round() as u32).max(1);
    (s(scene.width), s(scene.height))
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.r, c.g, c.b, c.a);
    p.anti_alias = true;
    p
}

/// Encodes an opaque pixmap as PNG.
///
/// `Fast` is tuned for live frames (measured in `examples/frame_cost.rs`):
/// the pixmap's RGBA is encoded directly (no RGB copy; the constant alpha
/// compresses to nothing) with the `Sub` row filter, ~3× faster than the
/// adaptive filter at almost the same size. `Small` is for files on disk.
pub fn encode_png(pm: &Pixmap, speed: PngSpeed) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(pm.data().len() / 16);
    let mut enc = png::Encoder::new(&mut out, pm.width(), pm.height());
    enc.set_depth(png::BitDepth::Eight);
    match speed {
        PngSpeed::Fast => {
            enc.set_color(png::ColorType::Rgba);
            enc.set_compression(png::Compression::Fast);
            enc.set_filter(png::Filter::Sub);
            let mut w = enc.write_header()?;
            w.write_image_data(pm.data())?;
            w.finish()?;
        }
        PngSpeed::Small => {
            enc.set_color(png::ColorType::Rgb);
            enc.set_compression(png::Compression::High);
            let rgb: Vec<u8> = pm
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, _]| [r, g, b])
                .collect();
            let mut w = enc.write_header()?;
            w.write_image_data(&rgb)?;
            w.finish()?;
        }
    }
    Ok(out)
}
