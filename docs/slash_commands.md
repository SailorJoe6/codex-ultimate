# Slash commands

For an overview of Codex CLI slash commands, see [this documentation](https://developers.openai.com/codex/cli/slash-commands).

## Custom slash commands

Codex also supports custom slash commands stored as Markdown files in:

- Project scope: `<project-root>/.codex/commands/`
- User scope: `~/.codex/commands/`

The command name comes from the filename (for example `deploy.md` defines `/deploy`), while
subdirectories are only used for scope labeling in help/autocomplete. If a project and user
command share the same name, the project command takes precedence.

Custom commands can define optional YAML frontmatter fields:

- `description`
- `argument-hint` (or `argument_hint`)
- `allowed-tools`
- `model`
- `disable-model-invocation`

Command bodies support Claude-style placeholders:

- `$ARGUMENTS` for all arguments joined by spaces
- `$1` … `$9` for positional arguments

Custom commands are available in interactive sessions and in exec mode. Interactive sessions
periodically refresh the custom command list so edits are picked up without restarting. Invalid
frontmatter or built-in name conflicts are reported as discovery errors and the offending commands
are skipped.
