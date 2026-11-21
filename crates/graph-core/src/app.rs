//! Frontend-independent interaction state. Frontends translate native input
//! into [`AppEvent`]s, call [`AppState::update`] / [`AppState::tick`], and
//! paint [`AppState::scene`] whenever something changed.

use crate::camera::Camera;
use crate::layout::Layout;
use crate::math::Vec2;
use crate::model::{Graph, NodeId};
use crate::physics::{PhysicsParams, Simulation};
use crate::scene::{Highlight, Scene, Style, build_scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Input in screen pixels, already mapped by the frontend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppEvent {
    PointerMove(Vec2),
    PointerDown(Vec2, MouseButton),
    PointerUp(Vec2, MouseButton),
    /// Positive = zoom in. One notch ≈ 1.0.
    Scroll(Vec2, f32),
    Resize(f32, f32),
    /// Re-frame the whole graph.
    FitView,
    /// Moves the view by a screen-space delta (keyboard panning).
    PanBy(Vec2),
    /// Zooms around the viewport centre (keyboard zoom). Same units as
    /// `Scroll`.
    Zoom(f32),
    /// Re-energises the simulation so the layout re-arranges.
    Reheat,
    ClearSelection,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    None,
    /// Dragging a node; it is pinned while held. `offset` keeps the grab
    /// point under the cursor instead of snapping the centre to it.
    Node {
        id: NodeId,
        was_pinned: bool,
        offset: Vec2,
    },
    /// Panning the canvas from the last pointer position.
    Pan {
        last: Vec2,
    },
}

/// Screen padding when fitting the graph: room for labels under edge nodes.
const FIT_PADDING: f32 = 64.0;

pub struct AppState {
    pub graph: Graph,
    pub layout: Layout,
    pub sim: Simulation,
    pub camera: Camera,
    pub style: Style,
    pub highlight: Highlight,
    /// Extra hit-test tolerance in screen pixels. Frontends with coarse
    /// pointer positions (terminal cells) raise it.
    pub hit_slop: f32,
    drag: Drag,
    dirty: bool,
    /// Keep the camera fitted to the graph until the user takes control.
    auto_fit: bool,
}

impl AppState {
    pub fn new(graph: Graph, params: PhysicsParams, style: Style, viewport: (f32, f32)) -> Self {
        let layout = Layout::for_graph(&graph, params.initial_spacing);
        let mut camera = Camera::new(viewport.0, viewport.1);
        camera.fit_layout(&layout, FIT_PADDING);
        Self {
            graph,
            layout,
            sim: Simulation::new(params),
            camera,
            style,
            highlight: Highlight::default(),
            hit_slop: 0.0,
            drag: Drag::None,
            dirty: true,
            auto_fit: true,
        }
    }

    /// Runs the simulation to rest without rendering (for static output).
    pub fn settle(&mut self, max_steps: usize) -> usize {
        let n = self
            .sim
            .run_until_settled(&self.graph, &mut self.layout, max_steps);
        self.fit_view();
        n
    }

    pub fn fit_view(&mut self) {
        self.camera.fit_layout(&self.layout, FIT_PADDING);
        self.dirty = true;
    }

    fn node_at(&self, p: Vec2) -> Option<NodeId> {
        let (g, s, c) = (&self.graph, &self.style, &self.camera);
        c.hit_test(&self.layout, p, |id| {
            s.screen_radius(g, id, c) + self.hit_slop
        })
    }

    /// True while the layout is still moving; frontends should keep
    /// producing frames until this (and `tick`) say otherwise.
    pub fn is_animating(&self) -> bool {
        !self.sim.is_settled()
    }

    /// True while a node or the canvas is being dragged.
    pub fn is_dragging(&self) -> bool {
        self.drag != Drag::None
    }

    /// The node the user is looking at: hovered, else selected.
    pub fn focused_node(&self) -> Option<NodeId> {
        self.highlight.hovered.or(self.highlight.selected)
    }

    pub fn update(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::PointerMove(p) => match self.drag {
                Drag::Node { id, offset, .. } => {
                    let w = self.camera.screen_to_world(p) + offset;
                    self.layout.set_position(id, w);
                    self.dirty = true;
                }
                Drag::Pan { last } => {
                    self.camera.pan_screen(p - last);
                    self.drag = Drag::Pan { last: p };
                    self.auto_fit = false;
                    self.dirty = true;
                }
                Drag::None => {
                    let hovered = self.node_at(p);
                    if hovered != self.highlight.hovered {
                        self.highlight.hovered = hovered;
                        self.dirty = true;
                    }
                }
            },
            AppEvent::PointerDown(p, MouseButton::Left) => {
                if let Some(id) = self.node_at(p) {
                    let was_pinned = self.layout.pinned[id.index()];
                    self.layout.set_pinned(id, true);
                    self.highlight.selected = Some(id);
                    let offset = self.layout.position(id) - self.camera.screen_to_world(p);
                    self.drag = Drag::Node {
                        id,
                        was_pinned,
                        offset,
                    };
                    self.sim.set_alpha_target(0.3);
                    self.auto_fit = false;
                } else {
                    self.highlight.selected = None;
                    self.drag = Drag::Pan { last: p };
                }
                self.dirty = true;
            }
            AppEvent::PointerDown(p, _) => {
                self.drag = Drag::Pan { last: p };
            }
            AppEvent::PointerUp(..) => {
                if let Drag::Node { id, was_pinned, .. } = self.drag {
                    self.layout.set_pinned(id, was_pinned);
                    self.sim.set_alpha_target(0.0);
                }
                self.drag = Drag::None;
            }
            AppEvent::Scroll(p, notches) => {
                self.camera.zoom_at(p, 1.15f32.powf(notches));
                self.auto_fit = false;
                self.dirty = true;
            }
            AppEvent::Resize(w, h) => {
                self.camera.resize(w, h);
                if self.auto_fit {
                    self.camera.fit_layout(&self.layout, FIT_PADDING);
                }
                self.dirty = true;
            }
            AppEvent::FitView => {
                self.auto_fit = true;
                self.fit_view();
            }
            AppEvent::Zoom(notches) => {
                let centre = self.camera.viewport * 0.5;
                self.update(AppEvent::Scroll(centre, notches));
            }
            AppEvent::PanBy(delta) => {
                self.camera.pan_screen(delta);
                self.auto_fit = false;
                self.dirty = true;
            }
            AppEvent::Reheat => {
                self.sim.reheat(1.0);
                self.dirty = true;
            }
            AppEvent::ClearSelection => {
                if self.highlight.selected.take().is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// Advances physics by `dt` seconds. Returns `true` if a redraw is
    /// needed; once the layout settles and nothing changes, this returns
    /// `false` and frontends should stop rendering.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.sim.advance(&self.graph, &mut self.layout, dt) {
            if self.auto_fit {
                self.camera.fit_layout(&self.layout, FIT_PADDING);
            }
            self.dirty = true;
        }
        std::mem::take(&mut self.dirty)
    }

    pub fn scene(&self) -> Scene {
        build_scene(
            &self.graph,
            &self.layout,
            &self.camera,
            self.highlight,
            &self.style,
        )
    }

    /// Like [`scene`](Self::scene), but with positions interpolated between
    /// physics steps — for frontends that draw faster than the physics rate
    /// (the GPU window at 120/144 Hz).
    pub fn scene_interpolated(&self) -> Scene {
        match self.sim.interpolated(&self.layout) {
            Some(pos) => {
                let blended = Layout {
                    pos,
                    vel: Vec::new(),
                    pinned: Vec::new(),
                };
                build_scene(
                    &self.graph,
                    &blended,
                    &self.camera,
                    self.highlight,
                    &self.style,
                )
            }
            None => self.scene(),
        }
    }
}
