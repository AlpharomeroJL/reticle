//! Bridging the agent's edits onto the collaboration layer.
//!
//! The propose-verify-correct harness ([`crate::run`]) edits a private
//! [`Session`](reticle_agent_api::Session). To let a human watch and edit alongside
//! the agent in real time, [`AgentCollaborator`] mirrors those edits onto a
//! [`SyncDocument`] (a `yrs` CRDT) under the [`AGENT_ACTOR`] identity.
//!
//! # What it guarantees
//!
//! * **Step-atomic transactions.** One logical agent step (one propose iteration,
//!   which may emit many commands) is applied as a **single**
//!   [`SyncDocument::step`] transaction. The whole step lands as one CRDT update, so
//!   a concurrent peer never observes a half-drawn step (one shape of a multi-shape
//!   placement on the wire, the rest not yet).
//! * **Presence.** After each step the agent publishes its cursor (the location of
//!   the last shape it placed) and selection (the ids placed this step) over the
//!   awareness layer, under [`AGENT_ACTOR`], so a watcher can render them.
//! * **A status channel.** The agent serializes an [`AgentStatus`] into the
//!   awareness status slot (again under [`AGENT_ACTOR`]) so a watcher can narrate the
//!   loop.
//! * **Pacing.** A [`Pacing`] setting inserts a delay between steps for a live demo,
//!   or runs instantly for tests and replay.
//!
//! # Which commands become CRDT edits
//!
//! Only the geometry-creating commands change what a human sees on the canvas, so
//! only those are mirrored: [`CreateCell`](AgentCommand::CreateCell),
//! [`AddRect`](AgentCommand::AddRect), [`AddPolygon`](AgentCommand::AddPolygon),
//! [`AddPath`](AgentCommand::AddPath),
//! [`PlaceInstance`](AgentCommand::PlaceInstance),
//! [`PlaceArray`](AgentCommand::PlaceArray), and
//! [`DeleteCell`](AgentCommand::DeleteCell). Read-only commands (DRC, queries,
//! export, render) and technology/session commands produce no drawing and are not
//! mirrored. Id-addressed edits (`TransformShapes`, `DeleteShapes`) address the
//! command surface's [`ElementId`](reticle_agent_api::ElementId)s, which do not map
//! to CRDT element ids; they are left to a future extension rather than applied
//! incorrectly.

use std::time::Duration;

use reticle_agent_api::args::{
    EndcapArg, LayerArg, OrientationArg, PointArg, RectArg, TransformArg,
};
use reticle_agent_api::{AGENT_ACTOR, AgentCommand, AgentStatus};
use reticle_geometry::{
    Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{ArrayInstance, DrawShape, Instance, ShapeKind};
use reticle_sync::{Presence, StepEdit, SyncDocument};

/// How the collaborator paces its steps on the wire.
///
/// Pacing is pure data the caller drives; the bridge sleeps only when told to, so
/// tests and replay stay instant and deterministic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pacing {
    /// Apply each step immediately, with no delay. The default; used by tests and
    /// replay.
    #[default]
    Instant,
    /// Sleep for the given duration *before* applying each step, to make a live demo
    /// legible to a human watcher.
    Delay(Duration),
}

impl Pacing {
    /// A fixed inter-step delay in milliseconds.
    #[must_use]
    pub fn millis(ms: u64) -> Self {
        Pacing::Delay(Duration::from_millis(ms))
    }

    /// Sleeps for this pacing's delay, if any. A no-op in [`Pacing::Instant`] mode.
    fn wait(self) {
        if let Pacing::Delay(d) = self {
            std::thread::sleep(d);
        }
    }
}

/// What one applied step changed on the CRDT: the ids placed and where the cursor
/// should sit.
///
/// Returned by [`AgentCollaborator::apply_step`] so a caller can react (for example,
/// forward the resulting CRDT update to peers). The `cursor` is the location of the
/// last shape placed this step, or unchanged from the previous step if the step
/// placed no shape (for example, a step that only created an empty cell).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StepReport {
    /// The CRDT element ids created this step, in placement order. These become the
    /// agent's selection.
    pub placed: Vec<String>,
    /// The cursor location published for this step (document DBU coordinates).
    pub cursor: Point,
}

