//! Failure mining: cluster failed and struggling benchmark runs into
//! candidate tasks.
//!
//! The scanner ([`scan`]) takes finished runs, each a [`ResultRecord`] paired
//! with the [`Transcript`] that produced it and the [`Tier`] it ran at, selects
//! the ones worth mining (failed outright, or needed
//! [`MiningOptions::high_iteration_threshold`] or more iterations), and groups
//! them by a failure [`Signature`] with three components:
//!
//! - the *persistent DRC rule ids*: the intersection of every DRC report in
//!   the transcript (the `run_drc` and `get_violations` data payloads), so a
//!   rule counts only if no correction attempt ever cleared it;
//! - a *geometric-pattern class* ([`PatternClass`]) derived from the drawing
//!   commands the model issued;
//! - an *intent-violation kind* ([`IntentViolationKind`]) read from the last
//!   `check_intent` report, if the transcript carries one.
//!
//! Runs sharing a signature form a [`Cluster`]; a cluster of at least
//! [`MiningOptions::min_cluster_size`] runs marks a recurring failure mode
//! that is a candidate for a new benchmark task.

use std::collections::{BTreeMap, BTreeSet};

use reticle_agent_api::{AgentCommand, AgentResponse, Outcome, Transcript};

use crate::{ResultRecord, Tier};

/// One finished run offered to the miner.
///
/// A [`ResultRecord`] does not carry its tier (mirroring
/// [`summarize`](crate::summarize), the caller supplies it from the
/// originating task), and the transcript is the command-level evidence the
/// signature is extracted from.
#[derive(Clone, Debug)]
pub struct MinedRun {
    /// The tier of the task this run executed.
    pub tier: Tier,
    /// The run's result row.
    pub record: ResultRecord,
    /// The full command transcript of the run.
    pub transcript: Transcript,
}

/// Tunable thresholds for the scanner.
#[derive(Clone, Debug)]
pub struct MiningOptions {
    /// A run with at least this many iterations is mined even if it passed:
    /// needing most of the iteration budget marks a task the model struggled
    /// with.
    pub high_iteration_threshold: u32,
    /// The minimum number of runs a cluster needs to become a candidate; a
    /// smaller cluster is treated as noise, not a recurring failure mode.
    pub min_cluster_size: usize,
    /// The technology file path, relative to the suite root, that drafted
    /// candidate tasks reference.
    pub technology: String,
}

impl Default for MiningOptions {
    fn default() -> Self {
        Self {
            high_iteration_threshold: 3,
            min_cluster_size: 2,
            technology: "sky130.tech".into(),
        }
    }
}

/// The geometric-pattern class of a run, derived from the transcript's drawing
/// commands.
///
/// Classification uses a fixed priority so every transcript maps to exactly
/// one class: hierarchy placement beats path drawing beats polygon drawing
/// beats a multi-layer rectangle stack beats plain rectangles; a transcript
/// with no drawing command at all is [`PatternClass::NoGeometry`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum PatternClass {
    /// The run placed instances or arrays (`place_instance` / `place_array`).
    Hierarchical,
    /// The run drew at least one path.
    PathRouting,
    /// The run drew at least one polygon (and no path).
    PolygonHeavy,
    /// The run drew rectangles on three or more distinct layers.
    LayerStack,
    /// The run drew rectangles on one or two layers and nothing else.
    RectOnly,
    /// The run issued no drawing command at all.
    NoGeometry,
}

impl PatternClass {
    /// The stable `snake_case` token for this class, used in signature keys,
    /// candidate ids, and provenance.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PatternClass::Hierarchical => "hierarchical",
            PatternClass::PathRouting => "path_routing",
            PatternClass::PolygonHeavy => "polygon_heavy",
            PatternClass::LayerStack => "layer_stack",
            PatternClass::RectOnly => "rect_only",
            PatternClass::NoGeometry => "no_geometry",
        }
    }
}

/// The intent-violation kind a run ended with, read from the last
/// `check_intent` report in the transcript.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum IntentViolationKind {
    /// No `check_intent` report, or the last one was clean.
    None,
    /// The last report carried at least one open and no short.
    Open,
    /// The last report carried at least one short and no open.
    Short,
    /// The last report carried both opens and shorts.
    OpenAndShort,
}

impl IntentViolationKind {
    /// The stable `snake_case` token for this kind, used in signature keys,
    /// candidate ids, and provenance.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            IntentViolationKind::None => "none",
            IntentViolationKind::Open => "open",
            IntentViolationKind::Short => "short",
            IntentViolationKind::OpenAndShort => "open_and_short",
        }
    }
}

