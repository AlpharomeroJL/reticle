//! Classroom teaching mode: an instructor broadcasts their current view to a
//! roster of students, a student can opt into following it live, and the
//! instructor can release ("unlock") a student to work independently again
//! (v8.2 Phase 3, ADR 0111).
//!
//! This builds entirely on the existing presence/awareness machinery
//! ([`Awareness`] and [`reticle_sync::Presence`]) and the existing read-only
//! viewer follow-mode ([`crate::viewer::ViewerSession`], ADR 0038); it adds no
//! new wire message and no `reticle-sync` field. [`ClassroomState`] is the
//! egui-free model: who is known (the roster, derived from [`Awareness`] exactly as
//! [`crate::viewer::participants`] already does for the session chip), whose
//! role is instructor versus student, this peer's local record of who is
//! currently following, and the instructor's last "bring everyone" broadcast
//! target. [`roster_panel`] is the thin egui rendering pass over that state,
//! styled entirely from [`crate::theme::components`]/[`crate::theme::tokens`]
//! (the check-style lint bans raw colors and font sizes outside `crate::theme`);
//! a headless-context test drives it too (no window, no GPU).
//!
//! # What is real today, and what is app-side bookkeeping
//!
//! A student's "Follow instructor" toggle is real: it flips the same
//! [`crate::viewer::ViewerSession`] follow flag the existing session chip's
//! checkbox does, so the canvas's existing per-frame `sync_camera` snaps the
//! student's camera to the instructor's live-published viewport (`Presence`
//! field 6, already flowing over the wire, ADR 0038). No new transport code was
//! needed for that half.
//!
//! The instructor's roster, by contrast, only ever lists actors that have
//! *published* presence into this session. Today the app publishes presence
//! from exactly one identity, [`crate::livesync::SHARER_ACTOR`] (`local_presence`
//! in `crate::app`); a read-only viewer (ADR 0038) never publishes at all by
//! design. So until a future lane wires a write-capable "join as a collaborator
//! and publish my own presence" path, an instructor's live roster is
//! honestly empty (rendered as an [`EmptyState`](crate::theme::components::EmptyState),
//! not a fake row). [`ClassroomState::bring_everyone`] and
//! [`ClassroomState::unlock_student`] are still fully real, tested state
//! transitions over whatever roster *is* known (constructed directly in tests,
//! matching the brief: no contract fixture, no live rig needed) so the moment
//! that publish path lands, the instructor side lights up with no change here.
//!
//! Multi-machine classrooms additionally depend on a deployed relay: the share
//! server default stays `127.0.0.1:3030` (`crate::share::DEFAULT_SERVER`); the
//! public-relay story is operator-owned, tracked as backlog item H1
//! (`scratch/campaign/v82-backlog.md`). This module does not change that
//! default and does not attempt to work around it.

use eframe::egui::{self, Sense, Vec2};
use reticle_geometry::Rect;
use reticle_sync::Awareness;

use crate::theme::{self, components::Ctx};

/// A roster member's role in a classroom session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The peer whose view the roster follows. At most one per room today (see
    /// the module docs): the well-known [`crate::livesync::SHARER_ACTOR`] identity.
    Instructor,
    /// Every other known collaborator.
    Student,
}

/// One roster row: a collaborator's published identity (actor id, display name,
/// color, exactly as [`crate::viewer::participants`] resolves them) plus this
/// peer's local record of whether they are currently following the instructor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RosterEntry {
    /// The actor id this row is keyed by.
    pub actor: String,
    /// The display name shown in the roster (falls back to a short actor label).
    pub name: String,
    /// The packed `0xRRGGBBAA` color for the roster dot.
    pub color_rgba: u32,
    /// Instructor or student.
    pub role: Role,
    /// Whether this peer's local bookkeeping currently considers `actor` to be
    /// following the instructor's broadcast view.
    pub following: bool,
}

/// Classroom teaching-mode state: the roster, each known peer's follow state,
/// and the instructor's last broadcast view target. Egui-free and window-free
/// (see the module docs for what draws it: [`roster_panel`]).
#[derive(Clone, Debug)]
pub struct ClassroomState {
    self_actor: String,
    instructor_actor: String,
    roster: Vec<RosterEntry>,
    broadcast_target: Option<Rect>,
}

