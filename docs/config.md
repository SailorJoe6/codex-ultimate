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

## JSON Schema

The generated JSON Schema for `config.toml` lives at `codex-rs/core/config.schema.json`.

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

Unsupported keys inside a selected role `profile` are ignored; supported keys are still applied, and each ignored key emits a warning.

Explicit unknown `agent_type` values still fail spawn.
Closed resume also fails when the recorded role for that sub-agent no longer exists.

## Notices

Codex stores "do not show again" flags for some UI prompts under the `[notice]` table.

Ctrl+C/Ctrl+D quitting uses a ~1 second double-press hint (`ctrl + c again to quit`).
