//! Keyboard focus traversal and the Escape cascade (lane 3C, catalog 83 and 84).
//!
//! Two small pure state machines the app glue drives:
//!
//! * [`FocusRegion`] is the ring F6 walks so every major region (toolbar, the two
//!   side panels, the canvas) is keyboard-reachable and shows a visible focus ring.
//!   The order and wraparound live here so they are unit-testable; the app maps
//!   each region to an `egui` id it requests focus for.
//! * [`esc_action`] resolves one Escape press to the single [`EscAction`] to take,
//!   following the documented cascade: **cancel the active tool, then clear the
//!   selection, then close a popover**. Pressing Escape repeatedly peels one layer
//!   at a time in that order, so the contract is exactly what the test asserts.

use eframe::egui::Id;

/// A keyboard-focusable region of the editor, in the order F6 cycles them.
///
/// The ring is deliberately small and fixed: the top toolbar, the left (Layers)
/// panel, the canvas, and the right (Inspector) panel. F6 advances forward and
/// Shift+F6 backward, both wrapping, so focus is never trapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusRegion {
    /// The top toolbar / menu bar row.
    Toolbar,
    /// The left panel (Layers).
    LeftPanel,
    /// The layout canvas.
    Canvas,
    /// The right panel (Inspector).
    RightPanel,
}

impl FocusRegion {
    /// Every region in ring order.
    #[must_use]
    pub fn all() -> [FocusRegion; 4] {
        [
            FocusRegion::Toolbar,
            FocusRegion::LeftPanel,
            FocusRegion::Canvas,
            FocusRegion::RightPanel,
        ]
    }

    /// The next region in the ring (wraps after the last), as F6 advances focus.
    #[must_use]
    pub fn next(self) -> FocusRegion {
        let ring = FocusRegion::all();
        let i = ring.iter().position(|&r| r == self).unwrap_or(0);
        ring[(i + 1) % ring.len()]
    }

    /// The previous region in the ring (wraps before the first), for Shift+F6.
    #[must_use]
    pub fn prev(self) -> FocusRegion {
        let ring = FocusRegion::all();
        let i = ring.iter().position(|&r| r == self).unwrap_or(0);
        ring[(i + ring.len() - 1) % ring.len()]
    }

    /// A short human-readable name, shown in the status bar when focus moves.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FocusRegion::Toolbar => "Toolbar",
            FocusRegion::LeftPanel => "Layers panel",
            FocusRegion::Canvas => "Canvas",
            FocusRegion::RightPanel => "Inspector panel",
        }
    }

    /// The stable `egui` id of this region's focus anchor, so the app can request
    /// keyboard focus for it and paint a focus ring there.
    #[must_use]
    pub fn anchor_id(self) -> Id {
        Id::new(("focus-region", self.label()))
    }
}

/// The one effect an Escape press produces, resolved by [`esc_action`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EscAction {
    /// Cancel the active tool (abort an in-progress draw / return to Select).
    CancelTool,
    /// Clear the current selection.
    ClearSelection,
    /// Close the top-most popover (palette, overlay, or context menu).
    ClosePopover,
    /// Nothing to do; let Escape fall through.
    Nothing,
}

/// The editor state the Escape cascade inspects: whether a tool operation is
/// cancelable, whether anything is selected, and whether a popover is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EscState {
    /// A tool is active with something to cancel (an in-progress draw, or a
    /// non-Select tool that Escape returns to Select).
    pub tool_active: bool,
    /// The selection is non-empty.
    pub has_selection: bool,
    /// A popover (palette, shortcuts overlay, context menu) is open.
    pub popover_open: bool,
}

/// Resolves one Escape press to the single action to take.
///
/// The cascade order is **tool, then selection, then popover**: an in-progress
/// tool is the most transient state, so the first Escape cancels it; the next
/// clears the selection; the last closes a lingering popover (which the user can
/// also dismiss by clicking away). Returning one action per press makes repeated
/// Escapes peel exactly one layer at a time, which is the tested contract.
#[must_use]
pub fn esc_action(state: EscState) -> EscAction {
    if state.tool_active {
        EscAction::CancelTool
    } else if state.has_selection {
        EscAction::ClearSelection
    } else if state.popover_open {
        EscAction::ClosePopover
    } else {
        EscAction::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f6_cycles_forward_through_every_region_and_wraps() {
        assert_eq!(FocusRegion::Toolbar.next(), FocusRegion::LeftPanel);
        assert_eq!(FocusRegion::LeftPanel.next(), FocusRegion::Canvas);
        assert_eq!(FocusRegion::Canvas.next(), FocusRegion::RightPanel);
        assert_eq!(FocusRegion::RightPanel.next(), FocusRegion::Toolbar);
    }

    #[test]
    fn shift_f6_cycles_backward_and_wraps() {
        assert_eq!(FocusRegion::Toolbar.prev(), FocusRegion::RightPanel);
        assert_eq!(FocusRegion::RightPanel.prev(), FocusRegion::Canvas);
        assert_eq!(FocusRegion::Canvas.prev(), FocusRegion::LeftPanel);
        assert_eq!(FocusRegion::LeftPanel.prev(), FocusRegion::Toolbar);
    }

    #[test]
    fn next_and_prev_are_inverses_for_every_region() {
        for r in FocusRegion::all() {
            assert_eq!(r.next().prev(), r, "{r:?}");
            assert_eq!(r.prev().next(), r, "{r:?}");
        }
    }

    #[test]
    fn esc_cancels_the_tool_first() {
        let s = EscState {
            tool_active: true,
            has_selection: true,
            popover_open: true,
        };
        assert_eq!(esc_action(s), EscAction::CancelTool);
    }

    #[test]
    fn esc_clears_selection_after_the_tool() {
        let s = EscState {
            tool_active: false,
            has_selection: true,
            popover_open: true,
        };
        assert_eq!(esc_action(s), EscAction::ClearSelection);
    }

    #[test]
    fn esc_closes_the_popover_last() {
        let s = EscState {
            tool_active: false,
            has_selection: false,
            popover_open: true,
        };
        assert_eq!(esc_action(s), EscAction::ClosePopover);
    }

    #[test]
    fn esc_does_nothing_when_there_is_nothing_to_peel() {
        assert_eq!(esc_action(EscState::default()), EscAction::Nothing);
    }
}
