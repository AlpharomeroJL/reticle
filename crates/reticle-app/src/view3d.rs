//! The 3D layer-stack panel: orbit-camera state and the egui-wgpu paint glue.
//!
//! The pure part of this module is [`View3d`], a tiny state machine holding the
//! [`OrbitCamera`]: it frames the scene on first sight, applies drag/scroll
//! deltas with pixel-to-radian and scroll-to-zoom mappings, and can be asked to
//! re-frame. That part is unit-tested without any GPU or window.
//!
//! The glue part ([`View3d::show`]) draws a floating egui window containing the
//! 3D viewport. Rendering goes through an `egui-wgpu` paint callback: `prepare`
//! renders the extruded scene into `reticle-render`'s [`StackView`] offscreen
//! color+depth target (egui's own render pass has no depth attachment), and
//! `paint` blits the finished frame into egui's pass. The scene mesh is rebuilt
//! every frame from the flattened top cell; this is an inspection view, so
//! simplicity wins over retained-scene plumbing, and layer visibility follows
//! the layer panel. Works on both native and web builds of the app (both run
//! the `wgpu` eframe backend); if the render state is missing the panel shows a
//! note instead of a viewport.

use eframe::egui;
use egui_wgpu::wgpu;

use reticle_model::{Document, Technology};
use reticle_render::{Mesh3d, OrbitCamera, Palette, Rgba, StackView, layer_spans};

use crate::layers::LayerState;

/// Radians of orbit per dragged pixel.
const ORBIT_PER_PIXEL: f32 = 0.01;

/// The zoom rate per scroll unit; matches the canvas zoom feel.
const ZOOM_PER_SCROLL: f32 = 0.0015;

/// The 3D viewport clear color (dark, close to the 2D canvas background).
const VIEW_CLEAR: u32 = 0x1012_16ff;

/// The pure state of the 3D view: the orbit camera and its framing latch.
#[derive(Clone, Copy, Debug, Default)]
pub struct View3d {
    /// The current camera; `None` until the first non-empty scene frames it.
    camera: Option<OrbitCamera>,
}

impl View3d {
    /// Creates the view; the camera frames itself on the first non-empty scene.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current camera, if one has been framed.
    #[must_use]
    pub fn camera(&self) -> Option<&OrbitCamera> {
        self.camera.as_ref()
    }

    /// Drops the camera so the next scene re-frames it (the "reset view" action).
    pub fn reset(&mut self) {
        self.camera = None;
    }

    /// Returns the camera, framing `mesh` first if none exists yet. `None` for
    /// an empty mesh with no prior camera (nothing sensible to frame).
    pub fn ensure_camera(&mut self, mesh: &Mesh3d) -> Option<OrbitCamera> {
        if self.camera.is_none() {
            self.camera = mesh.bounds().map(OrbitCamera::framing);
        }
        self.camera
    }

    /// Applies a pointer drag of `(dx, dy)` pixels: horizontal drag yaws,
    /// vertical drag pitches (drag up looks from higher above). No-op before
    /// the first framing.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        if let Some(camera) = &mut self.camera {
            camera.orbit(-dx * ORBIT_PER_PIXEL, dy * ORBIT_PER_PIXEL);
        }
    }

    /// Applies a scroll of `delta` units: scrolling up (positive) zooms in.
    /// No-op before the first framing.
    pub fn scroll(&mut self, delta: f32) {
        if let Some(camera) = &mut self.camera {
            camera.zoom((-delta * ZOOM_PER_SCROLL).exp());
        }
    }
}

/// Builds the palette for the 3D view: the document's layer colors with the
/// layer panel's visibility applied on top, so hiding a layer in the table also
/// removes its slab from the 3D view.
#[must_use]
pub fn palette_with_visibility(tech: &Technology, layers: &LayerState) -> Palette {
    let mut tech = tech.clone();
    for info in &mut tech.layers {
        info.visible = layers.is_visible(info.id);
    }
    Palette::from_technology(&tech)
}

/// The per-frame paint callback: plain data handed to `egui-wgpu`.
///
/// `prepare` renders the mesh into the shared [`StackView`] (stored in the
/// renderer's callback resources), `paint` blits that frame into egui's pass.
struct StackCallback {
    mesh: Mesh3d,
    camera: OrbitCamera,
    /// The viewport size in egui points; scaled by `pixels_per_point` at
    /// prepare time.
    size_points: [f32; 2],
}

impl egui_wgpu::CallbackTrait for StackCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(view) = callback_resources.get_mut::<StackView>() {
            let ppp = screen_descriptor.pixels_per_point;
            let size = (
                (self.size_points[0] * ppp).round().max(1.0) as u32,
                (self.size_points[1] * ppp).round().max(1.0) as u32,
            );
            view.prepare(
                device,
                egui_encoder,
                size,
                &self.mesh,
                &self.camera,
                Rgba::from_packed(VIEW_CLEAR),
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(view) = callback_resources.get::<StackView>() {
            view.paint(render_pass);
        }
    }
}

