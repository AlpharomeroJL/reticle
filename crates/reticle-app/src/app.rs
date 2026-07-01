//! The `eframe::App` implementation: panels, canvas, and interaction wiring.
//!
//! This is the only module that depends on `egui`. It owns the app state and, each
//! frame, draws the toolbar, layer manager, undo-history panel, status bar, command
//! palette window, and the layout canvas, routing pointer input through the active
//! tool. All the non-trivial logic (camera math, culling, selection, snapping,
//! measurement, history) lives in the sibling modules and is unit-tested there; the
//! code here is deliberately thin glue plus painting.

use eframe::egui;
use egui::{
    Align2, Color32, FontId, Pos2, Rect as EguiRect, Sense, Shape, Stroke, StrokeKind, Vec2,
};

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{DrawShape, ShapeKind};
use reticle_render::WgpuRenderer;
use reticle_sync::SyncDocument;

use crate::camera::{ScreenRect, ViewCamera};
use crate::command::{self, Command};
use crate::culling::{self, DetailLevel, SceneIndex};
use crate::demo;
use crate::grid::{self, GridSettings};
use crate::history::History;
use crate::layers::{self, LayerState};
use crate::selection::{self, Selection};
use crate::tool::{Tool, ToolState};

/// A transient status message shown in the bottom bar.
#[derive(Clone, Debug, Default)]
struct Status {
    /// The message text (empty means nothing to show).
    text: String,
}

impl Status {
    /// Replaces the status message.
    fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

/// The top-level application state: the collaborative document and the renderer.
///
/// The [`renderer`](App::renderer) and [`document`](App::document) accessors are the
/// frozen Wave 0 contract. Beyond them the struct now carries the full editor state:
/// the editable document with undo history, the view camera, the tool machine, the
/// layer/selection/grid models, and the command-palette UI state.
#[derive(Debug)]
pub struct App {
    /// GPU renderer handle (used by the native PNG-export action).
    renderer: WgpuRenderer,
    /// Collaboration mirror of the document (Wave 0 contract; presence overlay).
    document: SyncDocument,

    /// The editable document with undo/redo — the layout the user edits.
    history: History,
    /// The world<->screen camera.
    camera: ViewCamera,
    /// Whether the camera should fit the design on the next frame (deferred so the
    /// real canvas size is known).
    fit_requested: bool,
    /// The tool state machine.
    tools: ToolState,
    /// Layer table, visibility, and filter.
    layer_state: LayerState,
    /// The current shape selection (indices into the scene).
    selection: Selection,
    /// Grid, snapping, and ruler settings.
    grid: GridSettings,
    /// The name of the top cell being viewed.
    top_cell: String,

    /// The spatial index over the flattened scene, rebuilt when the document or
    /// viewed cell changes.
    scene: SceneIndex,

    /// Whether the command palette window is open.
    palette_open: bool,
    /// The command-palette search query.
    palette_query: String,
    /// The query-bar text for "select by layer".
    layer_query: String,
    /// The most recent status-bar message.
    status: Status,
    /// The last world position under the cursor, for the status readout.
    cursor_world: Option<Point>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates the app with the built-in demo document loaded.
    ///
    /// This is cheap (it builds a small in-memory document and a spatial index) so
    /// it is safe to call from both the native launcher and the web mount point.
    #[must_use]
    pub fn new() -> Self {
        let doc = demo::demo_document();
        let layer_state = LayerState::from_technology(doc.technology());
        let scene = SceneIndex::build(&doc, demo::TOP_CELL);
        let document = SyncDocument::from_document("local", &doc);
        Self {
            renderer: WgpuRenderer::new(),
            document,
            history: History::new(doc),
            camera: ViewCamera::new(Point::ORIGIN, 0.05),
            fit_requested: true,
            tools: ToolState::new(),
            layer_state,
            selection: Selection::new(),
            grid: GridSettings::default(),
            top_cell: demo::TOP_CELL.to_owned(),
            scene,
            palette_open: false,
            palette_query: String::new(),
            layer_query: String::new(),
            status: Status::default(),
            cursor_world: None,
        }
    }

    /// The renderer (frozen Wave 0 contract accessor).
    #[must_use]
    pub fn renderer(&self) -> &WgpuRenderer {
        &self.renderer
    }

    /// The collaborative document (frozen Wave 0 contract accessor).
    #[must_use]
    pub fn document(&self) -> &SyncDocument {
        &self.document
    }

    /// Rebuilds the scene spatial index from the current document and top cell.
    ///
    /// Called after any edit so culling and hit-testing see the new geometry. The
    /// selection is cleared because shape indices are no longer valid.
    fn rebuild_scene(&mut self) {
        self.scene = SceneIndex::build(self.history.document(), &self.top_cell);
        self.selection.clear();
    }