/// Mirrors an agent's edits onto a [`SyncDocument`] under [`AGENT_ACTOR`].
///
/// Construct one, then feed it the command batch from each propose iteration via
/// [`apply_step`](Self::apply_step) (and, optionally,
/// [`publish_status`](Self::publish_status) to narrate). The underlying
/// [`SyncDocument`] is available through [`document`](Self::document) /
/// [`sync`](Self::sync) so the resulting CRDT updates can be exchanged with peers
/// exactly as any other `reticle-sync` peer's are.
#[derive(Debug)]
pub struct AgentCollaborator {
    sync: SyncDocument,
    pacing: Pacing,
    /// The most recent cursor location, carried across steps so a no-placement step
    /// does not reset it to the origin.
    cursor: Point,
    /// Packed `0xRRGGBBAA` color the agent's cursor and selection render in.
    color_rgba: u32,
    /// A human-readable display name for the agent's presence.
    display_name: String,
}

impl Default for AgentCollaborator {
    fn default() -> Self {
        Self::new(Pacing::Instant)
    }
}

impl AgentCollaborator {
    /// A distinctive default color for the agent's cursor and selection (a warm
    /// amber, fully opaque), so a watcher can tell agent presence from a human's.
    pub const DEFAULT_COLOR_RGBA: u32 = 0xFF_A5_00_FF;

    /// Creates a collaborator whose `yrs` peer identity is [`AGENT_ACTOR`], pacing
    /// its steps as given.
    #[must_use]
    pub fn new(pacing: Pacing) -> Self {
        Self {
            sync: SyncDocument::new(AGENT_ACTOR),
            pacing,
            cursor: Point::ORIGIN,
            color_rgba: Self::DEFAULT_COLOR_RGBA,
            display_name: "Reticle agent".to_owned(),
        }
    }

    /// Sets the color the agent's cursor and selection render in (packed
    /// `0xRRGGBBAA`), returning `self` for chaining.
    #[must_use]
    pub fn with_color(mut self, color_rgba: u32) -> Self {
        self.color_rgba = color_rgba;
        self
    }

    /// Sets the display name shown for the agent's presence, returning `self` for
    /// chaining.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    /// The underlying collaboration document (a materialized model view of the CRDT).
    #[must_use]
    pub fn document(&self) -> &reticle_model::Document {
        self.sync.document()
    }

    /// The underlying [`SyncDocument`], for exchanging updates with peers.
    #[must_use]
    pub fn sync(&self) -> &SyncDocument {
        &self.sync
    }

    /// Mutable access to the underlying [`SyncDocument`], for applying peers'
    /// updates.
    pub fn sync_mut(&mut self) -> &mut SyncDocument {
        &mut self.sync
    }

    /// The agent's actor identity on the collaboration layer ([`AGENT_ACTOR`]).
    #[must_use]
    pub fn actor(&self) -> &str {
        self.sync.actor()
    }

