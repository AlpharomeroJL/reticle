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
//!
//! [`draft_candidates`] turns each cluster into a [`CandidateFile`]: a full
//! runnable task (id `cand_` plus the signature slug, a drafted prompt, and a
//! checker chosen from the signature) together with full provenance (the
//! signature and every source run) and a two-way test vector pair: a `good`
//! document the checker must accept and a `bad` document, reconstructed from a
//! representative failing transcript, that it must reject. [`write_candidates`]
//! writes one TOML per candidate under a suite's `candidates/` directory;
//! drafts are never added to the live manifest by this module.
//!
//! [`promote_candidate`] is the gate behind `just bench-promote <id>`: it
//! recompiles the candidate's checker, runs the two-way vectors
//! ([`verify_two_way`]), refuses a double promotion, and only then writes the
//! plain task file into the suite and rewrites the manifest with the id
//! appended and the minor version bumped. The manifest edit is surgical (it
//! preserves comments and formatting) and the edited text is reparsed before
//! anything is written, so a bad edit refuses instead of corrupting the suite.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reticle_agent_api::{AgentCommand, AgentResponse, Outcome, Transcript};
use reticle_extract::IntentSpec;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};
use serde::{Deserialize, Serialize};

use crate::loader::MANIFEST_FILE;
use crate::{BenchTask, CheckResult, CheckerRegistry, ResultRecord, SuiteManifest, Tier};

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

/// One rectangle of a two-way test vector: a layer written `layer/datatype`
/// (for example `68/20`) and the corners as `[min_x, min_y, max_x, max_y]` in
/// database units.
///
/// Vectors carry rectangles only; a transcript's paths and polygons are not
/// reconstructed. That keeps candidate files small and the promotion check
/// simple, at the cost of dropping non-rectangle evidence from the bad vector.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct VectorRect {
    /// The layer, written `layer/datatype`.
    pub layer: String,
    /// The rectangle corners: `[min_x, min_y, max_x, max_y]`.
    pub rect: [i32; 4],
}

/// The two-way test vectors a candidate must pass to be promoted: the
/// compiled checker has to accept the `good` document and reject the `bad`
/// one.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct TwoWay {
    /// Rectangles of a document the checker must accept.
    #[serde(default)]
    pub good: Vec<VectorRect>,
    /// Rectangles of a document the checker must reject (reconstructed from a
    /// representative failing transcript; empty when the failing run drew no
    /// rectangles).
    #[serde(default)]
    pub bad: Vec<VectorRect>,
}

/// One source run in a candidate's provenance: the tier plus every
/// [`ResultRecord`] field, flattened so the TOML stays a plain table.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SourceRun {
    /// The tier the source task ran at.
    pub tier: u8,
    /// The task that was run.
    pub task_id: String,
    /// The model identifier (or `mock`).
    pub model: String,
    /// The suite version the task came from.
    pub suite_version: String,
    /// Whether the checker passed.
    pub success: bool,
    /// How many propose-verify-correct iterations were used.
    pub iterations: u32,
    /// DRC violations in the model's first proposal.
    pub first_proposal_violations: u32,
    /// DRC violations in the final document.
    pub final_violations: u32,
    /// Wall-clock time for the whole task, in milliseconds.
    pub wall_ms: u64,
}

impl SourceRun {
    /// The provenance row for one mined run.
    fn of(run: &MinedRun) -> Self {
        Self {
            tier: run.tier.0,
            task_id: run.record.task_id.clone(),
            model: run.record.model.clone(),
            suite_version: run.record.suite_version.clone(),
            success: run.record.success,
            iterations: run.record.iterations,
            first_proposal_violations: run.record.first_proposal_violations,
            final_violations: run.record.final_violations,
            wall_ms: run.record.wall_ms,
        }
    }
}

/// The full provenance of a drafted candidate: where it came from and why.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Provenance {
    /// The one-line signature key ([`Signature::key`]) of the cluster.
    pub signature: String,
    /// The persistent DRC rule ids, in sorted order.
    pub drc_rules: Vec<String>,
    /// The geometric-pattern class token ([`PatternClass::as_str`]).
    pub pattern: String,
    /// The intent-violation token ([`IntentViolationKind::as_str`]).
    pub intent_violation: String,
    /// Every source run in the cluster, in scan input order.
    pub source_runs: Vec<SourceRun>,
}

/// A drafted candidate task: the runnable task fields (a superset of a
/// [`BenchTask`] file) plus provenance and the two-way promotion vectors.
///
/// Serialized as one TOML file per candidate under a suite's `candidates/`
/// directory; [`CandidateFile::task`] projects out the plain task for the
/// checker registry and for promotion.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CandidateFile {
    /// Candidate id (also the file stem), `cand_` plus the signature slug.
    pub id: String,
    /// Draft difficulty tier: the highest tier among the source runs.
    pub tier: Tier,
    /// The drafted natural-language prompt.
    pub prompt: String,
    /// Path, relative to the suite root, of the technology file.
    pub technology: String,
    /// The checker the drafted task would run under.
    pub checker: String,
    /// Serialized connectivity intent spec, for intent-checked candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Where the candidate came from.
    pub provenance: Provenance,
    /// The vectors the promotion gate verifies.
    pub two_way: TwoWay,
}

impl CandidateFile {
    /// The plain [`BenchTask`] this candidate would become when promoted.
    #[must_use]
    pub fn task(&self) -> BenchTask {
        BenchTask {
            id: self.id.clone(),
            tier: self.tier,
            prompt: self.prompt.clone(),
            technology: self.technology.clone(),
            checker: self.checker.clone(),
            intent: self.intent.clone(),
        }
    }
}