/// A failure signature: the clustering key the scanner groups mined runs by.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Signature {
    /// DRC rule ids present in every DRC report of the transcript (never
    /// cleared by a correction). Empty when the transcript has no DRC report
    /// or the final report was clean.
    pub drc_rules: BTreeSet<String>,
    /// The geometric-pattern class of the run.
    pub pattern: PatternClass,
    /// The intent-violation kind the run ended with.
    pub intent: IntentViolationKind,
}

impl Signature {
    /// Extracts the signature of one run from its transcript.
    #[must_use]
    pub fn of(run: &MinedRun) -> Self {
        Self {
            drc_rules: persistent_rules(&drc_reports(&run.transcript)),
            pattern: pattern_class(&run.transcript),
            intent: intent_violation(&run.transcript),
        }
    }

    /// A human-readable one-line key, stable across runs, for example
    /// `drc=m1.1+li.3|pattern=rect_only|intent=none` (rules in sorted order,
    /// `none` when empty).
    #[must_use]
    pub fn key(&self) -> String {
        let rules = if self.drc_rules.is_empty() {
            "none".to_owned()
        } else {
            self.drc_rules.iter().cloned().collect::<Vec<_>>().join("+")
        };
        let pattern = self.pattern.as_str();
        let intent = self.intent.as_str();
        format!("drc={rules}|pattern={pattern}|intent={intent}")
    }

    /// A filesystem-safe slug (lowercase alphanumerics and underscores) built
    /// from the signature components; candidate ids are `cand_` plus this
    /// slug. The `none` intent kind is omitted, so a pure-DRC signature slugs
    /// to, for example, `m1_1_rect_only`.
    #[must_use]
    pub fn slug(&self) -> String {
        let mut parts: Vec<String> = self.drc_rules.iter().map(|rule| sanitize(rule)).collect();
        parts.push(self.pattern.as_str().to_owned());
        if self.intent != IntentViolationKind::None {
            parts.push(self.intent.as_str().to_owned());
        }
        parts.join("_")
    }
}

/// Lowercases `text` and replaces every non-alphanumeric character with an
/// underscore, making it safe as a file-stem fragment.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// A group of mined runs sharing one failure [`Signature`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cluster {
    /// The signature every member shares.
    pub signature: Signature,
    /// Indices into the scanned run slice, in input order.
    pub members: Vec<usize>,
}

/// Whether a run qualifies for mining: it failed its check, or it consumed at
/// least the high-iteration threshold even though it passed.
fn is_mined(run: &MinedRun, options: &MiningOptions) -> bool {
    !run.record.success || run.record.iterations >= options.high_iteration_threshold
}

/// Clusters the failed and high-iteration runs in `runs` by failure
/// [`Signature`] and returns the clusters holding at least
/// [`MiningOptions::min_cluster_size`] members, in signature order (so the
/// result is deterministic for a given input).
#[must_use]
pub fn scan(runs: &[MinedRun], options: &MiningOptions) -> Vec<Cluster> {
    let mut by_signature: BTreeMap<Signature, Vec<usize>> = BTreeMap::new();
    for (index, run) in runs.iter().enumerate() {
        if is_mined(run, options) {
            by_signature
                .entry(Signature::of(run))
                .or_default()
                .push(index);
        }
    }
    by_signature
        .into_iter()
        .filter(|(_, members)| members.len() >= options.min_cluster_size)
        .map(|(signature, members)| Cluster { signature, members })
        .collect()
}

/// Extracts every DRC report in the transcript as a set of rule ids.
///
/// A report is the data payload of a successful `run_drc` or `get_violations`
/// command: a JSON array whose elements carry a string `rule` field (the shape
/// the agent session emits). Payloads of any other shape are ignored.
fn drc_reports(transcript: &Transcript) -> Vec<BTreeSet<String>> {
    let mut reports = Vec::new();
    for record in &transcript.records {
        if !matches!(
            record.command,
            AgentCommand::RunDrc { .. } | AgentCommand::GetViolations
        ) {
            continue;
        }
        let Outcome::Ok(AgentResponse::Data { value, .. }) = &record.outcome else {
            continue;
        };
        let Some(items) = value.as_array() else {
            continue;
        };
        let mut rules = BTreeSet::new();
        for item in items {
            if let Some(rule) = item.get("rule").and_then(serde_json::Value::as_str) {
                rules.insert(rule.to_owned());
            }
        }
        reports.push(rules);
    }
    reports
}

