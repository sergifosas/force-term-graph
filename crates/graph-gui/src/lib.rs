//! Native window frontend: paints the same [`graph_core::Scene`] as the
//! terminal pane, but with egui's GPU painter (wgpu). Nothing is rasterised
//! on the CPU or sent through a terminal, so it stays sharp at any size and
//! animates at the display's refresh rate.
//!
//! Like the pane, it only redraws while something moves; when the layout is
//! settled and there is no input, it sleeps.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};
use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Key, Pos2, Sense, Stroke,
};
use graph_core::{Anchor, AppEvent, AppState, Color, MouseButton, Shape, Vec2};

pub const HELP: &str =
    "drag move · scroll zoom · arrows pan · f fit · r relayout · esc deselect · q quit";

#[derive(Debug, Clone)]
pub struct WindowOptions {
    pub title: String,
    /// Show fps and frame timings in the status bar.
    pub stats: bool,
    pub size: (f32, f32),
    /// Benchmark: keep the layout moving for this long, print frame stats
    /// to stderr, then close.
    pub bench: Option<std::time::Duration>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "graph".into(),
            stats: false,
            size: (1400.0, 900.0),
            bench: None,
        }
    }
}

/// Opens the window and runs until it is closed.
pub fn run_window(app: AppState, opts: WindowOptions) -> Result<()> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&opts.title)
            .with_inner_size([opts.size.0, opts.size.1]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let title = opts.title.clone();
    eframe::run_native(
        &title,
        native,
        Box::new(move |cc| {
            install_font(&cc.egui_ctx);
            Ok(Box::new(GraphWindow::new(app, opts)))
        }),
    )
    .map_err(|e| anyhow!("window error: {e}"))
}

fn install_font(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "fira".into(),
        Arc::new(FontData::from_static(graph_render::LABEL_FONT)),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "fira".into());
    }
    ctx.set_fonts(fonts);
}

struct GraphWindow {
    app: AppState,
    opts: WindowOptions,
    last_pointer: Option<Pos2>,
    last_frame: Instant,
    frame_ms: f32,
    /// (first frame time, frames drawn) for `--bench`.
    bench: Option<(Instant, u64)>,
}

impl GraphWindow {
    fn new(app: AppState, opts: WindowOptions) -> Self {
        Self {
            app,
            opts,
            last_pointer: None,
            last_frame: Instant::now(),
            frame_ms: 0.0,
            bench: None,
        }
    }

    /// Bench bookkeeping; returns true when the run is over.
    fn bench_tick(&mut self, size: Vec2, ppp: f32) -> bool {
        let Some(limit) = self.opts.bench else {
            return false;
        };
        if self.app.sim.alpha() < 0.5 {
            self.app.sim.reheat(1.0);
        }
        let (start, frames) = self.bench.get_or_insert((Instant::now(), 0));
        *frames += 1;
        let elapsed = start.elapsed();
        if elapsed < limit {
            return false;
        }
        eprintln!(
            "window {:.0}x{:.0} px · {} frames in {:.1}s = {:.1} fps",
            size.x * ppp,
            size.y * ppp,
            *frames,
            elapsed.as_secs_f64(),
            (*frames - 1) as f64 / elapsed.as_secs_f64()
        );
        // Report once; the close request may still let another frame run.
        self.opts.bench = None;
        true
    }