/// Drafts one [`CandidateFile`] per cluster.
///
/// `clusters` must come from [`scan`] over the same `runs` slice (members are
/// indices into it). Per cluster: the id is `cand_` plus the signature slug,
/// the tier is the highest source tier, the prompt is templated from the
/// signature and the source task ids, and the checker is chosen from the
/// signature:
///
/// - persistent DRC rules draft `drc_clean` (the promoted task asks for the
///   same geometry class, clean);
/// - an intent violation drafts `intent`, reusing the spec of the last
///   `check_intent` command in the representative transcript;
/// - a no-geometry pattern drafts `rect_present` (the model must draw at
///   all);
/// - anything else (high-iteration passes) drafts `drc_clean`.
///
/// The bad vector replays the representative (first) member's rectangles; the
/// good vector is the canonical clean met1 rectangle, or per-net terminal
/// covers for intent candidates (empty when the spec spans layers, leaving
/// the draft unpromotable until edited by hand).
#[must_use]
pub fn draft_candidates(
    runs: &[MinedRun],
    clusters: &[Cluster],
    options: &MiningOptions,
) -> Vec<CandidateFile> {
    let mut candidates = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let members: Vec<&MinedRun> = cluster
            .members
            .iter()
            .filter_map(|&index| runs.get(index))
            .collect();
        let Some(representative) = members.first() else {
            continue;
        };
        let signature = &cluster.signature;
        let sources: BTreeSet<String> = members
            .iter()
            .map(|run| run.record.task_id.clone())
            .collect();
        let tier = members.iter().map(|run| run.tier.0).max().unwrap_or(1);
        let (checker, intent, good) = draft_checker(signature, representative);
        let bad = replay_rects(&representative.transcript);
        candidates.push(CandidateFile {
            id: format!("cand_{}", signature.slug()),
            tier: Tier(tier),
            prompt: draft_prompt(signature, &sources),
            technology: options.technology.clone(),
            checker,
            intent,
            provenance: Provenance {
                signature: signature.key(),
                drc_rules: signature.drc_rules.iter().cloned().collect(),
                pattern: signature.pattern.as_str().to_owned(),
                intent_violation: signature.intent.as_str().to_owned(),
                source_runs: members.iter().map(|run| SourceRun::of(run)).collect(),
            },
            two_way: TwoWay { good, bad },
        });
    }
    candidates
}

/// The checker name, optional intent spec, and good vector for a signature
/// (the drafting rules documented on [`draft_candidates`]).
fn draft_checker(
    signature: &Signature,
    representative: &MinedRun,
) -> (String, Option<String>, Vec<VectorRect>) {
    if !signature.drc_rules.is_empty() {
        return ("drc_clean".to_owned(), None, canonical_clean());
    }
    if signature.intent != IntentViolationKind::None {
        let source = intent_source(&representative.transcript);
        let good = source.as_deref().map(intent_good_rects).unwrap_or_default();
        return ("intent".to_owned(), source, good);
    }
    if signature.pattern == PatternClass::NoGeometry {
        return ("rect_present".to_owned(), None, canonical_clean());
    }
    ("drc_clean".to_owned(), None, canonical_clean())
}

/// The canonical known-good vector: a 500x500 met1 rectangle at the origin,
/// which is clean under the built-in SKY130 rule subset and satisfies the
/// default `rect_present` layer.
fn canonical_clean() -> Vec<VectorRect> {
    vec![VectorRect {
        layer: layer_token(68, 20),
        rect: [0, 0, 500, 500],
    }]
}

/// A layer written in the `layer/datatype` vector form.
fn layer_token(layer: u16, datatype: u16) -> String {
    format!("{layer}/{datatype}")
}

/// The drafted prompt for a cluster: templated from the signature and the
/// sorted source task ids, so a given cluster always drafts the same prompt.
fn draft_prompt(signature: &Signature, sources: &BTreeSet<String>) -> String {
    let sources_list = sources.iter().cloned().collect::<Vec<_>>().join(", ");
    let pattern = signature.pattern.as_str();
    if !signature.drc_rules.is_empty() {
        let rules = signature
            .drc_rules
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return format!(
            "Create a cell named top and rebuild the {pattern} geometry that mined runs of \
             {sources_list} never got clean: rule(s) {rules} stayed violated through every \
             correction attempt. Draw the same class of geometry so the final layout passes DRC."
        );
    }
    if signature.intent != IntentViolationKind::None {
        let kind = signature.intent.as_str();
        return format!(
            "Create a cell named top and rebuild the {pattern} geometry that mined runs of \
             {sources_list} left with an unsatisfied connectivity intent ({kind} in the final \
             check). Connect every terminal of the intent spec so the check reports no opens \
             and no shorts."
        );
    }
    if signature.pattern == PatternClass::NoGeometry {
        return format!(
            "Mined runs of {sources_list} produced no geometry at all. Create a cell named top \
             and place at least one met1 rectangle (layer 68/20) large enough to satisfy the \
             checker."
        );
    }
    format!(
        "Create a cell named top and redo the {pattern} geometry that mined runs of \
         {sources_list} only completed after several corrections. Produce the same class of \
         geometry with the final layout DRC-clean."
    )
}

/// Reconstructs the rectangles standing at the end of a transcript: every
/// successfully applied `add_rect`, minus those removed by a successful
/// `delete_shapes` that names their recorded element ids. Paths, polygons,
/// and bulk imports are not reconstructed.
fn replay_rects(transcript: &Transcript) -> Vec<VectorRect> {
    let mut shapes: Vec<(Option<u64>, VectorRect)> = Vec::new();
    for record in &transcript.records {
        match &record.command {
            AgentCommand::AddRect { layer, rect, .. } => {
                let Outcome::Ok(AgentResponse::Ok { affected, .. }) = &record.outcome else {
                    continue;
                };
                let id = affected.first().map(|element| element.0);
                shapes.push((
                    id,
                    VectorRect {
                        layer: layer_token(layer.layer, layer.datatype),
                        rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                    },
                ));
            }
            AgentCommand::DeleteShapes { ids } => {
                if matches!(record.outcome, Outcome::Ok(_)) {
                    let dead: BTreeSet<u64> = ids.iter().map(|element| element.0).collect();
                    shapes.retain(|(id, _)| id.is_none_or(|held| !dead.contains(&held)));
                }
            }
            _ => {}
        }
    }
    shapes.into_iter().map(|(_, rect)| rect).collect()
}

/// The intent spec of the *last* `check_intent` command in the transcript,
/// verbatim, if any.
fn intent_source(transcript: &Transcript) -> Option<String> {
    let mut source = None;
    for record in &transcript.records {
        if let AgentCommand::CheckIntent { intent, .. } = &record.command {
            source = Some(intent.clone());
        }
    }
    source
}