/// The rule ids present in every report: the persistent violations no
/// correction attempt cleared. Empty when there are no reports at all.
fn persistent_rules(reports: &[BTreeSet<String>]) -> BTreeSet<String> {
    let mut iter = reports.iter();
    let Some(first) = iter.next() else {
        return BTreeSet::new();
    };
    iter.fold(first.clone(), |acc, report| {
        acc.intersection(report).cloned().collect()
    })
}

/// Classifies the transcript's drawing commands into a [`PatternClass`] using
/// the fixed priority documented on the enum.
fn pattern_class(transcript: &Transcript) -> PatternClass {
    let mut rect_layers: BTreeSet<(u16, u16)> = BTreeSet::new();
    let mut any_polygon = false;
    let mut any_path = false;
    let mut any_placement = false;
    for record in &transcript.records {
        match &record.command {
            AgentCommand::AddRect { layer, .. } => {
                rect_layers.insert((layer.layer, layer.datatype));
            }
            AgentCommand::AddPolygon { .. } => any_polygon = true,
            AgentCommand::AddPath { .. } => any_path = true,
            AgentCommand::PlaceInstance { .. } | AgentCommand::PlaceArray { .. } => {
                any_placement = true;
            }
            _ => {}
        }
    }
    if any_placement {
        PatternClass::Hierarchical
    } else if any_path {
        PatternClass::PathRouting
    } else if any_polygon {
        PatternClass::PolygonHeavy
    } else if rect_layers.len() >= 3 {
        PatternClass::LayerStack
    } else if rect_layers.is_empty() {
        PatternClass::NoGeometry
    } else {
        PatternClass::RectOnly
    }
}

/// Reads the intent-violation kind from the *last* successful `check_intent`
/// report in the transcript (the run's final connectivity state); earlier
/// reports are superseded by later corrections.
fn intent_violation(transcript: &Transcript) -> IntentViolationKind {
    let mut kind = IntentViolationKind::None;
    for record in &transcript.records {
        if !matches!(record.command, AgentCommand::CheckIntent { .. }) {
            continue;
        }
        let Outcome::Ok(AgentResponse::Data { value, .. }) = &record.outcome else {
            continue;
        };
        let opens = value
            .get("opens")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|list| !list.is_empty());
        let shorts = value
            .get("shorts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|list| !list.is_empty());
        kind = match (opens, shorts) {
            (true, true) => IntentViolationKind::OpenAndShort,
            (true, false) => IntentViolationKind::Open,
            (false, true) => IntentViolationKind::Short,
            (false, false) => IntentViolationKind::None,
        };
    }
    kind
}

#[cfg(test)]
mod tests {
    use super::{
        Cluster, IntentViolationKind, MinedRun, MiningOptions, PatternClass, Signature, scan,
    };
    use crate::{ResultRecord, Tier};
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
    use reticle_agent_api::{
        AgentCommand, AgentResponse, CommandRecord, ElementId, Outcome, Transcript,
    };

    /// A transcript record with deterministic metadata around `command`.
    fn cmd(seq: u64, command: AgentCommand, outcome: Outcome) -> CommandRecord {
        CommandRecord {
            seq,
            command,
            revision_before: seq,
            revision_after: seq + 1,
            outcome,
            ts_start_ms: seq,
            ts_end_ms: seq + 1,
            tokens_in: None,
            tokens_out: None,
        }
    }

    /// A successful mutation outcome affecting the given element ids.
    fn ok_mutation(ids: &[u64]) -> Outcome {
        Outcome::Ok(AgentResponse::Ok {
            revision: 1,
            affected: ids.iter().copied().map(ElementId).collect(),
        })
    }

    /// A data outcome wrapping `value`.
    fn data(value: serde_json::Value) -> Outcome {
        Outcome::Ok(AgentResponse::Data { revision: 1, value })
    }

    /// A `run_drc` record whose report names the given rule ids.
    fn drc_report(seq: u64, rules: &[&str]) -> CommandRecord {
        let items: Vec<serde_json::Value> = rules
            .iter()
            .map(|rule| serde_json::json!({ "rule": rule, "message": "violated" }))
            .collect();
        cmd(
            seq,
            AgentCommand::RunDrc {
                cell: "top".into(),
                region: None,
            },
            data(serde_json::Value::Array(items)),
        )
    }

