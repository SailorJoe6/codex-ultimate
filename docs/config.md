# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Connecting to MCP servers

Codex can connect to MCP servers configured in `~/.codex/config.toml`. See the configuration reference for the latest MCP server options:

- https://developers.openai.com/codex/config-reference

## Apps (Connectors)

Use `$` in the composer to insert a ChatGPT connector; the popover lists accessible
apps. The `/apps` command lists available and installed apps. Connected apps appear first
and are labeled as connected; others are marked as can be installed.

## Notify

Codex can run a notification hook when the agent finishes a turn. See the configuration reference for the latest notification settings:

- https://developers.openai.com/codex/config-reference

When Codex knows which client started the turn, the legacy notify JSON payload also includes a top-level `client` field. The TUI reports `codex-tui`, and the app server reports the `clientInfo.name` value from `initialize`.

## JSON Schema

The generated JSON Schema for `config.toml` lives at `codex-rs/core/config.schema.json`.

## SQLite State DB

Codex stores the SQLite-backed state DB under `sqlite_home` (config key) or the
`CODEX_SQLITE_HOME` environment variable. When unset, WorkspaceWrite sandbox
sessions default to a temp directory; other modes default to `CODEX_HOME`.

## Sub-agent role config keys

Role declarations under `[agents.<role>]` support these keys:

- `description`: Human-facing role text surfaced in spawn tool guidance.
- `profile`: Optional profile name from `[profiles.<name>]`.
- `config_file`: Optional role-specific TOML layer path (resolved relative to the `config.toml` that declares it).

Role `config_file` paths are resolved during config loading but file existence/validity checks are deferred until role application time (for example during sub-agent spawn/resume flows).

When role layers are applied (`spawn_agent` and closed-thread `resume_agent` restores), precedence is:

1. Inherited parent config
2. Role `profile`
3. Role `config_file`
4. Spawn safety overrides (for example `approval_policy = never`)

For closed-agent resume, Codex re-reads current config files before role resolution so users can edit role/profile config and resume without restarting orchestration.

If role `profile` or `config_file` cannot be applied, Codex continues with remaining valid layers and emits warnings (both a warning event and a model-visible warning message) for each occurrence.

Explicit unknown `agent_type` values still fail spawn.
Closed resume also fails when the recorded role for that sub-agent no longer exists.

## Notices

Codex stores "do not show again" flags for some UI prompts under the `[notice]` table.

## Plan mode defaults

`plan_mode_reasoning_effort` lets you set a Plan-mode-specific default reasoning
effort override. When unset, Plan mode uses the built-in Plan preset default
(currently `medium`). When explicitly set (including `none`), it overrides the
Plan preset. The string value `none` means "no reasoning" (an explicit Plan
override), not "inherit the global default". There is currently no separate
config value for "follow the global default in Plan mode".

Ctrl+C/Ctrl+D quitting uses a ~1 second double-press hint (`ctrl + c again to quit`).