/// A generic known-good vector for an intent spec: one rectangle per net
/// covering the union bounding box of that net's terminals, which joins them
/// when they all sit on one layer. Returns an empty vector (leaving the
/// candidate unpromotable) when the spec does not parse or any net spans
/// layers, since a single-layer cover cannot connect those.
fn intent_good_rects(intent_json: &str) -> Vec<VectorRect> {
    let Ok(spec) = serde_json::from_str::<IntentSpec>(intent_json) else {
        return Vec::new();
    };
    let mut rects = Vec::new();
    for net in &spec.nets {
        let Some(first) = net.terminals.first() else {
            continue;
        };
        let layer = first.layer;
        if net.terminals.iter().any(|terminal| terminal.layer != layer) {
            return Vec::new();
        }
        let mut bounds = first.region;
        for terminal in &net.terminals {
            bounds.min.x = bounds.min.x.min(terminal.region.min.x);
            bounds.min.y = bounds.min.y.min(terminal.region.min.y);
            bounds.max.x = bounds.max.x.max(terminal.region.max.x);
            bounds.max.y = bounds.max.y.max(terminal.region.max.y);
        }
        rects.push(VectorRect {
            layer: layer_token(layer.layer, layer.datatype),
            rect: [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y],
        });
    }
    rects
}

/// A failure drafting or writing candidate files.
#[derive(Debug)]
pub enum MiningError {
    /// A directory could not be created or a file could not be written.
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A candidate could not be serialized to TOML.
    Serialize {
        /// The candidate id that failed.
        id: String,
        /// The serializer's message.
        message: String,
    },
}

impl std::fmt::Display for MiningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MiningError::Io { path, source } => write!(f, "writing {}: {source}", path.display()),
            MiningError::Serialize { id, message } => {
                write!(f, "serializing candidate `{id}`: {message}")
            }
        }
    }
}

impl std::error::Error for MiningError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MiningError::Io { source, .. } => Some(source),
            MiningError::Serialize { .. } => None,
        }
    }
}

/// The comment header prepended to every written candidate file.
const CANDIDATE_HEADER: &str = "# Mined candidate task (draft). Not part of the live suite until \
                                promoted with\n# `just bench-promote <id>`; promotion requires \
                                the two-way vectors below to pass\n# (good accepted, bad \
                                rejected) and bumps the suite manifest version.\n\n";

/// Writes one `<id>.toml` per candidate into `dir` (typically a suite's
/// `candidates/` directory), creating it if needed, and returns the written
/// paths in input order. The live suite manifest is never touched.
///
/// # Errors
///
/// Returns a [`MiningError`] if the directory cannot be created, a candidate
/// cannot be serialized, or a file cannot be written.
pub fn write_candidates(
    dir: &Path,
    candidates: &[CandidateFile],
) -> Result<Vec<PathBuf>, MiningError> {
    std::fs::create_dir_all(dir).map_err(|source| MiningError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let body = toml::to_string(candidate).map_err(|e| MiningError::Serialize {
            id: candidate.id.clone(),
            message: e.to_string(),
        })?;
        let path = dir.join(format!("{}.toml", candidate.id));
        std::fs::write(&path, format!("{CANDIDATE_HEADER}{body}")).map_err(|source| {
            MiningError::Io {
                path: path.clone(),
                source,
            }
        })?;
        paths.push(path);
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Promotion: candidates enter the live suite only through the two-way gate.
// ---------------------------------------------------------------------------

/// Why a candidate could not be promoted.
///
/// The refusal variants ([`is_refusal`](PromoteError::is_refusal)) mean the
/// candidate is not fit for the suite; the others are setup or IO failures
/// around the gate.
#[derive(Debug)]
pub enum PromoteError {
    /// The candidate file could not be read or parsed, its declared id did
    /// not match, or a vector was malformed.
    Candidate {
        /// What went wrong.
        detail: String,
    },
    /// Refusal: the candidate's checker did not compile or is unknown.
    Checker {
        /// The compile error.
        detail: String,
    },
    /// Refusal: the checker rejected the good vector, so the drafted task is
    /// not known to be satisfiable.
    GoodVectorRejected {
        /// The checker's failure reasons.
        detail: String,
    },
    /// Refusal: the checker accepted the bad vector, so it does not actually
    /// capture the mined failure mode.
    BadVectorAccepted,
    /// Refusal: the id is already in the manifest, or a task file with that
    /// name already exists in the suite.
    AlreadyInSuite {
        /// The offending id.
        id: String,
    },
    /// The suite manifest could not be read, edited, or reparsed.
    Manifest {
        /// What went wrong.
        detail: String,
    },
    /// A promoted file could not be written.
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl PromoteError {
    /// True when the gate refused the candidate itself (checker missing,
    /// two-way vectors failing, or double promotion), as opposed to a setup
    /// or IO failure around the gate.
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            PromoteError::Checker { .. }
                | PromoteError::GoodVectorRejected { .. }
                | PromoteError::BadVectorAccepted
                | PromoteError::AlreadyInSuite { .. }
        )
    }
}

impl std::fmt::Display for PromoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromoteError::Candidate { detail } => write!(f, "candidate: {detail}"),
            PromoteError::Checker { detail } => {
                write!(f, "the candidate's checker did not compile: {detail}")
            }
            PromoteError::GoodVectorRejected { detail } => {
                write!(
                    f,
                    "two-way gate: the checker rejected the good vector: {detail}"
                )
            }
            PromoteError::BadVectorAccepted => write!(
                f,
                "two-way gate: the checker accepted the bad vector, so it does not capture \
                 the mined failure"
            ),
            PromoteError::AlreadyInSuite { id } => {
                write!(f, "task `{id}` is already in the suite")
            }
            PromoteError::Manifest { detail } => write!(f, "manifest: {detail}"),
            PromoteError::Io { path, source } => {
                write!(f, "writing {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PromoteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PromoteError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A [`PromoteError::Manifest`] with the given detail.
fn manifest_err(detail: impl Into<String>) -> PromoteError {
    PromoteError::Manifest {
        detail: detail.into(),
    }
}

/// A successful promotion: what was written where.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Promotion {
    /// The promoted task id.
    pub id: String,
    /// The bumped suite version now in the manifest.
    pub new_version: String,
    /// The task file written into the suite directory.
    pub task_path: PathBuf,
    /// The manifest file that was rewritten.
    pub manifest_path: PathBuf,
}

/// Runs a candidate's two-way vectors against its own compiled checker: the
/// good document must pass and the bad document must fail.
///
/// This is the promotion gate's core. It compiles the checker exactly the way
/// the runner would ([`CheckerRegistry::for_task`]), so a candidate that
/// passes here runs unchanged in the suite.
///
/// # Errors
///
/// Returns [`PromoteError::Checker`] when the checker does not compile,
/// [`PromoteError::Candidate`] when a vector rectangle is malformed,
/// [`PromoteError::GoodVectorRejected`] when the good document fails the
/// check, or [`PromoteError::BadVectorAccepted`] when the bad document passes.
pub fn verify_two_way(candidate: &CandidateFile) -> Result<(), PromoteError> {
    let task = candidate.task();
    let registry =
        CheckerRegistry::for_task(&task).map_err(|detail| PromoteError::Checker { detail })?;
    let checker = registry
        .get(&task.checker)
        .ok_or_else(|| PromoteError::Checker {
            detail: format!("no checker named `{}`", task.checker),
        })?;
    let transcript = Transcript::default();

    let good = document_from(&candidate.two_way.good)?;
    if let CheckResult::Fail(failures) = checker.check(&good, &transcript) {
        let reasons: Vec<String> = failures.into_iter().map(|failure| failure.reason).collect();
        return Err(PromoteError::GoodVectorRejected {
            detail: reasons.join("; "),
        });
    }

    let bad = document_from(&candidate.two_way.bad)?;
    if checker.check(&bad, &transcript).is_pass() {
        return Err(PromoteError::BadVectorAccepted);
    }
    Ok(())
}

/// Materializes a vector's rectangles into a one-cell document named `top`,
/// the shape every built-in checker inspects.
fn document_from(rects: &[VectorRect]) -> Result<Document, PromoteError> {
    let mut cell = Cell::new("top");
    for shape in rects {
        let layer = parse_layer_token(&shape.layer).ok_or_else(|| PromoteError::Candidate {
            detail: format!(
                "malformed vector layer `{}` (expected `layer/datatype`)",
                shape.layer
            ),
        })?;
        let [min_x, min_y, max_x, max_y] = shape.rect;
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(
                Point::new(min_x, min_y),
                Point::new(max_x, max_y),
            )),
        ));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    Ok(doc)
}