    /// An `add_rect` command on `layer` from `min` to `max`.
    fn add_rect(layer: (u16, u16), min: (i32, i32), max: (i32, i32)) -> AgentCommand {
        AgentCommand::AddRect {
            cell: "top".into(),
            layer: LayerArg {
                layer: layer.0,
                datatype: layer.1,
            },
            rect: RectArg {
                min: PointArg { x: min.0, y: min.1 },
                max: PointArg { x: max.0, y: max.1 },
            },
        }
    }

    /// A `check_intent` record reporting `opens` opens and `shorts` shorts.
    fn intent_report(seq: u64, opens: usize, shorts: usize) -> CommandRecord {
        let opens: Vec<serde_json::Value> = (0..opens)
            .map(|i| serde_json::json!({ "net": format!("n{i}"), "detail": "open" }))
            .collect();
        let shorts: Vec<serde_json::Value> = (0..shorts)
            .map(|i| serde_json::json!({ "net_a": format!("a{i}"), "net_b": format!("b{i}") }))
            .collect();
        cmd(
            seq,
            AgentCommand::CheckIntent {
                cell: "top".into(),
                intent: "{}".into(),
            },
            data(serde_json::json!({ "opens": opens, "shorts": shorts })),
        )
    }

    /// A mined-run fixture around `records`.
    fn run_of(
        task_id: &str,
        tier: u8,
        success: bool,
        iterations: u32,
        records: Vec<CommandRecord>,
    ) -> MinedRun {
        MinedRun {
            tier: Tier(tier),
            record: ResultRecord {
                task_id: task_id.into(),
                model: "mock".into(),
                suite_version: "0.2.0".into(),
                success,
                iterations,
                first_proposal_violations: 2,
                final_violations: u32::from(!success),
                wall_ms: 5,
            },
            transcript: Transcript {
                records,
                final_hash: 0,
            },
        }
    }

    /// A failed rect-drawing run whose DRC reports never clear `rule`.
    fn failed_rect_run(task_id: &str, tier: u8, rule: &str) -> MinedRun {
        run_of(
            task_id,
            tier,
            false,
            4,
            vec![
                cmd(
                    0,
                    AgentCommand::CreateCell { name: "top".into() },
                    ok_mutation(&[]),
                ),
                cmd(1, add_rect((68, 20), (0, 0), (100, 100)), ok_mutation(&[1])),
                drc_report(2, &[rule]),
                cmd(
                    3,
                    add_rect((68, 20), (0, 200), (100, 300)),
                    ok_mutation(&[2]),
                ),
                drc_report(4, &[rule]),
            ],
        )
    }

    #[test]
    fn scan_clusters_failed_runs_by_signature() {
        let runs = vec![
            failed_rect_run("t1_min_width_met1", 1, "m1.1"),
            // A clean, fast pass: never mined.
            run_of(
                "t1_place_met1_rect",
                1,
                true,
                1,
                vec![cmd(
                    0,
                    add_rect((68, 20), (0, 0), (500, 500)),
                    ok_mutation(&[1]),
                )],
            ),
            failed_rect_run("t2_legal_spacing_met1", 2, "m1.1"),
            // A different persistent rule forms a different cluster.
            failed_rect_run("t1_min_area_li1", 1, "li.3"),
            failed_rect_run("t3_li1_pair", 3, "li.3"),
        ];
        let clusters = scan(&runs, &MiningOptions::default());
        assert_eq!(clusters.len(), 2, "two signatures with two members each");

        // BTreeMap ordering: "li.3" sorts before "m1.1".
        assert_eq!(
            clusters[0].signature.key(),
            "drc=li.3|pattern=rect_only|intent=none"
        );
        assert_eq!(clusters[0].members, vec![3, 4]);
        assert_eq!(
            clusters[1].signature.key(),
            "drc=m1.1|pattern=rect_only|intent=none"
        );
        assert_eq!(clusters[1].members, vec![0, 2]);
    }

    #[test]
    fn high_iteration_pass_is_mined() {
        let mut run = run_of(
            "t3_slow",
            3,
            true,
            4,
            vec![cmd(
                0,
                add_rect((68, 20), (0, 0), (500, 500)),
                ok_mutation(&[1]),
            )],
        );
        run.record.final_violations = 0;
        let options = MiningOptions {
            min_cluster_size: 1,
            ..MiningOptions::default()
        };
        let clusters = scan(std::slice::from_ref(&run), &options);
        assert_eq!(clusters.len(), 1, "a struggling pass is still mined");
        assert!(clusters[0].signature.drc_rules.is_empty());
        assert_eq!(clusters[0].signature.pattern, PatternClass::RectOnly);
    }