impl ClassroomState {
    /// An empty classroom: nobody synced yet. `self_actor` is this peer's own
    /// identity (excluded from its own roster, mirroring
    /// [`crate::viewer::participants`]); `instructor_actor` is the identity the
    /// roster treats as [`Role::Instructor`]. Passing the same id for both marks
    /// this peer itself as the instructor ([`ClassroomState::is_instructor`]).
    #[must_use]
    pub fn new(self_actor: impl Into<String>, instructor_actor: impl Into<String>) -> Self {
        Self {
            self_actor: self_actor.into(),
            instructor_actor: instructor_actor.into(),
            roster: Vec::new(),
            broadcast_target: None,
        }
    }

    /// This peer's own actor id.
    #[must_use]
    pub fn self_actor(&self) -> &str {
        &self.self_actor
    }

    /// The actor id the roster treats as the instructor.
    #[must_use]
    pub fn instructor_actor(&self) -> &str {
        &self.instructor_actor
    }

    /// Whether this peer *is* the instructor (its own actor id is the
    /// instructor id).
    #[must_use]
    pub fn is_instructor(&self) -> bool {
        self.self_actor == self.instructor_actor
    }

    /// The current roster, in the stable actor-sorted order
    /// [`crate::viewer::participants`] produces.
    #[must_use]
    pub fn roster(&self) -> &[RosterEntry] {
        &self.roster
    }

    /// The instructor's last broadcast view (set by
    /// [`ClassroomState::bring_everyone`]), if any.
    #[must_use]
    pub fn broadcast_target(&self) -> Option<Rect> {
        self.broadcast_target
    }

    /// Rebuilds the roster from the live `awareness` map (see the module docs
    /// for what populates it today).
    ///
    /// Every actor in `awareness` other than [`self_actor`](Self::self_actor)
    /// becomes a row, classified [`Role::Instructor`] when it matches
    /// [`instructor_actor`](Self::instructor_actor) and [`Role::Student`]
    /// otherwise. A row's [`following`](RosterEntry::following) flag survives a
    /// re-sync for an actor still present; an actor no longer in `awareness` is
    /// dropped, and a newly-seen actor starts `following: false`.
    pub fn sync_roster(&mut self, awareness: &Awareness) {
        let participants = crate::viewer::participants(awareness, &self.self_actor);
        self.roster = participants
            .into_iter()
            .map(|p| {
                let following = self.is_following(&p.actor);
                let role = if p.actor == self.instructor_actor {
                    Role::Instructor
                } else {
                    Role::Student
                };
                RosterEntry {
                    actor: p.actor,
                    name: p.name,
                    color_rgba: p.color_rgba,
                    role,
                    following,
                }
            })
            .collect();
    }

    /// Whether `actor` is currently recorded as following, per the last sync.
    #[must_use]
    fn is_following(&self, actor: &str) -> bool {
        self.roster.iter().any(|e| e.actor == actor && e.following)
    }

    /// The instructor's "bring everyone here" broadcast: records `viewport` as
    /// the [`broadcast_target`](Self::broadcast_target) and marks every student
    /// in the roster as following, so a currently-following student's next
    /// camera sync (`crate::viewer::follow_camera`, over the instructor's
    /// live-published `Presence.viewport`) lands exactly there.
    ///
    /// A no-op on the roster when it is empty (nothing to mark), though
    /// `broadcast_target` is still recorded.
    pub fn bring_everyone(&mut self, viewport: Rect) {
        self.broadcast_target = Some(viewport);
        for entry in &mut self.roster {
            if entry.role == Role::Student {
                entry.following = true;
            }
        }
    }

    /// Sets whether `actor` is recorded as following. Returns `true` if `actor`
    /// was found in the roster (and so the flag was actually set); `false` for
    /// an unknown actor, which leaves the roster unchanged.
    pub fn set_following(&mut self, actor: &str, following: bool) -> bool {
        match self.roster.iter_mut().find(|e| e.actor == actor) {
            Some(entry) => {
                entry.following = following;
                true
            }
            None => false,
        }
    }

    /// The instructor releases `actor` to work independently: clears their
    /// `following` flag. Returns `true` if `actor` was found in the roster.
    pub fn unlock_student(&mut self, actor: &str) -> bool {
        self.set_following(actor, false)
    }
}

