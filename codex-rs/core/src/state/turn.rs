//! Turn-scoped state and active turn metadata scaffolding.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use tokio::sync::oneshot;

use crate::codex::TurnContext;
use crate::protocol::ReviewDecision;
use crate::tasks::SessionTask;

/// Metadata about the currently running turn.
pub(crate) struct ActiveTurn {
    pub(crate) tasks: IndexMap<String, RunningTask>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            tasks: IndexMap::new(),
            turn_state: Arc::new(Mutex::new(TurnState::default())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    Regular,
    Review,
    Compact,
}

pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn SessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: Arc<AbortOnDropHandle<()>>,
    pub(crate) turn_context: Arc<TurnContext>,
    // Timer recorded when the task drops to capture the full turn duration.
    pub(crate) _timer: Option<codex_otel::Timer>,
}

impl ActiveTurn {
    pub(crate) fn add_task(&mut self, task: RunningTask) {
        let sub_id = task.turn_context.sub_id.clone();
        self.tasks.insert(sub_id, task);
    }

    pub(crate) fn remove_task(&mut self, sub_id: &str) -> bool {
        self.tasks.swap_remove(sub_id);
        self.tasks.is_empty()
    }

    pub(crate) fn drain_tasks(&mut self) -> Vec<RunningTask> {
        self.tasks.drain(..).map(|(_, task)| task).collect()
    }
}

/// Mutable state for a single turn.
#[derive(Default)]
pub(crate) struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_user_input: HashMap<String, oneshot::Sender<RequestUserInputResponse>>,
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pending_input: Vec<ResponseInputItem>,
    update_plan_retry: UpdatePlanRetryState,
}

impl TurnState {
    pub(crate) fn insert_pending_approval(
        &mut self,
        key: String,
        tx: oneshot::Sender<ReviewDecision>,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.insert(key, tx)
    }

    pub(crate) fn remove_pending_approval(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.remove(key)
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending_approvals.clear();
        self.pending_user_input.clear();
        self.pending_dynamic_tools.clear();
        self.pending_input.clear();
    }

    pub(crate) fn insert_pending_user_input(
        &mut self,
        key: String,
        tx: oneshot::Sender<RequestUserInputResponse>,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
        self.pending_user_input.insert(key, tx)
    }

    pub(crate) fn remove_pending_user_input(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
        self.pending_user_input.remove(key)
    }

    pub(crate) fn insert_pending_dynamic_tool(
        &mut self,
        key: String,
        tx: oneshot::Sender<DynamicToolResponse>,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.insert(key, tx)
    }

    pub(crate) fn remove_pending_dynamic_tool(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.remove(key)
    }

    pub(crate) fn push_pending_input(&mut self, input: ResponseInputItem) {
        self.pending_input.push(input);
    }

    pub(crate) fn take_pending_input(&mut self) -> Vec<ResponseInputItem> {
        if self.pending_input.is_empty() {
            Vec::with_capacity(0)
        } else {
            let mut ret = Vec::new();
            std::mem::swap(&mut ret, &mut self.pending_input);
            ret
        }
    }

    pub(crate) fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }

    /// Records one `update_plan` invocation and returns the consecutive
    /// post-completion no-progress retry attempt number when applicable.
    pub(crate) fn register_update_plan(&mut self, args: &UpdatePlanArgs) -> Option<u8> {
        let statuses = args
            .plan
            .iter()
            .map(|item| item.status.clone())
            .collect::<Vec<_>>();
        let all_completed = statuses
            .iter()
            .all(|status| matches!(status, StepStatus::Completed));

        let retry_state = &mut self.update_plan_retry;
        let had_completion_baseline = retry_state.completion_baseline_established;
        let had_previous_signature = retry_state.has_previous_signature;

        let plan_len_increased = had_previous_signature && args.plan.len() > retry_state.plan_len;
        let status_changed = had_previous_signature
            && retry_state
                .statuses
                .iter()
                .zip(&statuses)
                .any(|(previous, current)| {
                    std::mem::discriminant(previous) != std::mem::discriminant(current)
                });
        let made_progress = plan_len_increased || status_changed;

        let retry_attempt = if had_completion_baseline && all_completed && !made_progress {
            retry_state.consecutive_no_progress_retries = retry_state
                .consecutive_no_progress_retries
                .saturating_add(1);
            Some(retry_state.consecutive_no_progress_retries)
        } else {
            retry_state.consecutive_no_progress_retries = 0;
            None
        };

        if all_completed {
            retry_state.completion_baseline_established = true;
        }
        retry_state.plan_len = args.plan.len();
        retry_state.statuses = statuses;
        retry_state.has_previous_signature = true;

        retry_attempt
    }
}

#[derive(Default)]
struct UpdatePlanRetryState {
    completion_baseline_established: bool,
    plan_len: usize,
    statuses: Vec<StepStatus>,
    has_previous_signature: bool,
    consecutive_no_progress_retries: u8,
}

impl ActiveTurn {
    /// Clear any pending approvals and input buffered for the current turn.
    pub(crate) async fn clear_pending(&self) {
        let mut ts = self.turn_state.lock().await;
        ts.clear_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::TurnState;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use pretty_assertions::assert_eq;

    fn update_plan(plan: Vec<(&str, StepStatus)>, explanation: Option<&str>) -> UpdatePlanArgs {
        UpdatePlanArgs {
            explanation: explanation.map(str::to_string),
            plan: plan
                .into_iter()
                .map(|(step, status)| PlanItemArg {
                    step: step.to_string(),
                    status,
                })
                .collect(),
        }
    }

    #[test]
    fn register_update_plan_counts_post_completion_no_progress_retries() {
        let mut turn_state = TurnState::default();

        // Establish a completion baseline.
        let baseline = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            Some("baseline"),
        );
        assert_eq!(turn_state.register_update_plan(&baseline), None);

        // Explanation-only change.
        let explanation_only = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            Some("new explanation"),
        );
        assert_eq!(turn_state.register_update_plan(&explanation_only), Some(1));

        // Text-only change.
        let text_only = update_plan(
            vec![
                ("Inspect repository", StepStatus::Completed),
                ("Summarize results", StepStatus::Completed),
            ],
            Some("new explanation"),
        );
        assert_eq!(turn_state.register_update_plan(&text_only), Some(2));

        // Reorder-only change.
        let reordered = update_plan(
            vec![
                ("Summarize results", StepStatus::Completed),
                ("Inspect repository", StepStatus::Completed),
            ],
            Some("new explanation"),
        );
        assert_eq!(turn_state.register_update_plan(&reordered), Some(3));
    }

    #[test]
    fn register_update_plan_resets_retry_counter_on_progress() {
        let mut turn_state = TurnState::default();

        let baseline = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            None,
        );
        assert_eq!(turn_state.register_update_plan(&baseline), None);

        let retry = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            Some("retry"),
        );
        assert_eq!(turn_state.register_update_plan(&retry), Some(1));

        // A status change is progress and resets the retry counter.
        let status_change = update_plan(
            vec![
                ("Inspect workspace", StepStatus::InProgress),
                ("Report results", StepStatus::Completed),
            ],
            Some("status change"),
        );
        assert_eq!(turn_state.register_update_plan(&status_change), None);

        let completed_again = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            Some("completed again"),
        );
        assert_eq!(turn_state.register_update_plan(&completed_again), None);

        let retry_after_reset = update_plan(
            vec![
                ("Inspect workspace", StepStatus::Completed),
                ("Report results", StepStatus::Completed),
            ],
            Some("retry after reset"),
        );
        assert_eq!(turn_state.register_update_plan(&retry_after_reset), Some(1));
    }
}
