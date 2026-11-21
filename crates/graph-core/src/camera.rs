//! World ↔ screen transforms, zoom/pan and hit-testing. Shared by every
//! frontend so interaction behaves identically in a pane or a window.

use crate::layout::Layout;
use crate::math::{Rect, Vec2};
use crate::model::NodeId;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// World point shown at the viewport centre.
    pub center: Vec2,
    /// Screen pixels per world unit.
    pub zoom: f32,
    /// Viewport size in pixels.
    pub viewport: Vec2,
}

impl Camera {
    pub const MIN_ZOOM: f32 = 0.02;
    pub const MAX_ZOOM: f32 = 20.0;

    pub fn new(viewport_w: f32, viewport_h: f32) -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 1.0,
            viewport: Vec2::new(viewport_w, viewport_h),
        }
    }

    pub fn world_to_screen(&self, p: Vec2) -> Vec2 {
        (p - self.center) * self.zoom + self.viewport * 0.5
    }

    pub fn screen_to_world(&self, p: Vec2) -> Vec2 {
        (p - self.viewport * 0.5) / self.zoom + self.center
    }

    /// The world-space rectangle currently visible.
    pub fn visible_world(&self) -> Rect {
        Rect {
            min: self.screen_to_world(Vec2::ZERO),
            max: self.screen_to_world(self.viewport),
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.viewport = Vec2::new(w.max(1.0), h.max(1.0));
    }

    /// Pans by a screen-space delta (drag the canvas).
    pub fn pan_screen(&mut self, delta: Vec2) {
        self.center -= delta / self.zoom;
    }

    /// Zooms by `factor`, keeping the world point under `screen_point` fixed.
    pub fn zoom_at(&mut self, screen_point: Vec2, factor: f32) {
        let before = self.screen_to_world(screen_point);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let after = self.screen_to_world(screen_point);
        self.center += before - after;
    }

    /// Centres and zooms so `bounds` fits with `padding` pixels around it.
    pub fn fit(&mut self, bounds: Rect, padding: f32) {
        self.center = bounds.center();
        let size = bounds.size();
        let avail = (self.viewport - Vec2::new(padding * 2.0, padding * 2.0))
            .max_components(Vec2::new(1.0, 1.0));
        let zx = if size.x > 1e-3 {
            avail.x / size.x
        } else {
            f32::INFINITY
        };
        let zy = if size.y > 1e-3 {
            avail.y / size.y
        } else {
            f32::INFINITY
        };
        let z = zx.min(zy);
        self.zoom = if z.is_finite() {
            z.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM)
        } else {
            1.0
        };
    }

    pub fn fit_layout(&mut self, layout: &Layout, padding: f32) {
        if let Some(b) = layout.bounds() {
            self.fit(b, padding);
        }
    }

    /// The node under `screen_point`, if any, given each node's screen
    /// radius. Picks the closest when several overlap.
    pub fn hit_test(
        &self,
        layout: &Layout,
        screen_point: Vec2,
        radius_of: impl Fn(NodeId) -> f32,
    ) -> Option<NodeId> {
        let mut best: Option<(NodeId, f32)> = None;
        for (i, &p) in layout.pos.iter().enumerate() {
            let id = NodeId(i as u32);
            let d2 = (self.world_to_screen(p) - screen_point).length_squared();
            // Generous slop so small nodes are still easy to grab.
            let r = radius_of(id).max(6.0) + 3.0;
            if d2 <= r * r && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((id, d2));
            }
        }
        best.map(|(id, _)| id)
    }
}

impl Vec2 {
    fn max_components(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x.max(o.x), self.y.max(o.y))
    }
}