    #[test]
    fn small_clusters_are_dropped() {
        let runs = vec![failed_rect_run("t1_min_width_met1", 1, "m1.1")];
        let clusters = scan(&runs, &MiningOptions::default());
        assert!(
            clusters.is_empty(),
            "a lone failure is noise under min_cluster_size = 2"
        );
    }

    #[test]
    fn persistent_rules_are_the_report_intersection() {
        let run = run_of(
            "t2_two_rules",
            2,
            false,
            2,
            vec![
                drc_report(0, &["m1.1", "li.3"]),
                // The correction cleared li.3 but not m1.1.
                drc_report(1, &["m1.1"]),
            ],
        );
        let signature = Signature::of(&run);
        let rules: Vec<&str> = signature.drc_rules.iter().map(String::as_str).collect();
        assert_eq!(rules, vec!["m1.1"]);
    }

    #[test]
    fn cleared_final_report_leaves_no_persistent_rules() {
        let run = run_of(
            "t2_recovered",
            2,
            true,
            4,
            vec![drc_report(0, &["m1.1"]), drc_report(1, &[])],
        );
        assert!(Signature::of(&run).drc_rules.is_empty());
    }

    #[test]
    fn pattern_class_uses_documented_priority() {
        let path = AgentCommand::AddPath {
            cell: "top".into(),
            layer: LayerArg {
                layer: 68,
                datatype: 20,
            },
            width: 140,
            points: vec![PointArg { x: 0, y: 0 }, PointArg { x: 500, y: 0 }],
            endcap: None,
        };
        // A path outranks rectangles on many layers.
        let run = run_of(
            "t3_path",
            3,
            false,
            1,
            vec![
                cmd(0, add_rect((68, 20), (0, 0), (10, 10)), ok_mutation(&[1])),
                cmd(1, add_rect((69, 20), (0, 0), (10, 10)), ok_mutation(&[2])),
                cmd(2, add_rect((70, 20), (0, 0), (10, 10)), ok_mutation(&[3])),
                cmd(3, path, ok_mutation(&[4])),
            ],
        );
        assert_eq!(Signature::of(&run).pattern, PatternClass::PathRouting);

        // Rectangles on three distinct layers form a stack.
        let run = run_of(
            "t3_stack",
            3,
            false,
            1,
            vec![
                cmd(0, add_rect((67, 20), (0, 0), (10, 10)), ok_mutation(&[1])),
                cmd(1, add_rect((68, 20), (0, 0), (10, 10)), ok_mutation(&[2])),
                cmd(2, add_rect((69, 20), (0, 0), (10, 10)), ok_mutation(&[3])),
            ],
        );
        assert_eq!(Signature::of(&run).pattern, PatternClass::LayerStack);

        // No drawing command at all.
        let run = run_of(
            "t1_silent",
            1,
            false,
            1,
            vec![cmd(
                0,
                AgentCommand::CreateCell { name: "top".into() },
                ok_mutation(&[]),
            )],
        );
        assert_eq!(Signature::of(&run).pattern, PatternClass::NoGeometry);
    }

    #[test]
    fn intent_kind_comes_from_the_last_report() {
        let run = run_of(
            "t3_intent",
            3,
            false,
            2,
            vec![intent_report(0, 1, 1), intent_report(1, 1, 0)],
        );
        let signature = Signature::of(&run);
        assert_eq!(signature.intent, IntentViolationKind::Open);

        let run = run_of("t3_intent", 3, false, 1, vec![intent_report(0, 0, 2)]);
        assert_eq!(Signature::of(&run).intent, IntentViolationKind::Short);

        let run = run_of("t3_intent", 3, false, 1, vec![intent_report(0, 2, 1)]);
        assert_eq!(
            Signature::of(&run).intent,
            IntentViolationKind::OpenAndShort
        );
    }

    #[test]
    fn slug_is_filesystem_safe_and_deterministic() {
        let run = failed_rect_run("t1_min_width_met1", 1, "m1.1");
        let signature = Signature::of(&run);
        assert_eq!(signature.slug(), "m1_1_rect_only");

        let clusters = scan(
            &[run],
            &MiningOptions {
                min_cluster_size: 1,
                ..MiningOptions::default()
            },
        );
        let Cluster { signature, .. } = &clusters[0];
        assert!(
            signature
                .slug()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
    }
}
