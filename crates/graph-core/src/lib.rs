//! `graph-core` — the rendering-agnostic heart of force-term-graph.
//!
//! This crate deliberately has **no** I/O, terminal, windowing or GPU
//! dependencies. Frontends (terminal pane, egui window, PNG export) consume a
//! [`scene::Scene`] and feed back [`app::AppEvent`]s; nothing in here knows how
//! pixels reach the screen.

pub mod app;
pub mod camera;
pub mod data;
pub mod layout;
pub mod math;
pub mod model;
pub mod physics;
pub mod scene;

pub use app::{AppEvent, AppState, MouseButton};
pub use camera::Camera;
pub use data::{EdgeSpec, GraphData, NodeSpec};
pub use layout::Layout;
pub use math::{Rect, Vec2};
pub use model::{EdgeData, EdgeKind, Graph, NodeData, NodeId};
pub use physics::{PhysicsParams, Simulation};
pub use scene::{Anchor, Color, Highlight, Scene, Shape, Style, build_scene};
