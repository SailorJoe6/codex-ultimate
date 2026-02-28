use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;
use async_trait::async_trait;
use codex_protocol::config_types::ModeKind;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::EventMsg;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const POST_COMPLETION_RETRY_THRESHOLD: u8 = 5;
const POST_COMPLETION_RETRY_GUIDANCE: &str = "Your plan is already complete. Do not revise completed step text or explanation. If new work exists, add a step or change a step status; otherwise provide final response.";

pub struct PlanHandler;

pub static PLAN_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut plan_item_props = BTreeMap::new();
    plan_item_props.insert("step".to_string(), JsonSchema::String { description: None });
    plan_item_props.insert(
        "status".to_string(),
        JsonSchema::String {
            description: Some("One of: pending, in_progress, completed".to_string()),
        },
    );

    let plan_items_schema = JsonSchema::Array {
        description: Some("The list of steps".to_string()),
        items: Box::new(JsonSchema::Object {
            properties: plan_item_props,
            required: Some(vec!["step".to_string(), "status".to_string()]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut properties = BTreeMap::new();
    properties.insert(
        "explanation".to_string(),
        JsonSchema::String { description: None },
    );
    properties.insert("plan".to_string(), plan_items_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: "update_plan".to_string(),
        description: r#"Updates the task plan.
Provide an optional explanation and a list of plan items, each with a step and status.
At most one step can be in_progress at a time.
"#
        .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["plan".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
});

#[async_trait]
impl ToolHandler for PlanHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "update_plan handler received unsupported payload".to_string(),
                ));
            }
        };

        let content =
            handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }
}

/// This function doesn't do anything useful. However, it gives the model a structured way to record its plan that clients can read and render.
/// So it's the _inputs_ to this function that are useful to clients, not the outputs and neither are actually useful for the model other
/// than forcing it to come up and document a plan (TBD how that affects performance).
pub(crate) async fn handle_update_plan(
    session: &Session,
    turn_context: &TurnContext,
    arguments: String,
    call_id: String,
) -> Result<String, FunctionCallError> {
    if turn_context.collaboration_mode.mode == ModeKind::Plan {
        return Err(FunctionCallError::RespondToModel(
            "update_plan is a TODO/checklist tool and is not allowed in Plan mode".to_string(),
        ));
    }
    let args = parse_update_plan_arguments(&arguments)?;
    let turn_state = {
        let active_turn = session.active_turn.lock().await;
        active_turn
            .as_ref()
            .map(|active_turn| active_turn.turn_state.clone())
    };
    let retry_attempt = if let Some(turn_state) = turn_state {
        let mut turn_state = turn_state.lock().await;
        turn_state.register_update_plan(&args)
    } else {
        None
    };

    if let Some(retry_attempt) = retry_attempt {
        if retry_attempt >= POST_COMPLETION_RETRY_THRESHOLD {
            session
                .services
                .agent_control
                .shutdown_agent(session.conversation_id)
                .await
                .map_err(|err| {
                    FunctionCallError::Fatal(format!(
                        "failed to shutdown session after repeated update_plan retries: {err}"
                    ))
                })?;
            return Err(FunctionCallError::Fatal(format!(
                "update_plan post-completion no-progress retry threshold reached for call_id {call_id}; shutting down session"
            )));
        }

        session
            .send_event(turn_context, EventMsg::PlanUpdate(args))
            .await;
        return Err(FunctionCallError::RespondToModel(
            POST_COMPLETION_RETRY_GUIDANCE.to_string(),
        ));
    }

    session
        .send_event(turn_context, EventMsg::PlanUpdate(args))
        .await;
    Ok("Plan updated".to_string())
}

fn parse_update_plan_arguments(arguments: &str) -> Result<UpdatePlanArgs, FunctionCallError> {
    serde_json::from_str::<UpdatePlanArgs>(arguments).map_err(|e| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::make_session_and_context;
    use crate::state::ActiveTurn;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    fn completed_plan_args(explanation: &str) -> String {
        format!(
            r#"{{
            "explanation": "{explanation}",
            "plan": [
                {{"step":"Inspect workspace","status":"completed"}},
                {{"step":"Report results","status":"completed"}}
            ]
        }}"#
        )
    }

    async fn make_session_for_mode(mode: ModeKind) -> (Arc<Session>, Arc<TurnContext>) {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.collaboration_mode.mode = mode;
        let session = Arc::new(session);
        let turn_context = Arc::new(turn_context);
        *session.active_turn.lock().await = Some(ActiveTurn::default());
        (session, turn_context)
    }

    async fn call_update_plan_for_mode(mode: ModeKind) -> Result<String, FunctionCallError> {
        let (session, turn_context) = make_session_for_mode(mode).await;
        handle_update_plan(
            session.as_ref(),
            turn_context.as_ref(),
            completed_plan_args("complete"),
            "plan-call".to_string(),
        )
        .await
    }

    #[tokio::test]
    async fn update_plan_allowed_modes_accept_calls() {
        let default_result = call_update_plan_for_mode(ModeKind::Default).await;
        assert_eq!(default_result, Ok("Plan updated".to_string()));

        let execute_result = call_update_plan_for_mode(ModeKind::Execute).await;
        assert_eq!(execute_result, Ok("Plan updated".to_string()));

        let pair_result = call_update_plan_for_mode(ModeKind::PairProgramming).await;
        assert_eq!(pair_result, Ok("Plan updated".to_string()));
    }

    #[tokio::test]
    async fn update_plan_plan_mode_rejects_calls() {
        let result = call_update_plan_for_mode(ModeKind::Plan).await;
        assert_eq!(
            result,
            Err(FunctionCallError::RespondToModel(
                "update_plan is a TODO/checklist tool and is not allowed in Plan mode".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn update_plan_retry_guard_applies_in_allowed_modes() {
        for mode in [
            ModeKind::Default,
            ModeKind::Execute,
            ModeKind::PairProgramming,
        ] {
            let (session, turn_context) = make_session_for_mode(mode).await;

            let baseline = handle_update_plan(
                session.as_ref(),
                turn_context.as_ref(),
                completed_plan_args("baseline"),
                "plan-baseline".to_string(),
            )
            .await;
            assert_eq!(baseline, Ok("Plan updated".to_string()));

            let retry = handle_update_plan(
                session.as_ref(),
                turn_context.as_ref(),
                completed_plan_args("changed explanation"),
                "plan-retry".to_string(),
            )
            .await;
            assert_eq!(
                retry,
                Err(FunctionCallError::RespondToModel(
                    POST_COMPLETION_RETRY_GUIDANCE.to_string()
                ))
            );
        }
    }
}
