# Execution Plan: Prevent Post-Completion `update_plan` Retry Loops

## Context
- `README.md` reviewed for project overview and repo layout.
- `docs/README.md` does not exist in this checkout; docs inventory was inferred from `docs/` contents (`config.md`, `prompts.md`, `skills.md`, `sandbox.md`, `tui-*`, etc.).
- Spec source reviewed at `.ralph/plans/SPECIFICATION.md`.

## Progress Update (2026-02-27)
- Implemented turn-scoped `update_plan` retry tracking in `TurnState` with baseline, status signature, and consecutive retry counting.
- Implemented handler behavior for retries 1-4 (`RespondToModel` guidance + `PlanUpdate`) and retry 5 (`Fatal` + no `PlanUpdate` + shutdown request).
- Added guard classification tests in `core/src/state/turn.rs` (explanation-only, text-only, reorder-only, and reset-on-progress behavior).
- Added mode-coverage tests in `core/src/tools/handlers/plan.rs`, including retry-guard activation in `Default`, `Execute`, and `PairProgramming`.
- Added integration coverage in `core/tests/suite/tool_harness.rs` for repeated no-progress retries reaching shutdown and event/output behavior.
- Stabilized the integration harness by retaining `thread_manager` for the shutdown-path test so `shutdown_agent` can enqueue `Op::Shutdown` in test context.
- Added docs coverage in `docs/update_plan.md` and linked it from `README.md` to document the finalized post-completion retry-guard behavior.
- Ran `just fmt` successfully.
- Validated update-plan focused tests with a local non-root `libcap` pkg-config workaround (`PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig`):
  - `cargo test -p codex-core --lib update_plan_`
  - `cargo test -p codex-core update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt`
- Ran `cargo test -p codex-core`; result: spec-related tests pass, but one unrelated pre-existing failure remains in `tools::handlers::multi_agents::tests::resume_agent_reloads_current_config_and_applies_recorded_role`.

## Handoff Snapshot

### Code Areas Touched
- `codex-rs/core/src/state/turn.rs`
  - Added turn-scoped retry classifier state (`UpdatePlanRetryState`) and `register_update_plan`.
- `codex-rs/core/src/tools/handlers/plan.rs`
  - Added retry-threshold constants/guidance and handler branching for retries `1-4` vs retry `5`.
  - Retry `5` requests shutdown via `agent_control.shutdown_agent(...)` and returns fatal error.
- `codex-rs/core/src/codex.rs`
  - `drain_in_flight` now propagates tool future errors instead of swallowing them.
- `codex-rs/core/tests/suite/tool_harness.rs`
  - Added `update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt`.
  - Important harness nuance: test must retain `thread_manager` from `TestCodex` to keep `AgentControl` manager weak-reference valid for shutdown.
  - Assertions intentionally validate output text + event behavior; do not rely on `FunctionCallOutput.success` always being present.
- `codex-rs/core/src/tools/handlers/plan.rs` tests
  - Added mode coverage for `Default`, `Execute`, `PairProgramming`.
- `codex-rs/core/src/state/turn.rs` tests
  - Added classification/reset tests for explanation/text/reorder no-progress changes.

### Expected E2E Retry Sequence
- Baseline complete plan submission: emits `PlanUpdate`, tool output `"Plan updated"`.
- Next four no-progress complete submissions: retries `1-4`, each emits `PlanUpdate`, each returns guidance.
- Next no-progress complete submission: retry `5`, emits no `PlanUpdate`, returns fatal + session shutdown (`ShutdownComplete` expected).

### Validation Commands
- Formatting:
  - `just fmt`
- Focused tests:
  - `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core --lib update_plan_`
  - `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt -- --nocapture`
- Crate run:
  - `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core`
  - Current unrelated failure: `tools::handlers::multi_agents::tests::resume_agent_reloads_current_config_and_applies_recorded_role`

### Next Programmer Focus
- This spec’s acceptance criteria are implemented and covered.
- If continuing work under this plan, treat remaining `codex-core` crate failure as out-of-scope unless requested; do not regress update-plan guard behavior while addressing it.

## Audit Summary (Spec vs Current Code)

| Spec Requirement | Current State | Gap |
|---|---|---|
| Guard applies in all modes where `update_plan` is allowed | Guard logic runs for all non-Plan modes; Plan mode remains explicitly rejected | None |
| Establish completion baseline once all steps are `completed` | Implemented in `TurnState::register_update_plan` with completion baseline flag | None |
| No-progress definition after baseline: only `plan.len` growth or status change counts as progress; text/reorder/explanation changes do not | Implemented comparator based on plan length growth and ordered status signature changes | None |
| Count consecutive post-completion no-progress retries; reset on real progress | Implemented retry counter with reset semantics in `TurnState` | None |
| Threshold hard-coded to `5` | Implemented as `POST_COMPLETION_RETRY_THRESHOLD: u8 = 5` | None |
| Attempts 1-4: emit `PlanUpdate` + return `RespondToModel` corrective message | Implemented in `handle_update_plan` | None |
| Attempt 5: do not emit `PlanUpdate`; raise fatal and terminate | Implemented fatal path + shutdown request; no `PlanUpdate` emitted for threshold attempt | None |
| Tests for retries/events/reset/no-progress categories/mode coverage | Unit tests added for classification/reset; handler mode coverage added; integration test added for repeated retries and shutdown semantics | None for this spec; workspace-wide `codex-core` currently has one unrelated failing test in multi-agents resume behavior |