/// Parses the `layer/datatype` vector form back into a [`LayerId`].
fn parse_layer_token(token: &str) -> Option<LayerId> {
    let (layer, datatype) = token.split_once('/')?;
    Some(LayerId::new(
        layer.trim().parse().ok()?,
        datatype.trim().parse().ok()?,
    ))
}

/// Promotes candidate `id` from `candidates_dir` into the live suite at
/// `suite_dir`, but only if the candidate passes [`verify_two_way`] and is
/// not already in the suite.
///
/// On success the plain task file `<id>.toml` is written into the suite
/// directory (the candidate file stays behind as the provenance record), the
/// manifest gains the id at the end of its task list, and the manifest's
/// minor version is bumped (`0.2.0` becomes `0.3.0`). The task file is
/// re-loaded through [`crate::load_task`] before the manifest is touched, so
/// a failure at any step leaves the suite as it was.
///
/// # Errors
///
/// Returns the [`verify_two_way`] refusals, [`PromoteError::AlreadyInSuite`]
/// on a double promotion, [`PromoteError::Candidate`] when the candidate file
/// is unreadable or inconsistent, [`PromoteError::Manifest`] when the
/// manifest cannot be read or safely edited, or [`PromoteError::Io`] when a
/// file cannot be written.
pub fn promote_candidate(
    suite_dir: &Path,
    candidates_dir: &Path,
    id: &str,
) -> Result<Promotion, PromoteError> {
    let candidate = load_candidate(candidates_dir, id)?;
    verify_two_way(&candidate)?;

    // Refuse a double promotion before any write.
    let manifest_path = suite_dir.join(MANIFEST_FILE);
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| manifest_err(format!("reading {}: {e}", manifest_path.display())))?;
    let manifest: SuiteManifest = toml::from_str(&manifest_text)
        .map_err(|e| manifest_err(format!("parsing {}: {e}", manifest_path.display())))?;
    if manifest.tasks.iter().any(|task| task == id) {
        return Err(PromoteError::AlreadyInSuite { id: id.to_owned() });
    }
    let task_path = suite_dir.join(format!("{id}.toml"));
    if task_path.exists() {
        return Err(PromoteError::AlreadyInSuite { id: id.to_owned() });
    }

    let (new_manifest_text, new_version) = edited_manifest(&manifest_text, id)?;

    // Write the task file first and prove it loads; only then commit the
    // manifest, so a failure part-way leaves the live suite untouched.
    let body = toml::to_string(&candidate.task()).map_err(|e| PromoteError::Candidate {
        detail: format!("serializing task `{id}`: {e}"),
    })?;
    let header = format!(
        "# Promoted from mined candidate `{id}`; see candidates/{id}.toml for provenance.\n"
    );
    std::fs::write(&task_path, format!("{header}{body}")).map_err(|source| PromoteError::Io {
        path: task_path.clone(),
        source,
    })?;
    if let Err(e) = crate::load_task(&task_path) {
        let _ = std::fs::remove_file(&task_path);
        return Err(PromoteError::Candidate {
            detail: format!("promoted task failed to load back: {e}"),
        });
    }
    if let Err(source) = std::fs::write(&manifest_path, new_manifest_text) {
        let _ = std::fs::remove_file(&task_path);
        return Err(PromoteError::Io {
            path: manifest_path.clone(),
            source,
        });
    }
    Ok(Promotion {
        id: id.to_owned(),
        new_version,
        task_path,
        manifest_path,
    })
}