/// A user action the roster panel produced this frame.
///
/// This module has no access to the live document, camera, or
/// [`crate::viewer::ViewerSession`], so the caller (`App`) applies the effect:
/// [`RosterAction::BringEveryone`] reads the caller's own current viewport and
/// calls [`ClassroomState::bring_everyone`]; [`RosterAction::ToggleFollow`]
/// flips the caller's `ViewerSession` follow flag (the real camera-sync path,
/// see the module docs); [`RosterAction::UnlockStudent`] calls
/// [`ClassroomState::unlock_student`] with the carried actor id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RosterAction {
    /// The instructor clicked "Bring everyone here".
    BringEveryone,
    /// The instructor clicked "Unlock" on this student's row (their actor id).
    UnlockStudent(String),
    /// A student toggled their own "Follow instructor" control.
    ToggleFollow,
}

/// Draws the classroom roster section: for the instructor
/// ([`ClassroomState::is_instructor`]), every known student with a
/// following/independent indicator and a per-row "Unlock" action, plus a "Bring
/// everyone here" broadcast button; an empty roster renders an
/// [`EmptyState`](theme::components::EmptyState) naming why (see the module
/// docs), not a blank panel. For a student, the instructor's row (once their
/// presence has arrived) and a "Follow instructor" toggle.
///
/// `following` is the caller's own current follow-mode flag (student side
/// only; typically [`crate::viewer::ViewerSession::is_following`]) since this
/// module does not track the caller's own follow state, only other actors'
/// (see [`ClassroomState::sync_roster`]). Pass `None` when the caller has no
/// [`crate::viewer::ViewerSession`] (an instructor never needs the toggle).
///
/// Returns the action a click produced this frame, if any.
pub fn roster_panel(
    ui: &mut egui::Ui,
    cx: Ctx,
    state: &ClassroomState,
    following: Option<bool>,
) -> Option<RosterAction> {
    let mut action = None;
    theme::components::SectionHeader::new("Classroom").show(ui, cx);
    ui.add_space(cx.density.item_spacing().y);
    if state.is_instructor() {
        instructor_rows(ui, cx, state, &mut action);
    } else {
        student_row(ui, cx, state, following, &mut action);
    }
    action
}

/// The instructor's half of [`roster_panel`]: the broadcast button and one row
/// per known student.
fn instructor_rows(
    ui: &mut egui::Ui,
    cx: Ctx,
    state: &ClassroomState,
    action: &mut Option<RosterAction>,
) {
    if state.roster.is_empty() {
        theme::components::EmptyState::new(
            "No students yet",
            "Share the collaborator link so students can join (Share section). A \
             classroom that spans machines needs the operator relay, tracked as H1.",
        )
        .show(ui, cx);
        return;
    }
    if theme::components::Button::secondary("Bring everyone here")
        .show(ui, cx)
        .clicked()
    {
        *action = Some(RosterAction::BringEveryone);
    }
    ui.add_space(cx.density.item_spacing().y);
    for entry in &state.roster {
        ui.horizontal(|ui| {
            roster_dot(ui, entry.color_rgba);
            ui.label(&entry.name);
            let status_color = if entry.following {
                cx.tokens.success
            } else {
                cx.tokens.text_weak
            };
            ui.label(
                egui::RichText::new(if entry.following {
                    "Following"
                } else {
                    "Independent"
                })
                .color(status_color),
            );
            if entry.following
                && theme::components::Button::ghost("Unlock")
                    .show(ui, cx)
                    .clicked()
            {
                *action = Some(RosterAction::UnlockStudent(entry.actor.clone()));
            }
        });
    }
}

/// The student's half of [`roster_panel`]: the instructor's row and the follow
/// toggle.
fn student_row(
    ui: &mut egui::Ui,
    cx: Ctx,
    state: &ClassroomState,
    following: Option<bool>,
    action: &mut Option<RosterAction>,
) {
    let Some(instructor) = state.roster.iter().find(|e| e.role == Role::Instructor) else {
        theme::components::EmptyState::new(
            "Not connected",
            "Waiting for the instructor to go live.",
        )
        .show(ui, cx);
        return;
    };
    ui.horizontal(|ui| {
        roster_dot(ui, instructor.color_rgba);
        ui.label(&instructor.name);
    });
    if let Some(is_following) = following
        && theme::components::ToggleChip::new("Follow instructor", is_following)
            .show(ui, cx)
            .clicked()
    {
        *action = Some(RosterAction::ToggleFollow);
    }
}

