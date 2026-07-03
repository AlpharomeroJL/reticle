//! The model client the runner drives, and a deterministic mock implementation.
//!
//! [`ModelClient`] is the one seam between the runner and whatever proposes edits:
//! given the task prompt and a [`Context`] snapshot of the current attempt, it
//! returns the [`AgentCommand`]s to apply next. The runner calls it once per
//! propose-verify-correct iteration.
//!
//! [`MockModel`] is a scripted, fully deterministic client used for the whole test
//! and sample-suite flow, so the machinery exercises propose-verify-correct without a
//! live model. A script maps a task id to an ordered list of *attempts*, where each
//! attempt is the command batch for one iteration. The first attempt may be
//! deliberately violating (for example an under-width rectangle) and a later attempt
//! corrects it, so the runner observes a non-zero first-proposal violation count that
//! falls to zero.

use std::collections::HashMap;

use reticle_agent_api::AgentCommand;

/// What the runner tells the model about the attempt in progress.
///
/// Deliberately small and owned (no borrow of the session): the mock needs only the
/// iteration index to pick its scripted attempt, and a real client would serialize
/// this into a prompt. `violations` and `feedback` carry the verifier's result from
/// the previous attempt so a correcting model can react to it.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Zero-based index of the current propose-verify-correct iteration.
    pub iteration: u32,
    /// DRC/intent violation count from the previous attempt (`0` on the first).
    pub prev_violations: u32,
    /// Human-readable failure detail from the previous attempt, if any.
    pub feedback: Vec<String>,
}

/// A client that proposes engine commands for a task.
///
/// The runner calls [`propose`](ModelClient::propose) once per iteration, applies the
/// returned commands to the session, then verifies. `&mut self` lets a client keep
/// per-task state (the mock tracks nothing beyond its immutable script, but a live
/// client would hold a connection or token budget).
pub trait ModelClient {
    /// A stable identifier for this client, recorded in the [`ResultRecord`].
    ///
    /// [`ResultRecord`]: crate::ResultRecord
    fn id(&self) -> &str;

    /// Proposes the commands to apply for this iteration of `task_id`.
    ///
    /// `prompt` is the task's natural-language prompt and `context` describes the
    /// attempt (iteration index and the previous verification result). Returning an
    /// empty vector means "no further edits", which the runner treats as giving up on
    /// correcting.
    fn propose(&mut self, task_id: &str, prompt: &str, context: &Context) -> Vec<AgentCommand>;
}

/// A scripted, deterministic [`ModelClient`] for tests and the sample suite.
///
/// Built from a set of named scripts; each script is a task id mapped to an ordered
/// list of attempts. On iteration `i`, [`propose`](MockModel::propose) returns the
/// `i`-th attempt for the task, or an empty batch once the script is exhausted. A
/// script whose first attempt violates a rule and whose second attempt fixes it drives
/// the full propose-verify-correct loop.
#[derive(Clone, Debug, Default)]
pub struct MockModel {
    /// Per-task attempt scripts: `task_id -> [attempt_0, attempt_1, ...]`.
    scripts: HashMap<String, Vec<Vec<AgentCommand>>>,
    /// Fallback used when a task id has no specific script.
    default_script: Vec<Vec<AgentCommand>>,
}

impl MockModel {
    /// An empty mock: every task yields no commands until a script is added.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `attempts` as the script for `task_id`, one command batch per
    /// iteration, and returns `self` for chaining.
    #[must_use]
    pub fn with_script(
        mut self,
        task_id: impl Into<String>,
        attempts: Vec<Vec<AgentCommand>>,
    ) -> Self {
        self.scripts.insert(task_id.into(), attempts);
        self
    }

    /// Sets the fallback script used for any task id without its own entry.
    #[must_use]
    pub fn with_default(mut self, attempts: Vec<Vec<AgentCommand>>) -> Self {
        self.default_script = attempts;
        self
    }

    /// The attempts scripted for `task_id`, falling back to the default script.
    fn attempts_for(&self, task_id: &str) -> &[Vec<AgentCommand>] {
        self.scripts
            .get(task_id)
            .map_or(self.default_script.as_slice(), Vec::as_slice)
    }
}

impl ModelClient for MockModel {
    // The trait returns `&str` because a live client borrows its id from owned state;
    // the mock's id is a literal, but it must keep the trait's signature rather than
    // narrowing to `&'static str`.
    #[allow(clippy::unnecessary_literal_bound)]
    fn id(&self) -> &str {
        "mock"
    }

    fn propose(&mut self, task_id: &str, _prompt: &str, context: &Context) -> Vec<AgentCommand> {
        let attempts = self.attempts_for(task_id);
        attempts
            .get(context.iteration as usize)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, MockModel, ModelClient};
    use reticle_agent_api::AgentCommand;
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};

    /// A met1 rectangle command with the given extent, for building scripts.
    fn met1_rect(size: i32) -> AgentCommand {
        AgentCommand::AddRect {
            cell: "top".into(),
            layer: LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: RectArg {
                min: PointArg { x: 0, y: 0 },
                max: PointArg { x: size, y: size },
            },
        }
    }

    #[test]
    fn mock_returns_scripted_attempt_per_iteration() {
        let mut mock = MockModel::new().with_script(
            "t1",
            vec![
                vec![
                    AgentCommand::CreateCell { name: "top".into() },
                    met1_rect(100),
                ],
                vec![met1_rect(500)],
            ],
        );
        let first = mock.propose("t1", "p", &Context::default());
        assert_eq!(first.len(), 2);
        let second = mock.propose(
            "t1",
            "p",
            &Context {
                iteration: 1,
                prev_violations: 1,
                feedback: vec!["too narrow".into()],
            },
        );
        assert_eq!(second.len(), 1);
        assert_eq!(second[0], met1_rect(500));
    }

    #[test]
    fn mock_is_empty_past_end_of_script() {
        let mut mock = MockModel::new().with_script("t1", vec![vec![met1_rect(500)]]);
        let past = mock.propose(
            "t1",
            "p",
            &Context {
                iteration: 5,
                ..Context::default()
            },
        );
        assert!(past.is_empty());
    }

    #[test]
    fn mock_falls_back_to_default_script() {
        let mut mock = MockModel::new().with_default(vec![vec![met1_rect(500)]]);
        let cmds = mock.propose("unknown_task", "p", &Context::default());
        assert_eq!(cmds, vec![met1_rect(500)]);
    }

    #[test]
    fn mock_id_is_stable() {
        assert_eq!(MockModel::new().id(), "mock");
    }
}