/// Reads and validates `candidates_dir/<id>.toml`: it must parse as a
/// [`CandidateFile`] and declare the same id as its file stem.
fn load_candidate(candidates_dir: &Path, id: &str) -> Result<CandidateFile, PromoteError> {
    let path = candidates_dir.join(format!("{id}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|e| PromoteError::Candidate {
        detail: format!("reading {}: {e}", path.display()),
    })?;
    let candidate: CandidateFile = toml::from_str(&text).map_err(|e| PromoteError::Candidate {
        detail: format!("parsing {}: {e}", path.display()),
    })?;
    if candidate.id == id {
        Ok(candidate)
    } else {
        Err(PromoteError::Candidate {
            detail: format!(
                "candidate file {} declares id `{}`, expected `{id}`",
                path.display(),
                candidate.id
            ),
        })
    }
}

/// Produces the promoted manifest text and the new version: the version
/// line's minor component is bumped and `id` is appended to the `tasks`
/// array, all by surgical string edits that preserve comments and formatting.
/// The edited text is reparsed and checked before it is returned.
fn edited_manifest(text: &str, id: &str) -> Result<(String, String), PromoteError> {
    let (bumped, new_version) = bump_version_line(text)?;
    let with_task = insert_task_entry(&bumped, id)?;
    let manifest: SuiteManifest = toml::from_str(&with_task)
        .map_err(|e| manifest_err(format!("edited manifest no longer parses: {e}")))?;
    if manifest.version != new_version || !manifest.tasks.iter().any(|task| task == id) {
        return Err(manifest_err(
            "edited manifest did not carry the expected changes",
        ));
    }
    Ok((with_task, new_version))
}

/// Rewrites the first `version = "..."` line with the minor component bumped
/// and the patch reset, returning the new text and the new version.
fn bump_version_line(text: &str) -> Result<(String, String), PromoteError> {
    let mut out = String::with_capacity(text.len() + 8);
    let mut new_version: Option<String> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if new_version.is_none()
            && let Some(rest) = trimmed.strip_prefix("version")
            && rest.trim_start().starts_with('=')
        {
            let Some(open) = line.find('"') else {
                return Err(manifest_err("version line has no opening quote"));
            };
            let value_start = open + 1;
            let Some(len) = line[value_start..].find('"') else {
                return Err(manifest_err("version line has no closing quote"));
            };
            let old = &line[value_start..value_start + len];
            let bumped = bump_minor(old)
                .ok_or_else(|| manifest_err(format!("version `{old}` is not MAJOR.MINOR.PATCH")))?;
            out.push_str(&line[..value_start]);
            out.push_str(&bumped);
            out.push_str(&line[value_start + len..]);
            new_version = Some(bumped);
            continue;
        }
        out.push_str(line);
    }
    match new_version {
        Some(version) => Ok((out, version)),
        None => Err(manifest_err("no version line found")),
    }
}

/// Appends `"id",` as the last entry of the `tasks = [...]` array, adding a
/// separating comma when the current last entry lacks a trailing one.
///
/// Task ids contain no `]`, so the first `]` after the array opens closes it.
fn insert_task_entry(text: &str, id: &str) -> Result<String, PromoteError> {
    let mut tasks_offset: Option<usize> = None;
    let mut offset = 0_usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("tasks")
            && rest.trim_start().starts_with('=')
        {
            tasks_offset = Some(offset);
            break;
        }
        offset += line.len();
    }
    let Some(tasks_offset) = tasks_offset else {
        return Err(manifest_err("no tasks array found"));
    };
    let Some(open) = text[tasks_offset..].find('[').map(|i| tasks_offset + i) else {
        return Err(manifest_err("tasks array has no opening bracket"));
    };
    let Some(close) = text[open..].find(']').map(|i| open + i) else {
        return Err(manifest_err("tasks array has no closing bracket"));
    };

    let inner = text[open + 1..close].trim_end();
    let mut new_inner = inner.to_owned();
    if !inner.trim_start().is_empty() && !inner.ends_with(',') {
        new_inner.push(',');
    }
    new_inner.push_str("\n    \"");
    new_inner.push_str(id);
    new_inner.push_str("\",\n");
    Ok(format!("{}{}{}", &text[..=open], new_inner, &text[close..]))
}

/// Bumps `MAJOR.MINOR.PATCH` to `MAJOR.MINOR+1.0`; `None` when the version is
/// not three dot-separated integers.
fn bump_minor(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let _patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let next_minor = minor + 1;
    Some(format!("{major}.{next_minor}.0"))
}

#[cfg(test)]
mod tests {
    use super::{
        CandidateFile, Cluster, IntentViolationKind, MinedRun, MiningOptions, PatternClass,
        Signature, VectorRect, draft_candidates, scan, write_candidates,
    };
    use crate::{ResultRecord, Tier};
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
    use reticle_agent_api::{
        AgentCommand, AgentResponse, CommandRecord, ElementId, Outcome, Transcript,
    };
    use reticle_extract::{IntentNet, IntentSpec, Terminal};
    use reticle_geometry::{LayerId, Point, Rect};

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
                backend: "mock".into(),
                quantization: None,
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

    // ----- drafting ---------------------------------------------------------

    /// The synthetic corpus behind the committed sample candidate: two failed
    /// rectangle runs, tiers 1 and 2, that never clear `m1.1`.
    fn sample_corpus() -> Vec<MinedRun> {
        vec![
            failed_rect_run("t1_min_width_met1", 1, "m1.1"),
            failed_rect_run("t2_legal_spacing_met1", 2, "m1.1"),
        ]
    }

