# Specification: Prevent Post-Completion `update_plan` Retry Loops

## Problem Statement
Codex can enter an infinite (or very long) loop where the model repeatedly calls `update_plan` after the plan is already complete. This is most visible during final-plan bookkeeping and can consume large token budgets without making progress.

This behavior is a runtime bug in loop handling for `update_plan` and must be fixed in Codex core.

## Current System Behavior
- `update_plan` accepts parsed arguments and always emits `EventMsg::PlanUpdate(args)`.
- The handler returns a generic success response (`"Plan updated"`) without checking for progress.
- Repeated post-completion calls are treated the same as meaningful updates.
- When this repeats, Codex can remain in a non-terminating tool-call loop.

## Scope
This spec is intentionally narrow and targets `update_plan` behavior only.

- In scope:
  - Post-completion retry detection for `update_plan`
  - Model guidance for low-count retries
  - Escalation to session termination at threshold
- Out of scope:
  - Generic cross-tool loop detection
  - Changes to other tools
  - Configurability of thresholds (hard-coded for now)

## Required Behavior

### 1. Applicability
Apply this guard in all collaboration modes where `update_plan` is allowed.

### 2. Completion Baseline
Once a plan update establishes an all-completed state (all steps `status=completed`) within the current turn/session context, Codex must treat subsequent no-progress all-completed submissions as retries.

### 3. No-Progress Definition (Post-Completion)
After completion baseline is established, a subsequent `update_plan` call is no-progress unless at least one of the following is true:
- `plan.length` increases (new steps added), or
- at least one step `status` value changes.

The following by themselves are no-progress:
- changing step text/description
- reordering steps
- changing `explanation`
- resubmitting semantically complete plans with different wording

### 4. Retry Counter Semantics
- Count only consecutive post-completion no-progress retries.
- The all-completed submission that first establishes the completion baseline is **not** a retry attempt.
- Reset the counter when real progress occurs (as defined above).
- Hard-coded threshold: `5`.

### 5. Behavior by Attempt Number
For consecutive post-completion no-progress retries (after baseline establishment):
- Attempts `1` through `4`:
  - Still emit `EventMsg::PlanUpdate` (do not suppress)
  - Return `FunctionCallError::RespondToModel(...)` with corrective guidance
  - Treat as turn-level error behavior (session remains alive)
- Attempt `5`:
  - Do **not** emit `EventMsg::PlanUpdate` for this attempt
  - Raise session-level fatal error and terminate the session

## Model Guidance Message
For attempts `1` through `4`, use a strong corrective message equivalent to:

`Your plan is already complete. Do not revise completed step text or explanation. If new work exists, add a step or change a step status; otherwise provide final response.`

Message text can be adjusted for tone/format, but semantics must remain the same.

## Non-Functional Requirements
- Prevent unbounded token burn caused by repeated post-completion `update_plan` calls.
- Preserve existing plan event visibility for retries `1` through `4`.
- Keep behavior deterministic once retry conditions are met.

## Acceptance Criteria
1. If the model repeatedly submits no-progress all-complete `update_plan` calls:
   - calls `1-4` receive corrective `RespondToModel` errors
   - corresponding `PlanUpdate` events are still emitted
   - call `5` terminates session and emits no `PlanUpdate` for that call
2. If the model adds a step or changes any status, retry counter resets.
3. Explanation-only, reorder-only, and text-only edits after completion count as retries.
4. Behavior is enforced in all modes where `update_plan` is allowed.
5. Threshold is hard-coded to `5`.
6. Integration-sequence interpretation is unambiguous:
   - baseline all-complete submission emits `PlanUpdate` and `"Plan updated"`
   - the next four no-progress all-complete submissions are retries `1-4`
   - the following no-progress all-complete submission is retry `5` (fatal, no `PlanUpdate`)

## Test Requirements
Add/adjust core tests to cover at minimum:
- Repeated post-completion no-progress calls reaching session termination on 5th attempt
- Event emission present for attempts `1-4` and absent on attempt `5`
- Retry counter reset on real progress (step added or status changed)
- Explanation/text/reorder-only post-completion updates counted as no-progress retries
- Guard behavior active in all `update_plan`-allowed modes

## Notes for Implementation
- This is a targeted safety fix for `update_plan` final-bookkeeping loops.
- Do not broaden to generic multi-tool loop detection in this change.