## Target Design

### 1. Add Turn-Scoped Update-Plan Guard State
Add state to `TurnState` (`core/src/state/turn.rs`) to track, per active turn context:
- Last seen plan status signature (statuses in order and plan length)
- Whether an all-completed baseline has been observed
- Consecutive post-completion no-progress retry count

Rationale:
- Guard must be deterministic and turn-scoped.
- `TurnState` already stores mutable per-turn coordination state and is reachable from tool handlers via `Session.active_turn`.

### 2. Implement Retry Classification in `update_plan` Handler
Modify `handle_update_plan` in `core/src/tools/handlers/plan.rs`:
- Keep current Plan-mode rejection behavior unchanged.
- Parse args.
- Compute derived facts:
  - `all_completed`
  - `plan_len`
  - ordered `Vec<StepStatus>` signature
- Compare against previous signature to determine real progress:
  - progress if `plan_len` increased, or any status value changed
  - no-progress otherwise
- Retry logic:
  - Only treat as retry when baseline has been established and current submission is `all_completed` + no-progress
  - Increment consecutive retry counter only for those calls
  - Reset counter on progress
- Attempt handling:
  - Attempts `1..=4`: emit `EventMsg::PlanUpdate(args)` and return `Err(FunctionCallError::RespondToModel(corrective_message))`
  - Attempt `5`: trigger session shutdown behavior and do **not** emit `PlanUpdate`
- Non-retry path: emit `PlanUpdate` and return `"Plan updated"`.

### 3. Session Shutdown Semantics on Attempt 5
On the 5th consecutive post-completion no-progress retry, treat this as a session-level fatal:
- Raise a fatal path from tool handling.
- Convert that fatal path into an explicit shutdown operation (`Op::Shutdown`) in core turn/session flow.
- Ensure shutdown produces the normal shutdown lifecycle (`EventMsg::ShutdownComplete`) and stops further model/tool looping.

### 4. Corrective Guidance Message
Use a strong corrective message aligned with spec semantics:
- "Your plan is already complete. Do not revise completed step text or explanation. If new work exists, add a step or change a step status; otherwise provide final response."

Tone can be adjusted, but semantic content must stay equivalent.

## Test Plan

## A. Core loop guard behavior
File: `codex-rs/core/tests/suite/tool_harness.rs` (or new dedicated suite file if readability is better).

Add integration tests that drive repeated `update_plan` function calls across consecutive sampling requests:

1. `no_progress_retries_terminate_on_5th_attempt`
- Send 5 consecutive post-completion no-progress all-completed `update_plan` calls.
- Assert attempts 1-4 return corrective tool output.
- Assert attempt 5 does not emit `PlanUpdate` and transitions into shutdown path (`ShutdownComplete` observed).

2. `plan_update_events_emitted_for_attempts_1_to_4_only`
- Collect emitted events.
- Assert 4 `EventMsg::PlanUpdate` events for attempts 1-4.
- Assert no additional `PlanUpdate` for attempt 5.

3. `retry_counter_resets_on_real_progress`
- Establish baseline.
- Trigger retries.
- Submit progress update (add step or status change) and verify counter reset (next no-progress retry is attempt 1 again).

4. `explanation_text_reorder_only_changes_count_as_no_progress`
- Explanation-only change after baseline.
- Step text-only edit after baseline.
- Reorder-only after baseline.
- Assert each is counted as no-progress retry.

## B. Mode coverage
Add mode-coverage tests to ensure behavior is active for all update-plan-allowed modes:
- `ModeKind::Default`
- `ModeKind::Execute`
- `ModeKind::PairProgramming`

Plan mode remains rejected by existing check and should not enter retry logic.

## C. Existing tests to preserve
Ensure current tests still pass:
- Normal `update_plan` event emission path
- Malformed payload rejection

## Implementation Steps
1. Completed: extend `TurnState` with update-plan retry tracking fields and accessors.
2. Completed: implement comparator + retry counter logic in `handle_update_plan`.
3. Completed: ensure fatal tool-call errors propagate through in-flight drain so retry-threshold fatal is not swallowed.
4. Completed: test coverage refinement and validation of acceptance criteria (including `ShutdownComplete` path).
5. Completed: run `just fmt` in `codex-rs`.
6. Completed: run focused and crate-level `codex-core` tests using local `libcap` pkg-config workaround; noted one unrelated existing failure outside this spec scope.
