# Execution Plan: Custom Slash Commands (.codex/commands)

## Scope
Implement Claude-style custom slash commands from:
- User scope: `~/.codex/commands/`
- Project scope: `<project-root>/.codex/commands/`

Commands must coexist with `/prompts:<name>` and work in TUI, interactive CLI, and exec mode.

## Audit Summary (Current State)
- Custom commands from `.codex/commands` are discovered in `codex-core` (user + project scopes) with frontmatter validation and discovery errors.
- TUI loads custom commands, shows scope labels + argument hints, expands `/name` on submit, and hot reloads via polling.
- Exec mode loads custom commands at startup, expands `/name` with `$ARGUMENTS`/`$1..$9`, and surfaces discovery errors as warnings.
- Built-in slash command name collisions are discovery-time errors in core and are surfaced via UI warnings.
- Custom command overrides (`allowed-tools`, `model`, `disable-model-invocation`) are wired through protocol/core/exec/TUI; per-turn tool allowlists filter built-in/MCP/dynamic tools and `disable-model-invocation` skips model calls with a warning.
- Deferred: no parsing for `@path` references or inline `!` shell execution (explicitly out of scope for now).

## Gaps vs. Spec
- Missing non-TUI interactive CLI integration for listing/invoking custom commands (if applicable outside the TUI).
- Deferred: `@path` reference parsing and inline `!` shell execution support in command bodies.

## Plan of Action

## Progress Update (2026-01-31)
- Implemented core discovery for `.codex/commands` (user + project), including frontmatter validation, reserved-name errors, scope labels, and list_custom_commands wiring in `codex-core`.
- Added core discovery tests in `codex-rs/core/src/custom_commands.rs`.
- Added shared expansion helper for `$ARGUMENTS` and `$1..$9` in `codex-rs/core`.
- Exec mode now loads custom commands once, expands initial prompts, and surfaces discovery errors as warnings.
- Updated `docs/slash_commands.md` with custom command details.
- TUI now requests custom command lists, displays custom commands in the slash popup with scope labels + argument hints, expands `/name` on submit, and surfaces discovery errors as warnings.
- TUI now polls for custom command updates (periodic refresh) to support hot reload during interactive sessions.
- Exec mode now expands `/prompts:<name>` using saved custom prompts (matches TUI behavior for named and positional args).
- TUI tests run: `cargo test -p codex-tui` (passes; `insta` warns about legacy snapshot format, no new snapshots created).
- Test status: `cargo test -p codex-core` currently failing in two unrelated tests:
  - `suite::pending_input::injected_user_input_triggers_follow_up_request_with_deltas`
  - `suite::tool_parallelism::read_file_tools_run_in_parallel` (timing threshold exceeded, ~2s vs expected <1s)

## Current Status (handoff, 2026-02-01)
- Custom command discovery + expansion is complete in core and exec; TUI integration (listing, hints, hot reload, expansion) is complete.
- Per-command overrides are implemented:
  - `allowed-tools` filters tool specs/handlers per turn (built-in, MCP, dynamic tools).
  - `model` overrides the per-turn model selection.
  - `disable-model-invocation` skips the model call and emits a warning.
- Custom command expansion helper: `codex-rs/core/src/custom_command_expansion.rs` (uses shlex to parse args).
- Custom prompt expansion helper: `codex-rs/core/src/custom_prompt_expansion.rs` (wired into exec).
- Exec loads commands/prompts once at startup and emits discovery errors as warning events in `codex-rs/exec/src/lib.rs`.
- Docs updated: `docs/slash_commands.md` (custom commands + overrides) and `docs/prompts.md`.
- Tests run:
  - `just fmt`
  - `cargo test -p codex-core` failed: `suite::tool_parallelism::read_file_tools_run_in_parallel` (timing threshold exceeded, ~2.15s)
  - `cargo test -p codex-tui` (pass; legacy snapshot format warnings)
  - `cargo test -p codex-exec` (pass)
  - `cargo test -p codex-app-server` (pass)
- Full suite `cargo test --all-features` not run (per policy, requires explicit approval).

## Current Status Update (2026-02-01)
- Branch rebased onto `origin/main`, conflicts resolved, and force-pushed to `custom_slash_commands`.
- Build/install: `cargo build -p codex-cli --release` and `cargo install --path codex-rs/cli --force` completed.
- Tests: `cargo test -p codex-tui` and `cargo test -p codex-exec` passed; `cargo test -p codex-core` had two flaky failures in full run but both passed when run individually; `cargo test --all-features` timed out after ~4 minutes with no failures reported before timeout.
- **User report:** custom slash commands “not working at all” in the installed build (needs investigation).
  
## Remaining Work (most important)
- Confirm whether a non-TUI interactive CLI path exists; if so, add custom command listing/expansion and surface discovery errors there.
- If interactive CLI work is not needed, update this plan to mark it as out of scope and archive the plan/spec.
- Investigate report that custom slash commands are not working in the installed build:
  - Verify command files exist at `~/.codex/commands/*.md` or `<project-root>/.codex/commands/*.md` and are valid UTF-8 Markdown.
  - Check for discovery errors in the UI warning history and in exec-mode warnings (errors should surface immediately).
  - Confirm no built-in name collisions or invalid frontmatter fields (errors cause commands to be skipped).
  - Validate project root detection (`project_root_markers`) points to the expected root for project-scoped commands.
  - In TUI, confirm `Op::ListCustomCommands` is being sent on startup and the polling loop is active.
  - In exec mode, confirm expansion runs before the first prompt and that `/name` is expanded in the CLI prompt input.
  - If commands are discovered but not expanding, verify placeholder usage ($ARGUMENTS/$1..$9) and that the input begins with `/name`.

