# `update_plan` retry guard

`update_plan` now includes a post-completion retry guard to prevent long-running
no-progress loops.

## Scope

- Applies in collaboration modes where `update_plan` is allowed (`default`,
  `execute`, `pair-programming`).
- `plan` mode remains rejected for `update_plan` calls.

## Completion baseline

The guard activates after Codex first receives an all-completed plan
(`status = completed` for every step) during the active turn context.

## No-progress classification

After the completion baseline is established, a new all-completed `update_plan`
call is treated as **no-progress** unless at least one of these is true:

- The plan length increases (a step was added).
- A step status changed.

These changes are explicitly treated as no-progress on their own:

- Explanation-only edits
- Step text-only edits
- Reordering steps

## Retry behavior

- The guard counts only **consecutive** post-completion no-progress retries.
- The baseline all-completed submission is not counted as a retry.
- The counter resets when real progress is observed.
- Threshold is hard-coded to `5`.

Behavior by attempt number:

1. Attempts `1`-`4`: emit `PlanUpdate` and return corrective guidance to the model.
2. Attempt `5`: emit no `PlanUpdate`, raise a fatal error, and trigger session shutdown.

This keeps normal plan visibility for early retries while preventing unbounded
token usage from repeated post-completion bookkeeping calls.