    /// Translates egui input into AppEvents (coordinates relative to the
    /// canvas). Returns true if the user asked to quit.
    fn handle_input(&mut self, ctx: &egui::Context, canvas: egui::Rect, hovered: bool) -> bool {
        let to_local = |p: Pos2| Vec2::new(p.x - canvas.min.x, p.y - canvas.min.y);
        let mut events = Vec::new();
        let mut quit = false;

        ctx.input(|i| {
            let pointer = i.pointer.hover_pos();
            if pointer != self.last_pointer {
                if let Some(p) = pointer {
                    events.push(AppEvent::PointerMove(to_local(p)));
                }
                self.last_pointer = pointer;
            }
            let origin = i.pointer.press_origin().or(pointer);
            if hovered && let Some(o) = origin {
                if i.pointer.primary_pressed() {
                    events.push(AppEvent::PointerDown(to_local(o), MouseButton::Left));
                }
                if i.pointer.secondary_pressed()
                    || i.pointer.button_pressed(egui::PointerButton::Middle)
                {
                    events.push(AppEvent::PointerDown(to_local(o), MouseButton::Right));
                }
            }
            if i.pointer.any_released() {
                let p = pointer.map(to_local).unwrap_or_default();
                events.push(AppEvent::PointerUp(p, MouseButton::Left));
            }

            if hovered && let Some(p) = pointer {
                // Wheel: ~50 points per notch. Pinch / ctrl+wheel: zoom_delta.
                let notches = i.smooth_scroll_delta.y / 50.0 + i.zoom_delta().ln() / 1.15f32.ln();
                if notches.abs() > 1e-3 {
                    events.push(AppEvent::Scroll(to_local(p), notches));
                }
            }

            let pan = 60.0;
            for (key, ev) in [
                (Key::F, AppEvent::FitView),
                (Key::R, AppEvent::Reheat),
                (Key::Plus, AppEvent::Zoom(2.0)),
                (Key::Equals, AppEvent::Zoom(2.0)),
                (Key::Minus, AppEvent::Zoom(-2.0)),
                (Key::ArrowLeft, AppEvent::PanBy(Vec2::new(pan, 0.0))),
                (Key::ArrowRight, AppEvent::PanBy(Vec2::new(-pan, 0.0))),
                (Key::ArrowUp, AppEvent::PanBy(Vec2::new(0.0, pan))),
                (Key::ArrowDown, AppEvent::PanBy(Vec2::new(0.0, -pan))),
            ] {
                if i.key_pressed(key) {
                    events.push(ev);
                }
            }
            if i.key_pressed(Key::Q) {
                quit = true;
            }
            if i.key_pressed(Key::Escape) {
                if self.app.highlight.selected.is_some() {
                    events.push(AppEvent::ClearSelection);
                } else {
                    quit = true;
                }
            }
        });

        for ev in events {
            self.app.update(ev);
        }
        quit
    }

    fn status_text(&self) -> String {
        let left = match self.app.focused_node() {
            Some(id) => {
                let n = self.app.graph.node(id);
                let kind = if n.ghost { "unresolved" } else { &n.category };
                let degree = self.app.graph.degree(id);
                let plural = if degree == 1 { "" } else { "s" };
                format!("● {} · {kind} · {degree} link{plural}", n.label)
            }
            None => format!(
                "{} nodes · {} edges",
                self.app.graph.node_count(),
                self.app.graph.edge_count()
            ),
        };
        if self.opts.stats {
            let fps = if self.frame_ms > 0.0 {
                1000.0 / self.frame_ms
            } else {
                0.0
            };
            format!("{fps:.0} fps · {:.1} ms · {left}   {HELP}", self.frame_ms)
        } else {
            format!("{left}   {HELP}")
        }
    }
}

impl eframe::App for GraphWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        let ms = dt * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            ms
        } else {
            self.frame_ms * 0.9 + ms * 0.1
        };

        let bg = to_color32(color_of(&self.app.style.background));
        let status = self.status_text();
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(bg).inner_margin(6.0))
            .show(ui, |ui| {
                ui.colored_label(Color32::from_gray(130), status);
            });

        let mut quit = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(bg))
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                let size = Vec2::new(rect.width(), rect.height());
                if self.app.camera.viewport != size {
                    self.app.update(AppEvent::Resize(size.x, size.y));
                }
                quit = self.handle_input(&ctx, rect, response.hovered() || response.dragged());
                quit |= self.bench_tick(size, ctx.pixels_per_point());
                self.app.tick(dt.min(0.25));
                paint(
                    &ui.painter_at(rect),
                    rect.min,
                    &self.app.scene_interpolated(),
                );
            });

        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Keep animating while something moves; otherwise sleep until input.
        if self.app.is_animating() || self.app.is_dragging() {
            ctx.request_repaint();
        }
    }
}

fn paint(painter: &egui::Painter, origin: Pos2, scene: &graph_core::Scene) {
    let at = |v: Vec2| Pos2::new(origin.x + v.x, origin.y + v.y);
    for shape in &scene.shapes {
        match shape {
            Shape::Line { a, b, width, color } => {
                painter.line_segment([at(*a), at(*b)], Stroke::new(*width, to_color32(*color)));
            }
            Shape::Circle {
                center,
                radius,
                fill,
                stroke,
            } => {
                let stroke = stroke.map_or(Stroke::NONE, |(w, c)| Stroke::new(w, to_color32(c)));
                painter.circle(at(*center), *radius, to_color32(*fill), stroke);
            }
            Shape::Text {
                pos,
                text,
                size,
                color,
                anchor,
            } => {
                let align = match anchor {
                    Anchor::TopCenter => Align2::CENTER_TOP,
                    Anchor::TopLeft => Align2::LEFT_TOP,
                };
                painter.text(
                    at(*pos),
                    align,
                    text,
                    FontId::proportional(*size),
                    to_color32(*color),
                );
            }
        }
    }
}

fn to_color32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn color_of(hex: &str) -> Color {
    Color::from_hex(hex).unwrap_or(Color::rgb(30, 30, 36))
}