    /// Applies one logical agent step (the batch of commands from one propose
    /// iteration) as a single atomic CRDT transaction, then publishes the agent's
    /// cursor and selection over the awareness layer.
    ///
    /// Every geometry-creating command in `commands` is translated into one write on
    /// a shared [`StepEdit`], so the whole step commits as one `yrs` update: a peer
    /// never sees a partially-applied step. In [`Pacing::Delay`] mode this sleeps for
    /// the configured delay *before* applying, to pace a live demo; in
    /// [`Pacing::Instant`] mode it applies immediately.
    ///
    /// Returns a [`StepReport`] naming the ids placed (the agent's new selection) and
    /// the cursor location published for the step.
    pub fn apply_step<'a>(
        &mut self,
        commands: impl IntoIterator<Item = &'a AgentCommand>,
    ) -> StepReport {
        self.pacing.wait();

        let mut cursor = self.cursor;
        // Apply the whole batch inside one transaction; `placed` collects the ids of
        // shapes/placements created, in order, and `cursor` tracks the last location.
        let placed = self.sync.step(|edit| {
            let mut placed = Vec::new();
            for command in commands {
                mirror_command(edit, command, &mut placed, &mut cursor);
            }
            placed
        });

        self.cursor = cursor;
        self.publish_presence(&placed, cursor);
        StepReport { placed, cursor }
    }

    /// Publishes the agent's presence (cursor at `cursor`, selection = `selection`)
    /// over the awareness layer under [`AGENT_ACTOR`].
    fn publish_presence(&mut self, selection: &[String], cursor: Point) {
        let presence = Presence {
            actor: AGENT_ACTOR.to_owned(),
            display_name: self.display_name.clone(),
            color_rgba: self.color_rgba,
            cursor,
            selection: selection.to_vec(),
            viewport: Rect::default(),
        };
        self.sync.awareness_mut().set(presence);
    }

    /// Publishes an [`AgentStatus`] over the awareness status channel under
    /// [`AGENT_ACTOR`], so a watcher can narrate the propose-verify-correct loop.
    ///
    /// The status is serialized to JSON and stored in the awareness status slot; a
    /// watcher reads it back with `awareness().status(AGENT_ACTOR)` and deserializes.
    /// Serialization of [`AgentStatus`] is infallible in practice (all fields are
    /// plain scalars); a serialization error is dropped rather than propagated so
    /// narration never breaks the run.
    pub fn publish_status(&mut self, status: &AgentStatus) {
        if let Ok(json) = serde_json::to_string(status) {
            self.sync.awareness_mut().set_status(AGENT_ACTOR, json);
        }
    }

    /// Reads back the [`AgentStatus`] this collaborator last published, if any.
    ///
    /// A convenience mirror of [`publish_status`](Self::publish_status) for a watcher
    /// holding the same document; returns `None` if none was published or the stored
    /// payload does not parse as an [`AgentStatus`].
    #[must_use]
    pub fn published_status(&self) -> Option<AgentStatus> {
        let raw = self.sync.awareness().status(AGENT_ACTOR)?;
        serde_json::from_str(raw).ok()
    }
}

/// Mirrors one command onto the shared step transaction, recording any placed id and
/// updating the running cursor to the placement location.
///
/// Only geometry-creating commands draw; every other command is a no-op on the CRDT
/// (it changes no visible geometry). See the [module docs](self) for the rationale.
fn mirror_command(
    edit: &mut StepEdit,
    command: &AgentCommand,
    placed: &mut Vec<String>,
    cursor: &mut Point,
) {
    match command {
        AgentCommand::CreateCell { name } => {
            edit.add_empty_cell(name);
        }
        AgentCommand::DeleteCell { name } => {
            edit.remove_cell(name);
        }
        AgentCommand::AddRect { cell, layer, rect } => {
            let r = to_rect(*rect);
            let id = edit.add_rect(cell, to_layer(*layer), r);
            *cursor = rect_center(r);
            placed.push(id);
        }
        AgentCommand::AddPolygon {
            cell,
            layer,
            points,
        } => {
            let poly = Polygon::new(points.iter().map(|p| to_point(*p)).collect());
            let center = poly_center(&poly);
            let shape = DrawShape::new(to_layer(*layer), ShapeKind::Polygon(poly));
            let id = edit.add_shape(cell, &shape);
            if let Some(c) = center {
                *cursor = c;
            }
            placed.push(id);
        }
        AgentCommand::AddPath {
            cell,
            layer,
            width,
            points,
            endcap,
        } => {
            let pts: Vec<Point> = points.iter().map(|p| to_point(*p)).collect();
            let last = pts.last().copied();
            let path = Path::new(pts, (*width).max(0), to_endcap(*endcap));
            let shape = DrawShape::new(to_layer(*layer), ShapeKind::Path(path));
            let id = edit.add_shape(cell, &shape);
            if let Some(p) = last {
                *cursor = p;
            }
            placed.push(id);
        }
        AgentCommand::PlaceInstance {
            cell,
            child,
            transform,
        } => {
            if let Ok(t) = to_transform(*transform) {
                let instance = Instance {
                    cell: child.clone(),
                    transform: t,
                };
                *cursor = t.translation;
                let id = edit.add_instance(cell, &instance);
                placed.push(id);
            }
        }
        AgentCommand::PlaceArray {
            cell,
            child,
            transform,
            columns,
            rows,
            column_pitch,
            row_pitch,
        } => {
            if *columns == 0 || *rows == 0 {
                return;
            }
            if let Ok(t) = to_transform(*transform) {
                let array = ArrayInstance {
                    cell: child.clone(),
                    transform: t,
                    columns: *columns,
                    rows: *rows,
                    column_pitch: *column_pitch,
                    row_pitch: *row_pitch,
                };
                *cursor = t.translation;
                let id = edit.add_array(cell, &array);
                placed.push(id);
            }
        }
        // Every other command draws no geometry: read-only queries and reports, DRC,
        // routing, extraction, IO, technology, and session persistence. Id-addressed
        // edits are intentionally not mirrored (see the module docs). The match is
        // exhaustive over the non-exhaustive enum via this arm.
        _ => {}
    }
}