/// A small filled circle in `color_rgba`, matching the session chip's avatar
/// coloring (`crate::app::App::avatar`) at roster-row scale.
fn roster_dot(ui: &mut egui::Ui, color_rgba: u32) {
    let (r, g, b, _a) = crate::layers::rgba_components(color_rgba);
    let color = theme::tokens::layer_rgb(r, g, b);
    let (rect, _response) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

#[cfg(test)]
mod tests {
    use super::{ClassroomState, Role, roster_panel};
    use crate::theme::components::Ctx;
    use eframe::egui;
    use reticle_geometry::{Point, Rect};
    use reticle_sync::{Awareness, Presence};

    /// An `Awareness` with one `Presence` per `(actor, display_name, color_rgba)`.
    fn awareness_of(entries: &[(&str, &str, u32)]) -> Awareness {
        let mut aw = Awareness::new();
        for &(actor, name, color) in entries {
            let mut p = Presence::new(actor);
            p.display_name = name.to_owned();
            p.color_rgba = color;
            aw.set(p);
        }
        aw
    }

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    #[test]
    fn is_instructor_when_self_and_instructor_ids_match() {
        let mine = ClassroomState::new("sharer", "sharer");
        assert!(mine.is_instructor());
        let student = ClassroomState::new("viewer", "sharer");
        assert!(!student.is_instructor());
    }

    #[test]
    fn a_fresh_state_has_no_roster_and_no_broadcast_target() {
        let state = ClassroomState::new("sharer", "sharer");
        assert!(state.roster().is_empty());
        assert_eq!(state.broadcast_target(), None);
    }

    #[test]
    fn sync_roster_reflects_presence_and_excludes_self() {
        let aw = awareness_of(&[
            ("sharer", "Ms. Chen", 0x2f_81_f7_ff),
            ("alice", "Alice", 0xe5_48_4d_ff),
        ]);
        let mut state = ClassroomState::new("viewer", "sharer");
        state.sync_roster(&aw);

        let actors: Vec<&str> = state.roster().iter().map(|e| e.actor.as_str()).collect();
        assert_eq!(actors, vec!["alice", "sharer"], "sorted, self excluded");

        let instructor = state
            .roster()
            .iter()
            .find(|e| e.actor == "sharer")
            .expect("instructor row present");
        assert_eq!(instructor.role, Role::Instructor);
        assert_eq!(instructor.name, "Ms. Chen");
        assert_eq!(instructor.color_rgba, 0x2f_81_f7_ff);
        assert!(!instructor.following, "nobody follows by default");

        let student = state
            .roster()
            .iter()
            .find(|e| e.actor == "alice")
            .expect("student row present");
        assert_eq!(student.role, Role::Student);
        assert_eq!(student.name, "Alice");
    }

    #[test]
    fn self_never_appears_in_its_own_roster() {
        let aw = awareness_of(&[("sharer", "Ms. Chen", 0)]);
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&aw);
        assert!(
            state.roster().is_empty(),
            "the instructor's own presence is not a roster row"
        );
    }

    #[test]
    fn sync_roster_preserves_following_and_drops_absent_actors() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0), ("bob", "Bob", 0)]));
        assert!(state.set_following("alice", true));

        // Bob drops off, carol joins; alice's following flag must survive the resync.
        state.sync_roster(&awareness_of(&[
            ("alice", "Alice", 0),
            ("carol", "Carol", 0),
        ]));
        let actors: Vec<&str> = state.roster().iter().map(|e| e.actor.as_str()).collect();
        assert_eq!(actors, vec!["alice", "carol"], "bob dropped, carol added");
        assert!(
            state
                .roster()
                .iter()
                .find(|e| e.actor == "alice")
                .unwrap()
                .following,
            "alice's following flag survives the resync"
        );
        assert!(
            !state
                .roster()
                .iter()
                .find(|e| e.actor == "carol")
                .unwrap()
                .following,
            "a newly-seen actor starts not following"
        );
    }

    #[test]
    fn set_following_transitions_and_reports_whether_the_actor_was_found() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0)]));

        assert!(state.set_following("alice", true));
        assert!(state.roster()[0].following);
        assert!(state.set_following("alice", false));
        assert!(!state.roster()[0].following);
        assert!(
            !state.set_following("ghost", true),
            "an unknown actor is reported as not found"
        );
    }

    #[test]
    fn bring_everyone_sets_the_broadcast_target_and_follows_every_student() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0), ("bob", "Bob", 0)]));
        assert!(state.roster().iter().all(|e| !e.following));

        let viewport = rect(-1000, -2000, 3000, 4000);
        state.bring_everyone(viewport);

        assert_eq!(state.broadcast_target(), Some(viewport));
        assert!(
            state.roster().iter().all(|e| e.following),
            "bring_everyone sets every student's follow target"
        );
    }

    #[test]
    fn bring_everyone_on_an_empty_roster_still_records_the_target() {
        let mut state = ClassroomState::new("sharer", "sharer");
        let viewport = rect(0, 0, 10, 10);
        state.bring_everyone(viewport);
        assert_eq!(state.broadcast_target(), Some(viewport));
        assert!(state.roster().is_empty());
    }

    #[test]
    fn unlock_student_clears_follow_for_only_that_student() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0), ("bob", "Bob", 0)]));
        state.bring_everyone(rect(0, 0, 100, 100));
        assert!(state.roster().iter().all(|e| e.following));

        assert!(state.unlock_student("alice"));
        let alice = state.roster().iter().find(|e| e.actor == "alice").unwrap();
        let bob = state.roster().iter().find(|e| e.actor == "bob").unwrap();
        assert!(!alice.following, "alice was unlocked");
        assert!(bob.following, "bob is untouched");
    }

    #[test]
    fn unlock_student_on_an_unknown_actor_is_a_reported_no_op() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0)]));
        assert!(!state.unlock_student("ghost"));
        assert!(state.roster()[0].actor == "alice" && !state.roster()[0].following);
    }

    #[test]
    fn instructor_role_is_assigned_by_actor_id_not_publish_order() {
        let mut state = ClassroomState::new("viewer", "sharer");
        state.sync_roster(&awareness_of(&[
            ("alice", "Alice", 0),
            ("sharer", "Chen", 0),
        ]));
        assert_eq!(
            state
                .roster()
                .iter()
                .find(|e| e.actor == "sharer")
                .unwrap()
                .role,
            Role::Instructor
        );
        assert_eq!(
            state
                .roster()
                .iter()
                .find(|e| e.actor == "alice")
                .unwrap()
                .role,
            Role::Student
        );
    }

    // -- roster_panel: headless (no GPU, no window; ADR pattern shared with
    // -- crate::gallery / crate::trace_panel's own render tests) --------------

    fn ctx_dark() -> Ctx {
        Ctx::dark(crate::theme::tokens::Density::default())
    }

    #[test]
    fn instructor_panel_with_an_empty_roster_renders_without_panic() {
        let state = ClassroomState::new("sharer", "sharer");
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let mut action = None;
        egui::Window::new("classroom test empty").show(&ctx, |ui| {
            action = roster_panel(ui, ctx_dark(), &state, None);
        });
        let _ = ctx.end_pass();
        assert_eq!(action, None, "a synthetic pass clicks nothing");
    }

    #[test]
    fn instructor_panel_with_students_renders_without_panic() {
        let mut state = ClassroomState::new("sharer", "sharer");
        state.sync_roster(&awareness_of(&[("alice", "Alice", 0xe5_48_4d_ff)]));
        state.bring_everyone(rect(0, 0, 10, 10));
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let mut action = None;
        egui::Window::new("classroom test roster").show(&ctx, |ui| {
            action = roster_panel(ui, ctx_dark(), &state, None);
        });
        let _ = ctx.end_pass();
        assert_eq!(action, None);
    }

    #[test]
    fn student_panel_with_an_instructor_renders_without_panic() {
        let mut state = ClassroomState::new("viewer", "sharer");
        state.sync_roster(&awareness_of(&[("sharer", "Ms. Chen", 0x2f_81_f7_ff)]));
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let mut action = None;
        egui::Window::new("classroom test student").show(&ctx, |ui| {
            action = roster_panel(ui, ctx_dark(), &state, Some(false));
        });
        let _ = ctx.end_pass();
        assert_eq!(action, None);
    }

    #[test]
    fn student_panel_before_the_instructor_is_known_renders_without_panic() {
        let state = ClassroomState::new("viewer", "sharer");
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let mut action = None;
        egui::Window::new("classroom test waiting").show(&ctx, |ui| {
            action = roster_panel(ui, ctx_dark(), &state, Some(false));
        });
        let _ = ctx.end_pass();
        assert_eq!(action, None);
    }
}
