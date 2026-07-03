//! Canned mock scripts for the sample suite.
//!
//! Each sample task id maps to an ordered list of attempts (one command batch per
//! propose-verify-correct iteration). The scripts are chosen to exercise the whole
//! loop: a straight pass, a DRC violation that gets corrected, and a connectivity
//! open that gets bridged. Keeping them here (in the bin, not the library) means the
//! library's [`MockModel`](reticle_bench::MockModel) stays a generic scripted client
//! and the sample specifics live next to the sample suite.

use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_bench::MockModel;

/// GDSII met1 layer/datatype in the SKY130 technology.
const MET1: LayerArg = LayerArg {
    layer: 68,
    datatype: 20,
};

/// A `create_cell top` command; every sample builds its geometry in `top`.
fn create_top() -> AgentCommand {
    AgentCommand::CreateCell { name: "top".into() }
}

/// An axis-aligned met1 rectangle from `(x0, y0)` to `(x1, y1)`.
fn met1_rect(x0: i32, y0: i32, x1: i32, y1: i32) -> AgentCommand {
    AgentCommand::AddRect {
        cell: "top".into(),
        layer: MET1,
        rect: RectArg {
            min: PointArg { x: x0, y: y0 },
            max: PointArg { x: x1, y: y1 },
        },
    }
}

/// The mock model scripted for the sample suite's tasks.
///
/// - `t1_place_met1_rect` (`rect_present`): one clean placement, passes first try.
/// - `t1_drc_clean_met1` (`drc_clean`): an under-width rect first, then a corrected
///   clean rect, so the first proposal is dirty and the final is clean.
/// - `t1_intent_connect` (`intent`): two disjoint rects first (an open net), then a
///   bridging rect that joins the two terminals.
pub fn sample_mock() -> MockModel {
    MockModel::new()
        .with_script(
            "t1_place_met1_rect",
            vec![vec![create_top(), met1_rect(0, 0, 500, 500)]],
        )
        .with_script(
            "t1_drc_clean_met1",
            vec![
                // First proposal: a 100 x 100 met1 rect violates min width (140) and
                // min area (83000).
                vec![create_top(), met1_rect(0, 0, 100, 100)],
                // Correction: delete the offending shape (id 1) and draw a clean
                // 500 x 500 rect.
                vec![
                    AgentCommand::DeleteShapes {
                        ids: vec![reticle_agent_api::ElementId(1)],
                    },
                    met1_rect(0, 0, 500, 500),
                ],
            ],
        )
        .with_script(
            "t1_intent_connect",
            vec![
                // First proposal: two disjoint met1 rects, so the net whose terminals
                // sit on each is open.
                vec![
                    create_top(),
                    met1_rect(0, 0, 100, 100),
                    met1_rect(400, 200, 500, 300),
                ],
                // Correction: add a bridging rect overlapping both, joining them into
                // one net so both terminals are connected.
                vec![met1_rect(90, 0, 410, 300)],
            ],
        )
}

#[cfg(test)]
mod tests {
    use super::sample_mock;
    use reticle_bench::{Context, ModelClient};

    #[test]
    fn every_sample_task_has_a_nonempty_first_attempt() {
        let mut mock = sample_mock();
        for id in [
            "t1_place_met1_rect",
            "t1_drc_clean_met1",
            "t1_intent_connect",
        ] {
            let first = mock.propose(id, "", &Context::default());
            assert!(!first.is_empty(), "{id} must have a first attempt");
        }
    }
}