// ----- geometry conversions (mirroring reticle-agent-api's apply module) --------

/// Converts a [`PointArg`] to an engine [`Point`].
fn to_point(p: PointArg) -> Point {
    Point::new(p.x, p.y)
}

/// Converts a [`RectArg`] to an engine [`Rect`] (normalizing corners).
fn to_rect(r: RectArg) -> Rect {
    Rect::new(to_point(r.min), to_point(r.max))
}

/// Converts a [`LayerArg`] to an engine [`LayerId`].
fn to_layer(l: LayerArg) -> LayerId {
    LayerId::new(l.layer, l.datatype)
}

/// Converts an optional [`EndcapArg`] to an engine [`Endcap`] (default flat).
fn to_endcap(e: Option<EndcapArg>) -> Endcap {
    match e {
        None | Some(EndcapArg::Flat) => Endcap::Flat,
        Some(EndcapArg::Square) => Endcap::Square,
        Some(EndcapArg::Round) => Endcap::Round,
    }
}

/// Converts an [`OrientationArg`] to an engine [`Orientation`].
fn to_orientation(o: OrientationArg) -> Orientation {
    match o {
        OrientationArg::R0 => Orientation::R0,
        OrientationArg::R90 => Orientation::R90,
        OrientationArg::R180 => Orientation::R180,
        OrientationArg::R270 => Orientation::R270,
        OrientationArg::MirrorX => Orientation::MirrorX,
        OrientationArg::MirrorX90 => Orientation::MirrorX90,
        OrientationArg::MirrorX180 => Orientation::MirrorX180,
        OrientationArg::MirrorX270 => Orientation::MirrorX270,
    }
}

/// Converts a [`TransformArg`] to an engine [`Transform`], validating the
/// magnification ratio, mirroring `reticle-agent-api`'s conversion so a placement the
/// session accepts is mirrored and one it rejects is skipped.
fn to_transform(t: TransformArg) -> Result<Transform, ()> {
    if t.mag_num <= 0 || t.mag_den <= 0 {
        return Err(());
    }
    let num = u32::try_from(t.mag_num).map_err(|_| ())?;
    let den = u32::try_from(t.mag_den).map_err(|_| ())?;
    let magnification = Magnification::new(num, den).ok_or(())?;
    Ok(Transform {
        translation: Point::new(t.dx, t.dy),
        orientation: to_orientation(t.orientation),
        magnification,
    })
}