    /// A unique temp directory under the OS temp root, created fresh per call.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("reticle-bench-mining-{}-{n}", std::process::id()));
        // The OS reuses process ids over a ci run, and these dirs are not cleaned
        // up, so a later test can inherit a prior one's manifest and task files.
        // Clear any stale directory first so every test starts from an empty one.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    #[test]
    fn draft_builds_drc_candidate_with_full_provenance() {
        let corpus = sample_corpus();
        let options = MiningOptions::default();
        let clusters = scan(&corpus, &options);
        let candidates = draft_candidates(&corpus, &clusters, &options);
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.id, "cand_m1_1_rect_only");
        assert_eq!(candidate.tier, Tier(2), "tier is the max source tier");
        assert_eq!(candidate.checker, "drc_clean");
        assert!(candidate.intent.is_none());
        assert_eq!(candidate.technology, "sky130.tech");
        assert!(candidate.prompt.contains("m1.1"));
        assert!(candidate.prompt.contains("t1_min_width_met1"));
        assert!(candidate.prompt.contains("t2_legal_spacing_met1"));
        assert_eq!(
            candidate.provenance.signature,
            "drc=m1.1|pattern=rect_only|intent=none"
        );
        assert_eq!(candidate.provenance.drc_rules, vec!["m1.1".to_owned()]);
        assert_eq!(candidate.provenance.pattern, "rect_only");
        assert_eq!(candidate.provenance.source_runs.len(), 2);
        assert_eq!(
            candidate.provenance.source_runs[0].task_id,
            "t1_min_width_met1"
        );
        assert_eq!(candidate.provenance.source_runs[0].tier, 1);
        assert!(!candidate.provenance.source_runs[0].success);
        // Good: the canonical clean met1 rect. Bad: the representative's rects.
        assert_eq!(
            candidate.two_way.good,
            vec![VectorRect {
                layer: "68/20".into(),
                rect: [0, 0, 500, 500],
            }]
        );
        assert_eq!(candidate.two_way.bad.len(), 2);
        assert_eq!(candidate.two_way.bad[0].rect, [0, 0, 100, 100]);
        assert_eq!(candidate.two_way.bad[1].rect, [0, 200, 100, 300]);
    }

    #[test]
    fn replay_honors_recorded_deletes() {
        let run = run_of(
            "t1_fixup",
            1,
            false,
            2,
            vec![
                cmd(0, add_rect((68, 20), (0, 0), (100, 100)), ok_mutation(&[1])),
                cmd(
                    1,
                    AgentCommand::DeleteShapes {
                        ids: vec![ElementId(1)],
                    },
                    ok_mutation(&[1]),
                ),
                cmd(2, add_rect((68, 20), (0, 0), (90, 90)), ok_mutation(&[2])),
                drc_report(3, &["m1.1"]),
            ],
        );
        let options = MiningOptions {
            min_cluster_size: 1,
            ..MiningOptions::default()
        };
        let clusters = scan(std::slice::from_ref(&run), &options);
        let candidates = draft_candidates(std::slice::from_ref(&run), &clusters, &options);
        assert_eq!(
            candidates[0].two_way.bad,
            vec![VectorRect {
                layer: "68/20".into(),
                rect: [0, 0, 90, 90],
            }],
            "the deleted first rectangle must not survive into the bad vector"
        );
    }

    /// A two-terminal single-net intent spec on `layer`, terminals at the
    /// corners used by the intent drafting tests.
    fn two_terminal_spec(layer_a: LayerId, layer_b: LayerId) -> String {
        let spec = IntentSpec {
            nets: vec![IntentNet {
                name: "n".into(),
                terminals: vec![
                    Terminal {
                        name: "a".into(),
                        layer: layer_a,
                        region: Rect::new(Point::new(0, 0), Point::new(10, 10)),
                    },
                    Terminal {
                        name: "b".into(),
                        layer: layer_b,
                        region: Rect::new(Point::new(490, 290), Point::new(500, 300)),
                    },
                ],
            }],
            forbidden: vec![],
        };
        serde_json::to_string(&spec).expect("serialize spec")
    }

    /// An intent run: one rectangle drawn, then a `check_intent` carrying
    /// `spec` whose report leaves one open.
    fn open_intent_run(task_id: &str, spec: &str) -> MinedRun {
        run_of(
            task_id,
            3,
            false,
            4,
            vec![
                cmd(0, add_rect((68, 20), (0, 0), (10, 10)), ok_mutation(&[1])),
                cmd(
                    1,
                    AgentCommand::CheckIntent {
                        cell: "top".into(),
                        intent: spec.to_owned(),
                    },
                    data(serde_json::json!({
                        "opens": [{ "net": "n", "detail": "terminal b unreached" }],
                        "shorts": [],
                    })),
                ),
            ],
        )
    }

    #[test]
    fn intent_candidate_reuses_spec_and_covers_terminals() {
        let met1 = LayerId::new(68, 20);
        let spec = two_terminal_spec(met1, met1);
        let run = open_intent_run("t3_intent_met1_wire", &spec);
        let options = MiningOptions {
            min_cluster_size: 1,
            ..MiningOptions::default()
        };
        let clusters = scan(std::slice::from_ref(&run), &options);
        let candidates = draft_candidates(std::slice::from_ref(&run), &clusters, &options);
        let candidate = &candidates[0];
        assert_eq!(candidate.id, "cand_rect_only_open");
        assert_eq!(candidate.checker, "intent");
        assert_eq!(candidate.intent.as_deref(), Some(spec.as_str()));
        // The good vector covers the union bounding box of the net terminals.
        assert_eq!(
            candidate.two_way.good,
            vec![VectorRect {
                layer: "68/20".into(),
                rect: [0, 0, 500, 300],
            }]
        );
        assert_eq!(
            candidate.two_way.bad.len(),
            1,
            "the drawn rect is the bad vector"
        );
    }

    #[test]
    fn intent_spec_spanning_layers_gets_no_good_vector() {
        let spec = two_terminal_spec(LayerId::new(68, 20), LayerId::new(69, 20));
        let run = open_intent_run("t3_intent_layer_jog", &spec);
        let options = MiningOptions {
            min_cluster_size: 1,
            ..MiningOptions::default()
        };
        let clusters = scan(std::slice::from_ref(&run), &options);
        let candidates = draft_candidates(std::slice::from_ref(&run), &clusters, &options);
        assert!(
            candidates[0].two_way.good.is_empty(),
            "a single-layer cover cannot join terminals on two layers"
        );
    }

    #[test]
    fn no_geometry_candidate_asks_for_rect_present() {
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
        let options = MiningOptions {
            min_cluster_size: 1,
            ..MiningOptions::default()
        };
        let clusters = scan(std::slice::from_ref(&run), &options);
        let candidates = draft_candidates(std::slice::from_ref(&run), &clusters, &options);
        let candidate = &candidates[0];
        assert_eq!(candidate.checker, "rect_present");
        assert!(candidate.two_way.bad.is_empty());
        assert_eq!(candidate.two_way.good.len(), 1);
    }

    #[test]
    fn write_candidates_emits_parseable_toml() {
        let corpus = sample_corpus();
        let options = MiningOptions::default();
        let candidates = draft_candidates(&corpus, &scan(&corpus, &options), &options);
        let dir = tempdir();
        let paths = write_candidates(&dir, &candidates).expect("write candidates");
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("cand_m1_1_rect_only.toml"));
        let text = std::fs::read_to_string(&paths[0]).expect("read back");
        assert!(text.starts_with("# Mined candidate task (draft)"));
        let parsed: CandidateFile = toml::from_str(&text).expect("reparse");
        assert_eq!(&parsed, &candidates[0]);
    }

    #[test]
    fn committed_sample_candidate_matches_the_drafter() {
        let corpus = sample_corpus();
        let options = MiningOptions::default();
        let candidates = draft_candidates(&corpus, &scan(&corpus, &options), &options);
        assert_eq!(candidates.len(), 1);
        let dir = tempdir();
        let paths = write_candidates(&dir, &candidates).expect("write candidates");
        let generated = std::fs::read_to_string(&paths[0]).expect("read generated");

        let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/layout-tasks/candidates/cand_m1_1_rect_only.toml");
        if std::env::var_os("RETICLE_BLESS_SAMPLE").is_some() {
            if let Some(parent) = committed.parent() {
                std::fs::create_dir_all(parent).expect("create candidates dir");
            }
            std::fs::write(&committed, &generated).expect("bless the committed sample");
        }
        let text = std::fs::read_to_string(&committed).expect("read the committed sample");
        assert_eq!(
            text.replace("\r\n", "\n"),
            generated.replace("\r\n", "\n"),
            "the committed sample candidate drifted from the drafter; \
             regenerate it with RETICLE_BLESS_SAMPLE=1"
        );
    }

    // ----- promotion plumbing ------------------------------------------------

    #[test]
    fn bump_minor_resets_patch() {
        assert_eq!(super::bump_minor("0.2.0").as_deref(), Some("0.3.0"));
        assert_eq!(super::bump_minor("1.9.7").as_deref(), Some("1.10.0"));
        assert!(super::bump_minor("0.2").is_none());
        assert!(super::bump_minor("0.2.x").is_none());
        assert!(super::bump_minor("0.2.0.1").is_none());
    }

    /// A small manifest in the live suite's style: header comment, trailing
    /// commas.
    const MANIFEST_WITH_COMMENTS: &str = "# Suite header comment that must survive edits.\n\
        # Bump `version` when tasks change.\n\
        version = \"0.2.0\"\n\
        tasks = [\n    \"t1_existing\",\n]\n";

    #[test]
    fn edited_manifest_bumps_and_appends_preserving_comments() {
        let (text, version) =
            super::edited_manifest(MANIFEST_WITH_COMMENTS, "cand_new").expect("edit");
        assert_eq!(version, "0.3.0");
        assert!(text.starts_with("# Suite header comment"));
        assert!(text.contains("# Bump `version` when tasks change."));
        assert!(text.contains("version = \"0.3.0\""));
        let manifest: crate::SuiteManifest = toml::from_str(&text).expect("reparse");
        assert_eq!(
            manifest.tasks,
            vec!["t1_existing".to_owned(), "cand_new".to_owned()],
            "the new id lands at the end of the task list"
        );
    }

    #[test]
    fn edited_manifest_handles_missing_trailing_comma_and_empty_list() {
        let no_comma = "version = \"0.1.0\"\ntasks = [\n    \"t1_existing\"\n]\n";
        let (text, _) = super::edited_manifest(no_comma, "cand_new").expect("edit");
        let manifest: crate::SuiteManifest = toml::from_str(&text).expect("reparse");
        assert_eq!(manifest.tasks, vec!["t1_existing", "cand_new"]);

        let empty = "version = \"0.1.0\"\ntasks = []\n";
        let (text, version) = super::edited_manifest(empty, "cand_new").expect("edit");
        assert_eq!(version, "0.2.0");
        let manifest: crate::SuiteManifest = toml::from_str(&text).expect("reparse");
        assert_eq!(manifest.tasks, vec!["cand_new"]);
    }

    #[test]
    fn edited_manifest_rejects_malformed_input() {
        assert!(
            super::edited_manifest("tasks = []\n", "x").is_err(),
            "no version"
        );
        assert!(
            super::edited_manifest("version = \"0.1.0\"\n", "x").is_err(),
            "no tasks array"
        );
        assert!(
            super::edited_manifest("version = \"latest\"\ntasks = []\n", "x").is_err(),
            "unbumpable version"
        );
    }

    #[test]
    fn verify_two_way_rejects_malformed_vector_layer() {
        let corpus = sample_corpus();
        let options = MiningOptions::default();
        let mut candidates = draft_candidates(&corpus, &scan(&corpus, &options), &options);
        candidates[0].two_way.good[0].layer = "met1".into();
        let err = super::verify_two_way(&candidates[0]).expect_err("malformed layer");
        assert!(matches!(err, super::PromoteError::Candidate { .. }));
        assert!(!err.is_refusal(), "a malformed file is not a gate refusal");
    }

    // ----- the promotion gate, end to end ------------------------------------

    /// A minimal existing suite task, so a promoted suite still loads whole.
    const EXISTING_TASK: &str = "id = \"t1_existing\"\ntier = 1\nprompt = \"p\"\n\
        technology = \"sky130.tech\"\nchecker = \"rect_present\"\n";

    /// A run that issued no drawing command at all (the model went silent).
    fn silent_run(task_id: &str) -> MinedRun {
        run_of(
            task_id,
            1,
            false,
            1,
            vec![cmd(
                0,
                AgentCommand::CreateCell { name: "top".into() },
                ok_mutation(&[]),
            )],
        )
    }

    /// A temp suite (commented manifest at 0.2.0 plus one existing task) with
    /// the sample corpus drafted into its `candidates/` directory. Returns the
    /// suite dir, the candidates dir, and the drafted candidate.
    fn suite_with_sample_candidate() -> (std::path::PathBuf, std::path::PathBuf, CandidateFile) {
        let corpus = sample_corpus();
        let options = MiningOptions::default();
        let candidates = draft_candidates(&corpus, &scan(&corpus, &options), &options);
        let suite = tempdir();
        std::fs::write(suite.join("manifest.toml"), MANIFEST_WITH_COMMENTS)
            .expect("write manifest");
        std::fs::write(suite.join("t1_existing.toml"), EXISTING_TASK).expect("write existing");
        let candidates_dir = suite.join("candidates");
        write_candidates(&candidates_dir, &candidates).expect("write candidates");
        let candidate = candidates.into_iter().next().expect("one candidate");
        (suite, candidates_dir, candidate)
    }

    #[test]
    fn end_to_end_pipeline_scans_drafts_writes_and_promotes() {
        // Synthetic corpus: two m1.1 failures, two silent failures, one clean
        // pass. The pass is never mined; the failures form two clusters.
        let mut corpus = sample_corpus();
        corpus.push(silent_run("t1_silent_a"));
        corpus.push(silent_run("t1_silent_b"));
        corpus.push(run_of(
            "t1_ok",
            1,
            true,
            1,
            vec![cmd(
                0,
                add_rect((68, 20), (0, 0), (500, 500)),
                ok_mutation(&[1]),
            )],
        ));
        let options = MiningOptions::default();
        let clusters = scan(&corpus, &options);
        assert_eq!(clusters.len(), 2, "two recurring signatures");
        let candidates = draft_candidates(&corpus, &clusters, &options);
        let ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["cand_no_geometry", "cand_m1_1_rect_only"]);

        let suite = tempdir();
        std::fs::write(suite.join("manifest.toml"), MANIFEST_WITH_COMMENTS)
            .expect("write manifest");
        std::fs::write(suite.join("t1_existing.toml"), EXISTING_TASK).expect("write existing");
        let candidates_dir = suite.join("candidates");
        write_candidates(&candidates_dir, &candidates).expect("write candidates");

        let first = super::promote_candidate(&suite, &candidates_dir, "cand_m1_1_rect_only")
            .expect("the drc candidate passes its two-way gate");
        assert_eq!(first.new_version, "0.3.0");
        let second = super::promote_candidate(&suite, &candidates_dir, "cand_no_geometry")
            .expect("the rect_present candidate passes its two-way gate");
        assert_eq!(second.new_version, "0.4.0", "each promotion bumps again");

        // The grown suite still loads whole, in manifest order, and the
        // candidate files stay behind as provenance records.
        let (manifest, tasks) = crate::load_suite(&suite).expect("promoted suite loads");
        assert_eq!(manifest.version, "0.4.0");
        assert_eq!(
            manifest.tasks,
            vec!["t1_existing", "cand_m1_1_rect_only", "cand_no_geometry"]
        );
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[1].checker, "drc_clean");
        assert_eq!(tasks[2].checker, "rect_present");
        assert!(candidates_dir.join("cand_m1_1_rect_only.toml").exists());
    }

    #[test]
    fn promote_refuses_when_the_good_vector_fails() {
        let (suite, candidates_dir, mut candidate) = suite_with_sample_candidate();
        // A 100x100 met1 rect violates min width and area, so `drc_clean`
        // rejects the supposed known-good document.
        candidate.two_way.good = vec![VectorRect {
            layer: "68/20".into(),
            rect: [0, 0, 100, 100],
        }];
        write_candidates(&candidates_dir, std::slice::from_ref(&candidate)).expect("rewrite");

        let before = std::fs::read_to_string(suite.join("manifest.toml")).expect("manifest");
        let err = super::promote_candidate(&suite, &candidates_dir, &candidate.id)
            .expect_err("must refuse");
        assert!(matches!(
            err,
            super::PromoteError::GoodVectorRejected { .. }
        ));
        assert!(err.is_refusal());
        let after = std::fs::read_to_string(suite.join("manifest.toml")).expect("manifest");
        assert_eq!(before, after, "a refusal leaves the manifest untouched");
        assert!(
            !suite.join(format!("{}.toml", candidate.id)).exists(),
            "a refusal writes no task file"
        );
    }

    #[test]
    fn promote_refuses_when_the_bad_vector_passes() {
        let (suite, candidates_dir, mut candidate) = suite_with_sample_candidate();
        // A clean 500x500 met1 rect passes `drc_clean`, so the checker no
        // longer captures the mined failure mode.
        candidate.two_way.bad = vec![VectorRect {
            layer: "68/20".into(),
            rect: [0, 0, 500, 500],
        }];
        write_candidates(&candidates_dir, std::slice::from_ref(&candidate)).expect("rewrite");

        let err = super::promote_candidate(&suite, &candidates_dir, &candidate.id)
            .expect_err("must refuse");
        assert!(matches!(err, super::PromoteError::BadVectorAccepted));
        assert!(err.is_refusal());
        assert!(!suite.join(format!("{}.toml", candidate.id)).exists());
    }

    #[test]
    fn promote_refuses_an_unknown_checker() {
        let (suite, candidates_dir, mut candidate) = suite_with_sample_candidate();
        candidate.checker = "no_such_checker".into();
        write_candidates(&candidates_dir, std::slice::from_ref(&candidate)).expect("rewrite");

        let err = super::promote_candidate(&suite, &candidates_dir, &candidate.id)
            .expect_err("must refuse");
        assert!(matches!(err, super::PromoteError::Checker { .. }));
        assert!(err.is_refusal());
    }

    #[test]
    fn promote_refuses_double_promotion() {
        let (suite, candidates_dir, candidate) = suite_with_sample_candidate();
        super::promote_candidate(&suite, &candidates_dir, &candidate.id)
            .expect("first promotion passes");
        let err = super::promote_candidate(&suite, &candidates_dir, &candidate.id)
            .expect_err("second promotion must refuse");
        assert!(matches!(err, super::PromoteError::AlreadyInSuite { .. }));
        assert!(err.is_refusal());
    }
}
