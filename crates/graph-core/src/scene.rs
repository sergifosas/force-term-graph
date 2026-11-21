//! Backend-agnostic draw list. `build_scene` owns *all* styling decisions
//! (colours, sizes, label culling, hover highlighting) so frontends are thin
//! painters: tiny-skia for the terminal/PNG, egui for the window.

use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::layout::Layout;
use crate::math::Vec2;
use crate::model::{Graph, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Parses `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        let byte = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Color {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => None,
        }
    }

    fn scale_alpha(self, f: f32) -> Self {
        self.with_alpha((self.a as f32 * f).round().clamp(0.0, 255.0) as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `pos` is the top-centre of the text box.
    TopCenter,
    /// `pos` is the top-left of the text box.
    TopLeft,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Vec2,
        b: Vec2,
        width: f32,
        color: Color,
    },
    Circle {
        center: Vec2,
        radius: f32,
        fill: Color,
        stroke: Option<(f32, Color)>,
    },
    Text {
        pos: Vec2,
        text: String,
        size: f32,
        color: Color,
        anchor: Anchor,
    },
}

/// Screen-space draw list, painted in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub shapes: Vec<Shape>,
}

/// Visual theme. Deserializable from the `[style]` config section; colours
/// are hex strings there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub background: String,
    pub edge: String,
    pub edge_highlight: String,
    pub label: String,
    pub ghost: String,
    pub selection: String,
    /// Category colours, assigned by a stable hash of the category name.
    pub palette: Vec<String>,
    /// Base node radius in world units.
    pub node_radius: f32,
    /// Extra radius per sqrt(degree), world units.
    pub node_radius_per_degree: f32,
    /// Minimum on-screen radius in pixels.
    pub min_screen_radius: f32,
    pub edge_width: f32,
    pub label_size: f32,
    /// Labels are shown for all nodes once zoom reaches this value.
    pub label_min_zoom: f32,
    /// Graphs this small always show every label.
    pub always_label_below: usize,
    /// Opacity of non-highlighted items while hovering (0..1).
    pub dim_factor: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: "#1e1e24".into(),
            edge: "#4a4a5a".into(),
            edge_highlight: "#a68af9".into(),
            label: "#dcddde".into(),
            ghost: "#5c5c6a".into(),
            selection: "#ffffff".into(),
            palette: [
                "#a68af9", "#5fb3f9", "#62d6a3", "#f7c96b", "#f78c6b", "#ef6b9a", "#9ad06b",
                "#6bd6e0",
            ]
            .map(String::from)
            .to_vec(),
            node_radius: 5.0,
            node_radius_per_degree: 1.6,
            min_screen_radius: 2.0,
            edge_width: 1.0,
            label_size: 13.0,
            label_min_zoom: 1.4,
            always_label_below: 60,
            dim_factor: 0.25,
        }
    }
}

impl Style {
    /// Node radius in world units (grows with degree, like Obsidian).
    pub fn world_radius(&self, graph: &Graph, id: NodeId) -> f32 {
        self.node_radius + self.node_radius_per_degree * (graph.degree(id) as f32).sqrt()
    }

    pub fn screen_radius(&self, graph: &Graph, id: NodeId, camera: &Camera) -> f32 {
        (self.world_radius(graph, id) * camera.zoom).max(self.min_screen_radius)
    }

    pub fn category_color(&self, category: &str) -> Color {
        if self.palette.is_empty() {
            return Color::rgb(160, 160, 170);
        }
        let idx = (fnv1a(category.as_bytes()) % self.palette.len() as u64) as usize;
        hex(&self.palette[idx])
    }
}

/// Interaction state that affects drawing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Highlight {
    pub hovered: Option<NodeId>,
    pub selected: Option<NodeId>,
}

impl Highlight {
    /// The node whose neighbourhood should be emphasised.
    fn focus(&self) -> Option<NodeId> {
        self.hovered.or(self.selected)
    }
}

pub fn build_scene(
    graph: &Graph,
    layout: &Layout,
    camera: &Camera,
    highlight: Highlight,
    style: &Style,
) -> Scene {
    let width = camera.viewport.x.max(1.0) as u32;
    let height = camera.viewport.y.max(1.0) as u32;
    let mut shapes = Vec::with_capacity(graph.edge_count() + graph.node_count() * 2);
    let n = layout.len().min(graph.node_count());

    let focus = highlight.focus();
    let in_focus = |id: NodeId| -> bool {
        match focus {
            None => true,
            Some(f) => f == id || graph.neighbors(f).contains(&id),
        }
    };
    let dim = |c: Color, lit: bool| {
        if lit {
            c
        } else {
            c.scale_alpha(style.dim_factor)
        }
    };

    // Cull against the viewport with a margin for big nodes and labels.
    let visible = camera.visible_world().expand(80.0 / camera.zoom);
    let edge_color = hex(&style.edge);
    let edge_hi = hex(&style.edge_highlight);
    let edge_width = (style.edge_width * camera.zoom.sqrt()).clamp(0.6, 3.0);

    for e in graph.edges() {
        let (s, t) = (e.source.index(), e.target.index());
        if s >= n || t >= n {
            continue;
        }
        let (a, b) = (layout.pos[s], layout.pos[t]);
        if !visible.overlaps_segment(a, b) {
            continue;
        }
        let incident = focus.is_some_and(|f| f == e.source || f == e.target);
        let color = if incident {
            edge_hi
        } else {
            dim(edge_color, focus.is_none())
        };
        shapes.push(Shape::Line {
            a: camera.world_to_screen(a),
            b: camera.world_to_screen(b),
            width: if incident {
                edge_width * 1.6
            } else {
                edge_width
            },
            color,
        });
    }

    let ghost = hex(&style.ghost);
    let selection = hex(&style.selection);
    let label_color = hex(&style.label);
    let show_all_labels =
        camera.zoom >= style.label_min_zoom || graph.node_count() < style.always_label_below;
    let mut labels = Vec::new();

    for (i, &p) in layout.pos.iter().enumerate().take(n) {
        let id = NodeId(i as u32);
        if !visible.contains(p) {
            continue;
        }
        let node = graph.node(id);
        let lit = in_focus(id);
        let base = if node.ghost {
            ghost
        } else {
            style.category_color(&node.category)
        };
        let r = style.screen_radius(graph, id, camera);
        let center = camera.world_to_screen(p);
        let stroke = if highlight.selected == Some(id) {
            Some((2.0, selection))
        } else if highlight.hovered == Some(id) {
            Some((1.5, selection.with_alpha(180)))
        } else {
            None
        };
        shapes.push(Shape::Circle {
            center,
            radius: r,
            fill: dim(base, lit),
            stroke,
        });

        let wants_label = show_all_labels || (focus.is_some() && lit);
        if wants_label && !node.label.is_empty() {
            let emphasis = focus == Some(id);
            labels.push(Shape::Text {
                pos: center + Vec2::new(0.0, r + 3.0),
                text: node.label.clone(),
                size: if emphasis {
                    style.label_size * 1.15
                } else {
                    style.label_size
                },
                color: dim(label_color, lit),
                anchor: Anchor::TopCenter,
            });
        }
    }
    // Labels last so they sit on top of every node.
    shapes.extend(labels);

    Scene {
        width,
        height,
        background: hex(&style.background).with_alpha(255),
        shapes,
    }
}

fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::rgb(255, 0, 255))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