impl View3d {
    /// Draws the floating "3D stack" window: viewport, orbit/zoom input, and
    /// the reset button. Call once per frame from the app.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        doc: &Document,
        top_cell: &str,
        layers: &LayerState,
    ) {
        // Start collapsed and tucked below the toolbar (right of the Layers panel) so
        // the app opens on the canvas, not on two fully-expanded tool windows. The user
        // expands and drags it from there; egui persists the choice per session.
        egui::Window::new("3D stack")
            .default_size([440.0, 380.0])
            .default_open(false)
            .default_pos([200.0, 60.0])
            .resizable(true)
            .show(ctx, |ui| {
                self.show_contents(ui, frame, doc, top_cell, layers);
            });
    }

    /// The window body: controls plus the wgpu-painted viewport.
    fn show_contents(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        doc: &Document,
        top_cell: &str,
        layers: &LayerState,
    ) {
        let Some(render_state) = frame.wgpu_render_state() else {
            ui.label("3D view needs the wgpu render backend.");
            return;
        };

        // Rebuild the scene every frame: inspection view, no retained state.
        let shapes = doc.flatten(top_cell);
        let spans = layer_spans(doc.technology(), &shapes);
        let palette = palette_with_visibility(doc.technology(), layers);
        let mesh = Mesh3d::build(&shapes, &spans, &palette);

        ui.horizontal(|ui| {
            if ui.button("Reset view").clicked() {
                self.reset();
            }
            ui.label("Drag to orbit, scroll to zoom.");
        });

        let Some(camera) = self.ensure_camera(&mesh) else {
            ui.label("Nothing to show: the scene is empty.");
            return;
        };

        // Lazily park the shared GPU resources in egui's callback resources.
        {
            let mut renderer = render_state.renderer.write();
            if renderer.callback_resources.get::<StackView>().is_none() {
                renderer.callback_resources.insert(StackView::new(
                    &render_state.device,
                    render_state.target_format,
                ));
            }
        }

        let size = ui.available_size().max(egui::Vec2::new(240.0, 200.0));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());

        // Orbit on drag, zoom on scroll (hover only, so other panels keep
        // their scrolling).
        if response.dragged() {
            let delta = response.drag_delta();
            self.drag(delta.x, delta.y);
        }
        if response.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                self.scroll(scroll);
            }
        }
        // Use the camera as updated by this frame's input.
        let camera = self.camera().copied().unwrap_or(camera);

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            StackCallback {
                mesh,
                camera,
                size_points: [rect.width(), rect.height()],
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{DrawShape, LayerInfo, ShapeKind};
    use reticle_render::LayerSpan;

    fn one_rect_mesh() -> Mesh3d {
        let layer = LayerId::new(1, 0);
        let shapes = [DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 100))),
        )];
        let tech = test_technology(true);
        let palette = Palette::from_technology(&tech);
        let spans = vec![LayerSpan {
            layer,
            z_bottom: 0.0,
            z_top: 10.0,
        }];
        Mesh3d::build(&shapes, &spans, &palette)
    }

    fn test_technology(visible: bool) -> Technology {
        Technology {
            name: "t".to_owned(),
            dbu_per_micron: 1000,
            layers: vec![LayerInfo {
                id: LayerId::new(1, 0),
                name: "M1".to_owned(),
                color_rgba: 0xff00_00ff,
                visible,
            }],
            rules: Vec::new(),
            stack: Vec::new(),
        }
    }

    #[test]
    fn frames_once_then_keeps_user_camera() {
        let mut view = View3d::new();
        assert!(view.camera().is_none());
        let mesh = one_rect_mesh();
        let framed = view.ensure_camera(&mesh).expect("frames a non-empty mesh");
        // User orbits; the next ensure_camera must not re-frame.
        view.drag(30.0, -12.0);
        let after = view.ensure_camera(&mesh).expect("camera persists");
        assert!((after.yaw - framed.yaw).abs() > 1e-6, "yaw must move");
        assert!((after.pitch - framed.pitch).abs() > 1e-6, "pitch must move");
        assert!(
            (after.distance - framed.distance).abs() < 1e-6,
            "drag must not zoom"
        );
    }

    #[test]
    fn empty_mesh_gives_no_camera_until_reset_scene_appears() {
        let mut view = View3d::new();
        assert!(view.ensure_camera(&Mesh3d::default()).is_none());
        // Input before framing is a harmless no-op.
        view.drag(10.0, 10.0);
        view.scroll(5.0);
        assert!(view.camera().is_none());
        // Once a scene exists it frames normally.
        assert!(view.ensure_camera(&one_rect_mesh()).is_some());
    }

    #[test]
    fn scroll_up_zooms_in_and_pitch_stays_clamped() {
        let mut view = View3d::new();
        view.ensure_camera(&one_rect_mesh());
        let d0 = view.camera().unwrap().distance;
        view.scroll(120.0);
        let d1 = view.camera().unwrap().distance;
        assert!(d1 < d0, "scrolling up must zoom in");
        // A huge upward drag cannot push the pitch past the clamp.
        view.drag(0.0, 100_000.0);
        assert!(view.camera().unwrap().pitch <= OrbitCamera::MAX_PITCH + 1e-6);
    }

    #[test]
    fn reset_reframes_on_next_scene() {
        let mut view = View3d::new();
        let mesh = one_rect_mesh();
        let framed = view.ensure_camera(&mesh).unwrap();
        view.drag(50.0, 0.0);
        view.reset();
        assert!(view.camera().is_none());
        let reframed = view.ensure_camera(&mesh).unwrap();
        assert!(
            (reframed.yaw - framed.yaw).abs() < 1e-6,
            "reset restores the framing view"
        );
    }

    #[test]
    fn palette_visibility_follows_the_layer_panel() {
        let tech = test_technology(true);
        let mut layers = LayerState::from_technology(&tech);
        let id = LayerId::new(1, 0);
        assert!(palette_with_visibility(&tech, &layers).is_visible(id));
        layers.set_visible(id, false);
        assert!(!palette_with_visibility(&tech, &layers).is_visible(id));
    }
}