    /// The technology database-units-per-micron for the current document.
    fn dbu_per_micron(&self) -> i64 {
        self.history.document().technology().dbu_per_micron
    }

    /// Runs a command-palette [`Command`], mutating the relevant app state.
    ///
    /// Centralizing execution here means the toolbar, keyboard shortcuts, and the
    /// palette all funnel through the same effects.
    fn run_command(&mut self, cmd: Command, screen: Option<ScreenRect>) {
        match cmd {
            Command::SetTool(tool) => {
                self.tools.set_active(tool);
                self.status.set(format!("Tool: {}", tool.label()));
            }
            Command::ToggleLayer(i) => {
                if let Some(row) = self.layer_state.rows().get(i) {
                    let id = row.id;
                    if let Some(now) = self.layer_state.toggle(id) {
                        self.status.set(format!(
                            "{} {}",
                            row_name(&self.layer_state, i),
                            on_off(now)
                        ));
                    }
                }
            }
            Command::Undo => {
                if self.history.undo() {
                    self.rebuild_scene();
                    self.status.set("Undo");
                } else {
                    self.status.set("Nothing to undo");
                }
            }
            Command::Redo => {
                if self.history.redo() {
                    self.rebuild_scene();
                    self.status.set("Redo");
                } else {
                    self.status.set("Nothing to redo");
                }
            }
            Command::ZoomToFit => {
                self.fit_requested = true;
                self.status.set("Zoom to fit");
            }
            Command::ToggleGrid => {
                self.grid.visible = !self.grid.visible;
                self.status
                    .set(format!("Grid {}", on_off(self.grid.visible)));
            }
            Command::ToggleSnap => {
                self.grid.snap_enabled = !self.grid.snap_enabled;
                self.status
                    .set(format!("Snap {}", on_off(self.grid.snap_enabled)));
            }
            Command::ClearSelection => {
                self.selection.clear();
                self.status.set("Selection cleared");
            }
            Command::SelectLayer(i) => {
                if let Some(row) = self.layer_state.rows().get(i) {
                    let id = row.id;
                    let hits = selection::shapes_on_layer(self.scene.shapes(), id);
                    let n = hits.len();
                    self.selection.set(hits);
                    self.status.set(format!("Selected {n} shape(s) on layer"));
                }
            }
            Command::ExportPng => self.export_png(screen),
        }
    }

    /// Exports the current view to a PNG next to the working directory (native).
    ///
    /// Uses the offscreen GPU renderer at the canvas resolution; if no GPU is
    /// available it sets a status message instead of failing. On the web this is a
    /// no-op (the palette does not offer it there).
    #[cfg(not(target_arch = "wasm32"))]
    fn export_png(&mut self, screen: Option<ScreenRect>) {
        let (w, h) = screen.map_or((1024, 768), |s| {
            (s.width.max(16.0) as u32, s.height.max(16.0) as u32)
        });
        let camera = screen.map_or_else(
            || reticle_model::Camera {
                center: self.camera.center(),
                pixels_per_dbu: self.camera.pixels_per_dbu() as f32,
                viewport: Rect::default(),
            },
            |s| self.camera.to_model_camera(&s),
        );
        match reticle_render::WgpuContext::new_blocking() {
            Some(ctx) => {
                let rgba = self.renderer.render_document_offscreen(
                    &ctx,
                    self.history.document(),
                    &self.top_cell,
                    &camera,
                    (w, h),
                );
                match write_png("reticle-export.png", w, h, &rgba) {
                    Ok(path) => self.status.set(format!("Exported {path}")),
                    Err(e) => self.status.set(format!("Export failed: {e}")),
                }
            }
            None => self.status.set("No GPU available; PNG export skipped"),
        }
    }

    /// PNG export is unavailable on the web; this stub keeps the call site uniform.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unused_self)]
    fn export_png(&mut self, _screen: Option<ScreenRect>) {
        self.status.set("PNG export is native-only");
    }

