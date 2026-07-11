//! Second-oracle agreement, adapter-gated (honest run-or-skip).
//!
//! This mirrors the `tt_precheck` / `lefdef_oracle` container-oracle tests: it RUNS when a
//! vision model is present on this host (and the render path has a GPU adapter), and SKIPS
//! honestly with a printed reason otherwise. It never fails a gate for a missing model or
//! a headless host, and it never fabricates a verdict.
//!
//! When it runs it renders two committed fixtures through the same `RenderPng` path the
//! run writer uses, asks the vision oracle beside the authoritative
//! [`RectPresent`](reticle_bench::RectPresent) checker, and reports the measured agreement
//! rate:
//!
//! - a *faithful* layout (cell `top` with a large met1 rectangle) that the checker passes,
//!   and
//! - a *corrupt* layout (cell `top` with no geometry) that the checker fails.
//!
//! The robust invariant asserted (independent of the vision model's non-deterministic
//! answer) is that the corrupt fixture is caught by at least one oracle. The agreement
//! rate between the two oracles is measured and printed for the RESULT record, not
//! asserted, so a best-effort vision model can never make the gate flaky.

use reticle_agent::vision_oracle::{
    AgreementTally, VisionOracle, VisionOutcome, caught_by_any_oracle, oracles_agree,
};
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_agent_api::{AgentCommand, Session, Transcript};
use reticle_bench::{CheckResult, Checker, RectPresent};

/// A met1 (68/20) rectangle command in cell `top` spanning `(x0, y0)..(x1, y1)` DBU.
fn met1_rect_at(x0: i32, y0: i32, x1: i32, y1: i32) -> AgentCommand {
    AgentCommand::AddRect {
        cell: "top".into(),
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

/// A faithful fixture: cell `top` with four separated met1 rectangles in a 2x2 grid.
///
/// Separated rectangles (with dark gaps between them) render as several distinct filled
/// squares on a dark background rather than one frame-filling solid color, so the render
/// reads unambiguously as "a layout region with filled rectangles" for the vision oracle.
fn faithful_session() -> Session {
    let mut session = Session::new();
    session
        .apply(AgentCommand::CreateCell { name: "top".into() })
        .expect("create cell");
    for (x0, y0) in [(0, 0), (600, 0), (0, 600), (600, 600)] {
        session
            .apply(met1_rect_at(x0, y0, x0 + 400, y0 + 400))
            .expect("add rect");
    }
    session
}

/// A corrupt fixture: cell `top` with no geometry at all (an empty layout).
fn empty_session() -> Session {
    let mut session = Session::new();
    session
        .apply(AgentCommand::CreateCell { name: "top".into() })
        .expect("create cell");
    session
}

/// Whether the authoritative `RectPresent` checker passes for `session`.
fn authoritative_pass(session: &Session) -> bool {
    let checker = RectPresent::new(68, 20);
    matches!(
        checker.check(session.document(), &Transcript::default()),
        CheckResult::Pass
    )
}

#[test]
fn second_oracle_agreement_runs_or_skips_honestly() {
    let oracle = VisionOracle::from_env();

    // Adapter gate: server down/unreachable or the model not pulled -> honest skip. The
    // probe is a bounded HTTP call (never an unbounded CLI subprocess, which once hung a
    // gate for 25 minutes via Windows Ollama auto-start pipe inheritance).
    if let Err(reason) = oracle.availability() {
        eprintln!("vision second-oracle SKIPPED (honest not-run): {reason}");
        return;
    }

    // The intent both oracles are judging: does the render show drawn metal geometry?
    let intent = "one or more filled metal rectangles (a non-empty drawn layout region)";

    let faithful = faithful_session();
    let empty = empty_session();

    // Authoritative (deterministic) verdicts. These are the anchor the vision oracle is
    // compared against, and they must be exactly what the fixtures were built to produce.
    let faithful_pass = authoritative_pass(&faithful);
    let empty_pass = authoritative_pass(&empty);
    assert!(
        faithful_pass,
        "the faithful fixture must pass the authoritative checker"
    );
    assert!(
        !empty_pass,
        "the empty fixture must fail the authoritative checker"
    );

    // Vision verdicts: render each fixture and ask the model. A render skip (no GPU) or a
    // transport hiccup is an honest not-run, so bail out with a printed reason rather than
    // fail. The first render exercises the GPU path; if it is unavailable we skip here.
    let v_faithful = oracle.verify_session(&faithful, intent);
    if let VisionOutcome::Skipped(reason) = &v_faithful {
        eprintln!("vision second-oracle SKIPPED (honest not-run): {reason}");
        return;
    }
    let v_empty = oracle.verify_session(&empty, intent);

    let (VisionOutcome::Ran(vf), VisionOutcome::Ran(ve)) = (&v_faithful, &v_empty) else {
        eprintln!(
            "vision second-oracle SKIPPED (honest not-run): model/render became unavailable mid-run"
        );
        return;
    };

    // Measured agreement between the two oracles over the fixtures (reported, not asserted:
    // the vision model is best-effort and non-deterministic).
    let mut tally = AgreementTally::new();
    tally.record(oracles_agree(vf, faithful_pass));
    tally.record(oracles_agree(ve, empty_pass));

    eprintln!(
        "vision second-oracle RAN: model={}, agreement_rate={:.0}% ({}/{})",
        oracle.model(),
        tally.rate() * 100.0,
        tally.agreements,
        tally.total,
    );
    eprintln!(
        "  faithful: authoritative_pass={faithful_pass}, vision_matches={} :: {}",
        vf.matches, vf.rationale
    );
    eprintln!(
        "  empty:    authoritative_pass={empty_pass}, vision_matches={} :: {}",
        ve.matches, ve.rationale
    );

    // Robust invariant, independent of the vision model's answer: the corrupt (empty)
    // fixture is caught by at least one oracle. The authoritative checker always catches
    // it, so this never depends on the vision model being correct; it documents the
    // two-oracle safety property the fixture pair exists to demonstrate.
    assert!(
        caught_by_any_oracle(ve.matches, empty_pass),
        "the empty layout must be caught by at least one oracle"
    );

    // The oracle ran and produced a rate in the valid range.
    assert!(
        (0.0..=1.0).contains(&tally.rate()),
        "agreement rate must be a fraction"
    );
}