/// The center of a rectangle in DBU (each axis widened to avoid overflow).
fn rect_center(r: Rect) -> Point {
    let cx = i64::midpoint(i64::from(r.min.x), i64::from(r.max.x)) as i32;
    let cy = i64::midpoint(i64::from(r.min.y), i64::from(r.max.y)) as i32;
    Point::new(cx, cy)
}

/// The center of a polygon's bounding box, or `None` if it has no vertices.
fn poly_center(p: &Polygon) -> Option<Point> {
    Rect::from_points(p.vertices().iter().copied()).map(rect_center)
}

#[cfg(test)]
mod tests {
    use super::{AgentCollaborator, Pacing, to_transform};
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg, TransformArg};
    use reticle_agent_api::{AGENT_ACTOR, AgentCommand, AgentStatus};
    use reticle_geometry::Point;

    /// A met1 rectangle command from `(x0,y0)` to `(x1,y1)` in `cell`.
    fn rect_cmd(cell: &str, x0: i32, y0: i32, x1: i32, y1: i32) -> AgentCommand {
        AgentCommand::AddRect {
            cell: cell.into(),
            layer: LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: RectArg {
                min: PointArg { x: x0, y: y0 },
                max: PointArg { x: x1, y: y1 },
            },
        }
    }

    #[test]
    fn a_multi_shape_step_lands_as_one_selection_and_moves_the_cursor() {
        let mut agent = AgentCollaborator::new(Pacing::Instant);
        let report = agent.apply_step(&[
            AgentCommand::CreateCell { name: "top".into() },
            rect_cmd("top", 0, 0, 10, 10),
            rect_cmd("top", 20, 0, 40, 20),
        ]);

        // Two shapes placed, so two ids in the selection; the empty-cell create adds
        // no id. Both shapes are in the cell.
        assert_eq!(report.placed.len(), 2, "two rects placed");
        let cell = agent.document().cell("top").expect("cell top");
        assert_eq!(cell.shapes.len(), 2);

        // The cursor sits at the center of the last rect, (30, 10).
        assert_eq!(report.cursor, Point::new(30, 10));

        // Presence was published under AGENT_ACTOR with the selection and cursor.
        let presence = agent
            .sync()
            .awareness()
            .get(AGENT_ACTOR)
            .expect("agent presence");
        assert_eq!(presence.selection, report.placed);
        assert_eq!(presence.cursor, Point::new(30, 10));
        assert_eq!(presence.actor, AGENT_ACTOR);
    }

    #[test]
    fn read_only_commands_draw_nothing() {
        let mut agent = AgentCollaborator::new(Pacing::Instant);
        agent.apply_step(&[AgentCommand::CreateCell { name: "top".into() }]);
        let report = agent.apply_step(&[
            AgentCommand::RunDrc {
                cell: "top".into(),
                region: None,
            },
            AgentCommand::GetViolations,
            AgentCommand::ExportGds,
        ]);
        assert!(report.placed.is_empty(), "no geometry from read-only ops");
        assert_eq!(agent.document().cell("top").unwrap().shapes.len(), 0);
    }

    #[test]
    fn status_round_trips_through_the_awareness_channel() {
        let mut agent = AgentCollaborator::new(Pacing::Instant);
        let status = AgentStatus {
            iteration: 2,
            step: "verifying".into(),
            violations: 1,
            running: true,
        };
        agent.publish_status(&status);
        assert_eq!(agent.published_status(), Some(status));
    }

    #[test]
    fn instant_pacing_is_the_default() {
        assert_eq!(Pacing::default(), Pacing::Instant);
        let agent = AgentCollaborator::default();
        assert_eq!(agent.actor(), AGENT_ACTOR);
    }

    #[test]
    fn a_bad_magnification_is_rejected_like_the_session() {
        // mag_den = 0 is invalid; the placement is skipped rather than mirrored.
        let bad = TransformArg {
            mag_den: 0,
            ..TransformArg::default()
        };
        assert!(to_transform(bad).is_err());
    }
}