    /// Handles global keyboard shortcuts (palette, tools, undo/redo, fit).
    ///
    /// Shortcuts are ignored while a text field has focus so typing in the palette
    /// or query bar does not trigger them.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Suppress shortcuts while a text field owns keyboard focus so typing in the
        // palette or query bar does not trigger tool changes.
        if ctx.memory(|m| m.focused().is_some()) {
            // Still allow Escape to close the palette even while its field has focus.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.palette_open = false;
            }
            return;
        }
        let (ctrl, key_p, key_z, key_y, key_f, key_g, esc, key_s, key_m, key_v) = ctx.input(|i| {
            let m = i.modifiers.command || i.modifiers.ctrl;
            (
                m,
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::Y),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::S),
                i.key_pressed(egui::Key::M),
                i.key_pressed(egui::Key::V),
            )
        });
        if ctrl && key_p {
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
        }
        if esc {
            self.palette_open = false;
        }
        if ctrl && key_z {
            self.run_command(Command::Undo, None);
        }
        if ctrl && key_y {
            self.run_command(Command::Redo, None);
        }
        if ctrl && key_g {
            self.run_command(Command::ToggleGrid, None);
        }
        if key_f && !ctrl {
            self.run_command(Command::ZoomToFit, None);
        }
        // Single-key tool shortcuts (no modifier).
        if !ctrl {
            if key_v {
                self.run_command(Command::SetTool(Tool::Select), None);
            }
            if key_s {
                self.run_command(Command::SetTool(Tool::Pan), None);
            }
            if key_m {
                self.run_command(Command::SetTool(Tool::Measure), None);
            }
        }
    }

    /// Draws the top toolbar: tool buttons, view actions, and the palette hint.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Tool:");
            for tool in Tool::all() {
                let selected = self.tools.active() == tool;
                if ui.selectable_label(selected, tool.label()).clicked() {
                    self.tools.set_active(tool);
                }
            }
            ui.separator();
            if ui.button("Fit").clicked() {
                self.fit_requested = true;
            }
            let can_undo = self.history.can_undo();
            let can_redo = self.history.can_redo();
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo"))
                .clicked()
            {
                self.run_command(Command::Undo, None);
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("Redo"))
                .clicked()
            {
                self.run_command(Command::Redo, None);
            }
            ui.separator();
            ui.checkbox(&mut self.grid.visible, "Grid");
            ui.checkbox(&mut self.grid.snap_enabled, "Snap");
            ui.separator();
            if ui.button("Palette (Ctrl+P)").clicked() {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
            }
        });
    }

    /// Draws the left layer-manager panel: filter, per-layer swatch + visibility.
    fn layer_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layers");
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(self.layer_state.filter_mut());
        });
        ui.horizontal(|ui| {
            if ui.small_button("Show all").clicked() {
                self.layer_state.show_all();
            }
            if ui.small_button("Hide all").clicked() {
                self.layer_state.hide_all();
            }
        });
        ui.separator();

        let indices = self.layer_state.filtered_indices();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let rows = self.layer_state.rows_mut();
                for i in indices {
                    let row = &mut rows[i];
                    ui.horizontal(|ui| {
                        let (r, g, b, _) = layers::rgba_components(row.color_rgba);
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                        ui.checkbox(&mut row.visible, &row.name);
                    });
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Select layer:");
            ui.text_edit_singleline(&mut self.layer_query);
        });
        if ui.button("Select by layer name").clicked() {
            self.select_by_layer_name();
        }
    }

    /// Selects every shape whose layer name matches the query bar (case-insensitive
    /// substring); an empty query selects nothing.
    fn select_by_layer_name(&mut self) {
        let q = self.layer_query.trim().to_lowercase();
        if q.is_empty() {
            self.status.set("Enter a layer name to select");
            return;
        }
        let ids: Vec<LayerId> = self
            .layer_state
            .rows()
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&q))
            .map(|r| r.id)
            .collect();
        let mut hits = Vec::new();
        for id in ids {
            hits.extend(selection::shapes_on_layer(self.scene.shapes(), id));
        }
        let n = hits.len();
        self.selection.set(hits);
        self.status.set(format!("Selected {n} shape(s)"));
    }

    /// Draws the right-hand undo-history panel: stack depths and step buttons.
    fn history_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("History");
        ui.label(format!("Undo stack: {}", self.history.undo_depth()));
        ui.label(format!("Redo stack: {}", self.history.redo_depth()));
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("Step back"))
                .clicked()
            {
                self.run_command(Command::Undo, None);
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("Step fwd"))
                .clicked()
            {
                self.run_command(Command::Redo, None);
            }
        });
        ui.separator();
        ui.label(format!("Selected shapes: {}", self.selection.len()));
        ui.label(format!("Scene shapes: {}", self.scene.len()));
        if ui.button("Add demo rectangle").clicked() {
            self.add_demo_rectangle();
        }
    }

    /// Appends a rectangle to the top cell through the undo history, then rebuilds
    /// the scene — a concrete edit so undo/redo and the history panel are exercised
    /// from the UI.
    fn add_demo_rectangle(&mut self) {
        // Place it near the current view center so it is visible.
        let c = self.camera.center();
        let rect = Rect::new(
            c,
            Point::new(c.x.saturating_add(800), c.y.saturating_add(800)),
        );
        let layer = self
            .layer_state
            .rows()
            .first()
            .map_or(LayerId::new(1, 0), |r| r.id);
        let shape = DrawShape::new(layer, ShapeKind::Rect(rect));
        match self.history.apply(reticle_model::Edit::AddShape {
            cell: self.top_cell.clone(),
            shape,
        }) {
            Ok(()) => {
                self.rebuild_scene();
                self.status.set("Added rectangle");
            }
            Err(e) => self.status.set(format!("Edit failed: {e}")),
        }
    }

    /// Draws the bottom status bar: tool, cursor coordinates, zoom, and messages.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("Tool: {}", self.tools.active().label()));
            ui.separator();
            if let Some(p) = self.cursor_world {
                ui.label(format!("Cursor: ({}, {}) DBU", p.x, p.y));
            } else {
                ui.label("Cursor: --");
            }
            ui.separator();
            ui.label(format!("Zoom: {:.4} px/DBU", self.camera.pixels_per_dbu()));
            if let Some(m) = self.tools.measurement() {
                ui.separator();
                ui.label(format!(
                    "Measure: {:.1} DBU = {:.3} um  (dx {}, dy {})",
                    m.distance_dbu(),
                    m.distance_microns(),
                    m.dx(),
                    m.dy()
                ));
            }
            if !self.status.text.is_empty() {
                ui.separator();
                ui.label(&self.status.text);
            }
        });
    }

    /// Draws the command-palette window and runs the chosen command.
    fn palette_window(&mut self, ctx: &egui::Context, screen: Option<ScreenRect>) {
        if !self.palette_open {
            return;
        }
        let layer_names: Vec<String> = self
            .layer_state
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        let entries = command::catalog(&layer_names);

        let mut open = self.palette_open;
        let mut chosen: Option<Command> = None;
        egui::Window::new("Command palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(Pos2::new(200.0, 120.0))
            .show(ctx, |ui| {
                ui.label("Type to filter, click to run:");
                let resp = ui.text_edit_singleline(&mut self.palette_query);
                resp.request_focus();
                ui.separator();
                let matches = command::filter(&entries, &self.palette_query);
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in matches {
                            if ui.selectable_label(false, &entry.label).clicked() {
                                chosen = Some(entry.command);
                            }
                        }
                    });
            });
        self.palette_open = open;
        if let Some(cmd) = chosen {
            self.run_command(cmd, screen);
            self.palette_open = false;
        }
    }

    /// Draws the layout canvas and processes pointer interaction on it.
    ///
    /// Returns the canvas [`ScreenRect`] so the caller can hand it to actions (PNG
    /// export, deferred zoom-to-fit) that need the real pixel size.
    fn canvas(&mut self, ui: &mut egui::Ui) -> ScreenRect {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        let screen = ScreenRect::new(rect.min.x, rect.min.y, rect.width(), rect.height());

        // Background.
        painter.rect_filled(rect, 0.0, Color32::from_rgb(16, 18, 22));

        // Deferred fit now that we know the canvas size.
        if self.fit_requested {
            if let Some(bounds) = self.scene.bounds() {
                self.camera.zoom_to_fit(&screen, bounds);
            }
            self.fit_requested = false;
        }

        self.process_canvas_input(ui.ctx(), &response, &screen);

        // Draw grid + rulers under the geometry.
        if self.grid.visible {
            self.draw_grid(&painter, &screen);
        }

        // Draw the scene (shapes or, at low zoom, cell boxes).
        let viewport = self.camera.visible_world_rect(&screen);
        match culling::lod_for_zoom(self.camera.pixels_per_dbu()) {
            DetailLevel::Shapes => self.draw_shapes(&painter, &screen, viewport),
            DetailLevel::CellBoxes => self.draw_cell_boxes(&painter, &screen, viewport),
        }

        self.draw_rulers(&painter, &screen);
        self.draw_measure(&painter, &screen);
        self.draw_presence(&painter, &screen);

        screen
    }

    /// Routes pointer input on the canvas through the active tool.
    fn process_canvas_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        // Track the cursor world position (snapped) for the status bar.
        if let Some(pos) = response.hover_pos() {
            let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
            self.cursor_world = Some(self.grid.snap(raw));
        } else {
            self.cursor_world = None;
        }

        // Zoom to cursor on scroll, regardless of tool.
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0
                && let Some(pos) = response.hover_pos()
            {
                let factor = (f64::from(scroll) * 0.0015).exp();
                self.camera.zoom_about(screen, factor, pos.x, pos.y);
            }
        }

        match self.tools.active() {
            Tool::Pan => {
                if response.dragged() {
                    let d = response.drag_delta();
                    self.camera.pan_pixels(d.x, d.y);
                }
            }
            Tool::Measure => {
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
                    let world = self.grid.snap(raw);
                    let dpm = self.dbu_per_micron();
                    if let Some(m) = self.tools.measure_click(world, dpm) {
                        self.status.set(format!(
                            "Distance {:.1} DBU ({:.3} um)",
                            m.distance_dbu(),
                            m.distance_microns()
                        ));
                    }
                }
            }
            Tool::Select => self.handle_select_input(ctx, response, screen),
        }
    }

    /// Select-tool input: click to pick the topmost shape, drag to rubber-band.
    fn handle_select_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        let additive = ctx.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);

        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            match self.scene.pick(world) {
                Some(idx) => {
                    if additive {
                        self.selection.toggle(idx);
                    } else {
                        self.selection.select_one(idx);
                    }
                }
                None => {
                    if !additive {
                        self.selection.clear();
                    }
                }
            }
        }

        // Rubber-band: on drag release, select shapes fully inside the drag box.
        if response.drag_stopped()
            && let (Some(origin), Some(current)) =
                (Self::drag_origin(response), response.hover_pos())
        {
            let a = self.camera.screen_to_world(screen, origin.x, origin.y);
            let b = self.camera.screen_to_world(screen, current.x, current.y);
            let band = Rect::new(a, b);
            if band.width() > 0 && band.height() > 0 {
                let hits = selection::shapes_in_rect(self.scene.shapes(), band);
                if additive {
                    self.selection.extend(hits);
                } else {
                    self.selection.set(hits);
                }
            }
        }
    }

    /// The screen position where the current drag started, if any.
    ///
    /// egui exposes the press origin via `interact_pointer_pos` during a drag; when
    /// the drag has just stopped we reconstruct the origin from the current pointer
    /// and the accumulated drag delta.
    fn drag_origin(response: &egui::Response) -> Option<Pos2> {
        let current = response.hover_pos()?;
        let delta = response.drag_delta();
        Some(Pos2::new(current.x - delta.x, current.y - delta.y))
    }

    /// Draws the background grid lines within the canvas.
    fn draw_grid(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let ppd = self.camera.pixels_per_dbu();
        let step = self.grid.display_step_dbu(ppd);
        let world = self.camera.visible_world_rect(screen);
        let color = Color32::from_rgb(34, 38, 46);
        let stroke = Stroke::new(1.0, color);

        for x in grid::grid_lines(world.min.x, world.max.x, step) {
            let (sx, _) = self
                .camera
                .world_to_screen(screen, Point::new(x, world.min.y));
            painter.line_segment(
                [
                    Pos2::new(sx, screen.top),
                    Pos2::new(sx, screen.top + screen.height),
                ],
                stroke,
            );
        }
        for y in grid::grid_lines(world.min.y, world.max.y, step) {
            let (_, sy) = self
                .camera
                .world_to_screen(screen, Point::new(world.min.x, y));
            painter.line_segment(
                [
                    Pos2::new(screen.left, sy),
                    Pos2::new(screen.left + screen.width, sy),
                ],
                stroke,
            );
        }

        // Emphasize the world axes.
        let axis = Stroke::new(1.5, Color32::from_rgb(60, 66, 78));
        let (ox, oy) = self.camera.world_to_screen(screen, Point::ORIGIN);
        painter.line_segment(
            [
                Pos2::new(ox, screen.top),
                Pos2::new(ox, screen.top + screen.height),
            ],
            axis,
        );
        painter.line_segment(
            [
                Pos2::new(screen.left, oy),
                Pos2::new(screen.left + screen.width, oy),
            ],
            axis,
        );
    }

    /// Draws the visible, non-hidden shapes with per-layer colors, highlighting the
    /// current selection.
    fn draw_shapes(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let shapes = self.scene.shapes();
        for idx in self.scene.query(viewport) {
            let shape = &shapes[idx];
            if !self.layer_state.is_visible(shape.layer) {
                continue;
            }
            let (r, g, b, a) = self.layer_color(shape.layer);
            let fill = Color32::from_rgba_unmultiplied(r, g, b, a);
            let selected = self.selection.contains(idx);
            self.draw_one_shape(painter, screen, shape, fill, selected);
        }
    }

    /// Draws a single [`DrawShape`] in the given fill color.
    fn draw_one_shape(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        shape: &DrawShape,
        fill: Color32,
        selected: bool,
    ) {
        let outline = if selected {
            Stroke::new(2.0, Color32::from_rgb(255, 240, 120))
        } else {
            Stroke::new(1.0, fill.gamma_multiply(1.4))
        };
        match &shape.kind {
            ShapeKind::Rect(rect) => {
                let e = self.world_rect_to_screen(screen, *rect);
                painter.rect_filled(e, 0.0, fill);
                painter.rect_stroke(e, 0.0, outline, StrokeKind::Middle);
            }
            ShapeKind::Polygon(poly) => {
                let pts: Vec<Pos2> = poly
                    .vertices()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 3 {
                    painter.add(Shape::convex_polygon(pts, fill, outline));
                }
            }
            ShapeKind::Path(path) => {
                let pts: Vec<Pos2> = path
                    .points()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 2 {
                    // Width in screen pixels, at least 1px so thin wires stay visible.
                    let w =
                        (f64::from(path.width()) * self.camera.pixels_per_dbu()).max(1.0) as f32;
                    let stroke = if selected {
                        Stroke::new(w.max(2.0), Color32::from_rgb(255, 240, 120))
                    } else {
                        Stroke::new(w, fill)
                    };
                    painter.add(Shape::line(pts, stroke));
                }
            }
        }
    }

    /// Draws cell bounding boxes for the low-zoom level of detail.
    fn draw_cell_boxes(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let stroke = Stroke::new(1.0, Color32::from_rgb(120, 140, 180));
        let fill = Color32::from_rgba_unmultiplied(60, 80, 120, 40);
        for cb in culling::visible_cell_boxes(self.history.document(), &self.top_cell, viewport) {
            let e = self.world_rect_to_screen(screen, cb.bbox);
            painter.rect_filled(e, 0.0, fill);
            painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
        }
    }

    /// Draws top/left rulers with tick marks and DBU labels.
    fn draw_rulers(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let bar = 18.0;
        let bg = Color32::from_rgb(24, 27, 33);
        let top_bar = EguiRect::from_min_size(
            Pos2::new(screen.left, screen.top),
            Vec2::new(screen.width, bar),
        );
        let left_bar = EguiRect::from_min_size(
            Pos2::new(screen.left, screen.top),
            Vec2::new(bar, screen.height),
        );
        painter.rect_filled(top_bar, 0.0, bg);
        painter.rect_filled(left_bar, 0.0, bg);

        let ppd = self.camera.pixels_per_dbu();
        let step = self.grid.display_step_dbu(ppd);
        let world = self.camera.visible_world_rect(screen);
        let tick = Stroke::new(1.0, Color32::from_rgb(90, 96, 110));
        let font = FontId::monospace(9.0);
        let label = Color32::from_rgb(170, 176, 190);

        for x in grid::grid_lines(world.min.x, world.max.x, step) {
            let (sx, _) = self.camera.world_to_screen(screen, Point::new(x, 0));
            if sx < screen.left + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(sx, screen.top), Pos2::new(sx, screen.top + bar)],
                tick,
            );
            painter.text(
                Pos2::new(sx + 2.0, screen.top + 1.0),
                Align2::LEFT_TOP,
                x.to_string(),
                font.clone(),
                label,
            );
        }
        for y in grid::grid_lines(world.min.y, world.max.y, step) {
            let (_, sy) = self.camera.world_to_screen(screen, Point::new(0, y));
            if sy < screen.top + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(screen.left, sy), Pos2::new(screen.left + bar, sy)],
                tick,
            );
            painter.text(
                Pos2::new(screen.left + 1.0, sy + 1.0),
                Align2::LEFT_TOP,
                y.to_string(),
                font.clone(),
                label,
            );
        }
    }

    /// Draws the in-progress or completed measurement overlay.
    fn draw_measure(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let color = Color32::from_rgb(255, 210, 90);
        let stroke = Stroke::new(1.5, color);
        if let Some(m) = self.tools.measurement() {
            let a = self.world_pos_to_screen(screen, m.start);
            let b = self.world_pos_to_screen(screen, m.end);
            painter.line_segment([a, b], stroke);
            painter.circle_filled(a, 3.0, color);
            painter.circle_filled(b, 3.0, color);
            let mid = Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            painter.text(
                mid,
                Align2::CENTER_CENTER,
                format!(
                    "{:.0} DBU / {:.2} um",
                    m.distance_dbu(),
                    m.distance_microns()
                ),
                FontId::monospace(11.0),
                color,
            );
        } else if let Some(start) = self.tools.measure_start() {
            // First point placed, awaiting the second.
            let a = self.world_pos_to_screen(screen, start);
            painter.circle_filled(a, 3.0, color);
        }
    }

    /// Draws remote collaborators' cursors from the sync presence map (stretch:
    /// there are no live peers in this build, so this is normally empty).
    fn draw_presence(&self, painter: &egui::Painter, screen: &ScreenRect) {
        for (_, presence) in self.document.awareness().iter() {
            let (r, g, b, _) = layers::rgba_components(presence.color_rgba);
            let color = Color32::from_rgb(r, g, b);
            let p = self.world_pos_to_screen(screen, presence.cursor);
            painter.circle_filled(p, 4.0, color);
            if !presence.display_name.is_empty() {
                painter.text(
                    Pos2::new(p.x + 6.0, p.y),
                    Align2::LEFT_CENTER,
                    &presence.display_name,
                    FontId::proportional(11.0),
                    color,
                );
            }
        }
    }

    /// The `(r, g, b, a)` color for a layer, or a neutral gray if unknown.
    fn layer_color(&self, layer: LayerId) -> (u8, u8, u8, u8) {
        self.layer_state
            .rows()
            .iter()
            .find(|r| r.id == layer)
            .map_or((160, 160, 160, 190), |r| {
                let (rr, gg, bb, _) = layers::rgba_components(r.color_rgba);
                // Semi-transparent fill so overlapping layers read clearly.
                (rr, gg, bb, 170)
            })
    }

    /// Converts a world point to an egui screen position.
    fn world_pos_to_screen(&self, screen: &ScreenRect, p: Point) -> Pos2 {
        let (x, y) = self.camera.world_to_screen(screen, p);
        Pos2::new(x, y)
    }

    /// Converts a world rectangle to a (normalized) egui screen rectangle.
    fn world_rect_to_screen(&self, screen: &ScreenRect, r: Rect) -> EguiRect {
        let a = self.world_pos_to_screen(screen, r.min);
        let b = self.world_pos_to_screen(screen, r.max);
        EguiRect::from_two_pos(a, b)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);

        // Cache the canvas rect across panels so the palette/export can use it.
        let mut canvas_screen: Option<ScreenRect> = None;

        egui::Panel::top("toolbar").show(ui, |ui| {
            self.toolbar(ui);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            self.status_bar(ui);
        });
        egui::Panel::left("layers")
            .resizable(true)
            .default_size(210.0)
            .show(ui, |ui| {
                self.layer_panel(ui);
            });
        egui::Panel::right("history")
            .resizable(true)
            .default_size(190.0)
            .show(ui, |ui| {
                self.history_panel(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui));
        });

        self.palette_window(&ctx, canvas_screen);

        // Keep animating while dragging/measuring so interaction feels live.
        ctx.request_repaint();
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // View/UI state is persisted to our own session file on native; egui's
        // storage is not used directly (no serde dependency in this crate).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let hidden: Vec<LayerId> = self
                .layer_state
                .rows()
                .iter()
                .filter(|r| !r.visible)
                .map(|r| r.id)
                .collect();
            let state = crate::session::SessionState::capture(
                &self.camera,
                self.tools.active(),
                self.grid,
                &hidden,
            );
            let _ = crate::session::save(&state);
        }
    }
}