## Progress Update (2026-02-01)
- Implemented `allowed-tools`, `model`, and `disable-model-invocation` execution overrides for custom commands.
- Tool allowlist now filters built-in, MCP, and dynamic tool specs on a per-turn basis.
- Added per-turn skip of model invocation when `disable-model-invocation` is set.
- Updated protocol, exec, and TUI plumbing to propagate command overrides.
- Docs updated in `docs/slash_commands.md`.
- Added core test for allowlist filtering; updated TUI tests to handle new submission metadata.
- Tests run:
  - `just fmt`
  - `cargo test -p codex-core` failed: `suite::tool_parallelism::read_file_tools_run_in_parallel` (timing threshold exceeded, ~2.15s)
  - `cargo test -p codex-tui` (pass; legacy snapshot format warnings)
  - `cargo test -p codex-exec` (pass)
  - `cargo test -p codex-app-server` (pass)

### 1) Data Model + Protocol
- Add a new custom command type (e.g., `CustomCommand`) in `codex-rs/protocol` with fields:
  - `name`, `path`, `content`
  - `description`, `argument_hint`
  - `allowed_tools`, `model`, `disable_model_invocation`
  - `scope` (user/project) and `scope_subdir` for UI labeling
- Add protocol messages to list custom commands (parallel to `ListCustomPrompts`).
- Include discovery errors in the response payload so UI/CLI can surface them.

### 2) Command Discovery (Core)
- Implement discovery in `codex-rs/core`:
  - Roots: `~/.codex/commands` and `<project-root>/.codex/commands` only.
  - Ignore non-`.md`, non-files; allow subdirectories but use filename-only for command name.
  - Enforce built-in command name conflicts as hard errors.
  - Enforce frontmatter validation (only 5 allowed fields; malformed/unknown fields -> error).
  - Resolve scope label (`project:<subdir>`, `user:<subdir>`, or `project`/`user`).
  - Apply project-over-user hiding (project command shadows user command of same name).
- Use `project_root` detection from config loader (same markers used today) to locate the project commands root.
  - Implemented by reading `project_root_markers` from the effective config layer stack.

### 3) Command Expansion Engine (Shared)
- Extract a shared expansion module (likely in `codex-rs/core` or `codex-rs/protocol`) for:
  - `$ARGUMENTS` and `$1...$9` substitution

### 4) TUI / Interactive CLI Integration
- Load custom commands in interactive sessions and display them alongside built-ins and `/prompts:`.
- Update popup rows to show:
  - Description and argument hint
  - Scope label (`project`, `project:<subdir>`, `user`, `user:<subdir>`)
- Ensure hidden user commands (shadowed by project command) are not shown.
- Ensure built-in commands remain reserved (conflicts are error, not hidden).
- Hot reload implemented via periodic polling of `Op::ListCustomCommands` in the TUI.
- TODO: if interactive CLI exists outside TUI, ensure it refreshes too.
- TODO: surface discovery errors in non-TUI UI/CLI (warnings/errors list), and ensure invalid commands are omitted.

### 5) Exec Mode + Non-TUI Input
- Integrate custom command expansion in exec mode before sending the prompt to the agent.
- Maintain parity with TUI: same discovery, precedence, and expansion behavior.
- Exec mode can be load-once (no hot reload required).
  - Done: exec expansion + discovery warnings are in place.

### 6) Error Reporting
- Surface discovery errors immediately on startup/refresh in interactive UI and CLI output.
- Ensure commands with errors are not available in help/popup or invocation paths.

### 7) Tests
- Core discovery tests:
  - Roots and precedence (project over user)
  - Built-in name conflicts
  - Invalid frontmatter and unsupported fields
  - Subdir scope labeling
- Expansion tests:
  - `$ARGUMENTS` and `$1..$9`
- TUI tests:
  - Popup list includes custom commands + hints + scope
  - Shadowed user commands hidden
  - Errors surfaced and offending commands omitted
- Exec tests:
  - Custom command expansion in non-interactive runs

### Test Runtime Guidance
- Full `cargo nextest run --no-fail-fast` currently takes ~3–4 minutes on a warm build and can be heavy on memory; use `-j 2` if linking OOMs.
- Prefer targeted runs first (`cargo test -p <crate>` or `cargo nextest run -p <crate> <test_name>`) to validate changes quickly.
- When full-suite runs are needed, run them once at the end and keep the output for any failures to avoid repeated long cycles.

### 8) Documentation Updates
- Add/refresh docs for custom slash commands in `docs/slash_commands.md` (and any other relevant docs).
- Ensure docs clarify coexistence with `/prompts:<name>` and the two command roots.

## Final Step: Commit and Push

When all work is complete and tests pass:
```bash
git add -A
git commit -m "Implement custom slash commands (.codex/commands)"
git push origin custom_slash_commands
```

This feature branch (`custom_slash_commands`) should be merged to `main` after review.

## Deferred (Not Yet Supported)
- `@path` reference tokens in custom command bodies.
- Inline `!` shell execution in custom command bodies.
