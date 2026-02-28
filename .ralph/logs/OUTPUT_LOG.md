Completed one high-priority remaining task: stabilized and validated the retry-threshold shutdown integration coverage for `update_plan`.

- Updated [tool_harness.rs](/home/sailorjoe6/Code/codex-ultimate/codex-rs/core/tests/suite/tool_harness.rs) to keep `thread_manager` alive in `update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt`, preventing false failures from `unsupported operation: thread manager dropped`.
- Kept assertions focused on spec-relevant behavior (event/output semantics) and not optional `success` flag presence.
- Updated [EXECUTION_PLAN.md](/home/sailorjoe6/Code/codex-ultimate/.ralph/plans/EXECUTION_PLAN.md) with current progress and test status.
- Created and closed beads issue `codex-ultimate-zsu`, then ran `bd sync`.

Validation run:
- `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core --lib update_plan_` passed.
- `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt -- --nocapture` passed.
- `PKG_CONFIG_PATH=/home/sailorjoe6/.local/libcap-pkgconfig cargo test -p codex-core` runs in this environment now, but currently has one unrelated failing test: `tools::handlers::multi_agents::tests::resume_agent_reloads_current_config_and_applies_recorded_role`.
- `just fmt` was run in `codex-rs`.

Next likely task: if you want, I can now move to the documentation-completion step for this spec (per your workflow) and then determine whether plan/spec should be archived.

----------------
Running handoff...
Handoff prep is complete.

Updated:
- [SPECIFICATION.md](/home/sailorjoe6/Code/codex-ultimate/.ralph/plans/SPECIFICATION.md)
- [EXECUTION_PLAN.md](/home/sailorjoe6/Code/codex-ultimate/.ralph/plans/EXECUTION_PLAN.md)

What I added:
- Clarified retry-attempt semantics (baseline is not a retry, post-baseline attempt numbering).
- Clarified the exact integration event/attempt sequence.
- Added a concise handoff snapshot in the plan:
  - touched files and why they changed
  - validation commands used (including local `libcap` workaround)
  - current known out-of-scope failing test in `codex-core`
  - next-programmer focus guidance

No new handoff doc was created. Beads tracking for this documentation task was created, closed, and synced.

Ready for further instructions.

---

Pass 2:
Audit complete: implementation for this spec is already in place and matches the acceptance criteria, so the highest-priority remaining task was documentation completion.

Changes made:
- Added new docs page: [`docs/update_plan.md`](/home/sailorjoe6/Code/codex-ultimate/docs/update_plan.md)
- Linked it from top-level docs list in [`README.md`](/home/sailorjoe6/Code/codex-ultimate/README.md)
- Updated then archived the finished planning docs:
  - [`SPECIFICATION.md`](/home/sailorjoe6/Code/codex-ultimate/.ralph/plans/archive/update_plan_retry_guard/SPECIFICATION.md)
  - [`EXECUTION_PLAN.md`](/home/sailorjoe6/Code/codex-ultimate/.ralph/plans/archive/update_plan_retry_guard/EXECUTION_PLAN.md)

Beads updates:
- Closed `codex-ultimate-q09` after adding completion notes.

Validation run:
- `cargo test -p codex-core --lib update_plan_` (pass)
- `cargo test -p codex-core update_plan_post_completion_retries_shutdown_on_fifth_no_progress_attempt` (pass)

Status: there is nothing left to do for this plan/spec track; awaiting your next instruction.