/// Formats a boolean as `on`/`off` for status messages.
fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// The display name of the layer row at `index`, or an empty string.
fn row_name(state: &LayerState, index: usize) -> String {
    state
        .rows()
        .get(index)
        .map(|r| r.name.clone())
        .unwrap_or_default()
}

/// Writes RGBA8 pixels to a PNG file, returning the absolute path written.
///
/// Uses a minimal hand-rolled PNG encoder (single IDAT, zlib stored blocks) so the
/// crate needs no image dependency. Native only.
#[cfg(not(target_arch = "wasm32"))]
fn write_png(name: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<String> {
    let path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(name);
    let bytes = crate::app::png::encode_rgba(width, height, rgba);
    std::fs::write(&path, bytes)?;
    Ok(path.display().to_string())
}

/// A dependency-free PNG encoder for the export action.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod png {
    /// Encodes tightly-packed RGBA8 pixels (row 0 at the top) into PNG file bytes.
    ///
    /// The image data is stored uncompressed inside a single zlib stream using
    /// "stored" (type 0) deflate blocks, so no compression library is required. The
    /// result is a valid, if unoptimized, PNG.
    #[must_use]
    pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

        // IHDR.
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(6); // color type RGBA
        ihdr.push(0); // compression
        ihdr.push(0); // filter
        ihdr.push(0); // interlace
        write_chunk(&mut out, *b"IHDR", &ihdr);

        // Raw image with a 0 filter byte per row.
        let stride = (width as usize) * 4;
        let mut raw = Vec::with_capacity((stride + 1) * height as usize);
        for y in 0..height as usize {
            raw.push(0);
            let start = y * stride;
            let end = start + stride;
            if end <= rgba.len() {
                raw.extend_from_slice(&rgba[start..end]);
            } else {
                raw.extend(std::iter::repeat_n(0u8, stride));
            }
        }

        let idat = zlib_store(&raw);
        write_chunk(&mut out, *b"IDAT", &idat);
        write_chunk(&mut out, *b"IEND", &[]);
        out
    }

    /// Wraps `data` in a PNG chunk (length, type, data, CRC).
    fn write_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        let mut crc = Crc::new();
        crc.update(&kind);
        crc.update(data);
        out.extend_from_slice(&crc.finish().to_be_bytes());
    }

    /// Wraps `data` in a zlib stream using uncompressed (stored) deflate blocks.
    fn zlib_store(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0x78); // CMF: deflate, 32K window
        out.push(0x01); // FLG: no dict, fastest
        // Stored blocks of at most 65535 bytes.
        let mut i = 0;
        while i < data.len() {
            let chunk = (data.len() - i).min(0xFFFF);
            let last = i + chunk >= data.len();
            out.push(u8::from(last)); // BFINAL, BTYPE=00
            let len = chunk as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&data[i..i + chunk]);
            i += chunk;
        }
        // Adler-32 of the uncompressed data.
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }

    /// Computes an Adler-32 checksum.
    fn adler32(data: &[u8]) -> u32 {
        let mut a = 1u32;
        let mut b = 0u32;
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    /// A minimal CRC-32 (as used by PNG) accumulator.
    struct Crc {
        value: u32,
    }

    impl Crc {
        /// Starts a new CRC accumulator.
        fn new() -> Self {
            Self { value: 0xFFFF_FFFF }
        }

        /// Feeds bytes into the CRC.
        fn update(&mut self, data: &[u8]) {
            for &byte in data {
                let mut c = (self.value ^ u32::from(byte)) & 0xFF;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                self.value = c ^ (self.value >> 8);
            }
        }

        /// Finalizes and returns the CRC value.
        fn finish(self) -> u32 {
            self.value ^ 0xFFFF_FFFF
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn png_has_signature_and_chunks() {
            let px = [255u8, 0, 0, 255, 0, 255, 0, 255];
            let png = encode_rgba(2, 1, &px);
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            // Contains IHDR, IDAT, IEND markers.
            let has = |needle: &[u8; 4]| png.windows(4).any(|w| w == needle);
            assert!(has(b"IHDR"));
            assert!(has(b"IDAT"));
            assert!(has(b"IEND"));
        }

        #[test]
        fn adler_and_crc_known_values() {
            // Adler-32 of "abc" is 0x024D0127.
            assert_eq!(adler32(b"abc"), 0x024D_0127);
            let mut crc = Crc::new();
            crc.update(b"abc");
            // CRC-32 of "abc" is 0x352441C2.
            assert_eq!(crc.finish(), 0x3524_41C2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_new_loads_demo_scene() {
        let app = App::new();
        assert!(!app.scene.is_empty());
        assert_eq!(app.top_cell, demo::TOP_CELL);
        assert!(app.history.document().cell(demo::TOP_CELL).is_some());
    }

    #[test]
    fn toggling_layer_via_command_hides_it() {
        let mut app = App::new();
        let id = app.layer_state.rows()[0].id;
        assert!(app.layer_state.is_visible(id));
        app.run_command(Command::ToggleLayer(0), None);
        assert!(!app.layer_state.is_visible(id));
    }

    #[test]
    fn hidden_layer_is_excluded_from_draw_list() {
        // Build the set of visible shape indices for a viewport the way `draw_shapes`
        // does, and confirm hiding a layer removes its shapes.
        let mut app = App::new();
        let bounds = app.scene.bounds().unwrap();
        let visible_layer = app.scene.shapes()[app.scene.query(bounds)[0]].layer;

        let count_visible = |app: &App| -> usize {
            app.scene
                .query(bounds)
                .into_iter()
                .filter(|&i| app.layer_state.is_visible(app.scene.shapes()[i].layer))
                .count()
        };
        let before = count_visible(&app);
        app.layer_state.set_visible(visible_layer, false);
        let after = count_visible(&app);
        assert!(after < before, "hiding a layer should shrink the draw list");
    }

    #[test]
    fn undo_redo_command_restores_scene() {
        let mut app = App::new();
        let before = app.scene.len();
        app.add_demo_rectangle();
        assert_eq!(app.scene.len(), before + 1);
        app.run_command(Command::Undo, None);
        assert_eq!(app.scene.len(), before);
        app.run_command(Command::Redo, None);
        assert_eq!(app.scene.len(), before + 1);
    }

    #[test]
    fn set_tool_command_switches_tool() {
        let mut app = App::new();
        app.run_command(Command::SetTool(Tool::Measure), None);
        assert_eq!(app.tools.active(), Tool::Measure);
    }

    #[test]
    fn select_layer_command_populates_selection() {
        let mut app = App::new();
        // Pick a layer index that actually has shapes.
        let target_layer = app.scene.shapes()[0].layer;
        let idx = app
            .layer_state
            .rows()
            .iter()
            .position(|r| r.id == target_layer)
            .unwrap();
        app.run_command(Command::SelectLayer(idx), None);
        assert!(!app.selection.is_empty());
        for i in app.selection.iter() {
            assert_eq!(app.scene.shapes()[i].layer, target_layer);
        }
    }

    #[test]
    fn toggle_grid_and_snap_commands() {
        let mut app = App::new();
        let g0 = app.grid.visible;
        app.run_command(Command::ToggleGrid, None);
        assert_ne!(app.grid.visible, g0);
        let s0 = app.grid.snap_enabled;
        app.run_command(Command::ToggleSnap, None);
        assert_ne!(app.grid.snap_enabled, s0);
    }

    #[test]
    fn clear_selection_command_empties_it() {
        let mut app = App::new();
        app.selection.set([0, 1, 2]);
        app.run_command(Command::ClearSelection, None);
        assert!(app.selection.is_empty());
    }
}
