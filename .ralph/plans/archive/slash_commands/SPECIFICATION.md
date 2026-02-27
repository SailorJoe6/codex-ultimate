# Specification: Custom Slash Commands (.codex/commands)

## Current system (as-is)
- Codex has built-in slash commands in the TUI/CLI.
- Codex supports **custom prompts** stored in `$CODEX_HOME/prompts` and invoked via `/prompts:<name>`.
  - Prompt files are Markdown `.md` with optional frontmatter limited to `description` and `argument-hint`.
  - These prompts appear in the slash popup and can be invoked by name.
- Codex can discover a `.codex` config directory by searching from `pwd` up to the project root, but it does **not** load custom slash commands from `.codex/commands` today.
  - Note: project root detection uses `project_root_markers` (defaults to [".git"]).

## Goal (to-be)
Add **custom slash commands** that work **like Claude’s documented slash commands**, with Codex-specific location changes:
- Project-level commands live in `.codex/commands/` at the **project root**.
- User-level commands live in `~/.codex/commands/`.
- Command behavior matches Claude’s documented slash-command behavior, **excluding** Claude’s broader “skills” extensions.

Custom commands must **coexist** with existing `/prompts:<name>` functionality. They do not replace custom prompts.

## Functional requirements

### Command discovery
- Load commands from **two roots only**:
  1) `~/.codex/commands/` (user scope)
  2) `<project-root>/.codex/commands/` (project scope)
- Do **not** load commands from intermediate `.codex/commands` directories between `pwd` and project root.
- Only Markdown files (`.md`) are commands. Non-files and non-`.md` are ignored.
- Subdirectories are allowed, but **command names are based only on filename** (no folder prefix).
  - Example: `.codex/commands/frontend/component.md` defines `/component`.
  - The folder path is only used for **scope labeling** in help/autocomplete (see below).
- If the same command name exists in both scopes, **the project command wins** and the user command is **hidden** from help/autocomplete and cannot be invoked.
- Built-in slash command names are **reserved**. If a custom command tries to use a built-in name, it is a **hard error** during discovery.

### Frontmatter support (Claude slash-command fields)
Support exactly these five frontmatter fields (same meaning as Claude’s slash-command docs):
- `description`
- `argument-hint` (also accept `argument_hint` for compatibility)
- `allowed-tools`
- `model`
- `disable-model-invocation`

Notes:
- These are **slash-command** fields only. Do **not** add Claude “skills”-level fields or behaviors.

### Command body behavior (Claude slash-command behavior)
- Support Claude-style argument interpolation and placeholders in the command body:
  - `$ARGUMENTS` expansion (all arguments as a single string)
  - `$1`, `$2`, … positional arguments
- Deferred (not yet supported): Claude-style file reference tokens in the body (e.g., `@path` references).
- Deferred (not yet supported): Claude-style inline shell execution (e.g., lines or tokens prefixed by `!`).

(Exact parsing/expansion behavior should match Claude’s documented slash-command behavior.)

### Invocation and availability
- Custom commands are usable **everywhere Codex accepts input**, including:
  - Interactive TUI
  - Interactive CLI
  - Non-interactive/exec mode
- Built-in commands always take precedence over custom names (and conflicts are errors as above).

### Help/autocomplete UI
- `/help` and the slash-command popup list **both built-in and custom commands**.
- Custom commands display:
  - `description` and `argument-hint` like Claude.
  - Scope label indicating source:
    - `project:<subdir>` or `user:<subdir>` when in subdirectories
    - `project` or `user` when directly under the commands root
- If a project command hides a user command of the same name, the **user command does not appear** in help/autocomplete.

### Error handling
- Invalid command files (e.g., malformed frontmatter or unsupported fields) produce an **error at discovery time**.
- Errors are surfaced immediately (startup/refresh) and the offending command is not usable.
- Built-in name conflicts are treated as **hard errors** at discovery time.

### Hot reload
- **Interactive sessions** (TUI and interactive CLI) should hot-reload custom commands when files change.
- **Non-interactive/exec mode** can remain load-once (no hot reload requirement).

## Non-goals
- Do not modify or replace `/prompts:<name>` behavior or its storage location.
- Do not implement Claude “skills” frontmatter fields or behaviors beyond the five slash-command fields listed above.
- Do not load commands from intermediate `.codex/commands` directories between `pwd` and project root.

## Deferred (not yet supported)
- `@path` file reference tokens in custom command bodies.
- Inline `!` shell execution in custom command bodies.

## Open clarifications resolved
- Project commands override user commands by hiding them.
- Built-in slash command names are reserved; conflicts are discovery-time errors.
- Custom commands should be available in all input modes, including exec.
- Hot reload is required for interactive sessions only.
