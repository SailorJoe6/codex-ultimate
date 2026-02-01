use codex_protocol::custom_commands::CustomCommand;
use codex_protocol::custom_commands::CustomCommandErrorInfo;
use codex_protocol::custom_commands::CustomCommandScope;
use serde_yaml::Value as YamlValue;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;
use toml::Value as TomlValue;

use crate::config::Config;

const DEFAULT_PROJECT_ROOT_MARKERS: &[&str] = &[".git"];
const RESERVED_COMMAND_NAMES: &[&str] = &[
    "model",
    "personality",
    "approvals",
    "permissions",
    "setup-elevated-sandbox",
    "experimental",
    "skills",
    "review",
    "new",
    "resume",
    "fork",
    "init",
    "compact",
    "collab",
    "agent",
    "diff",
    "mention",
    "status",
    "mcp",
    "logout",
    "quit",
    "exit",
    "feedback",
    "rollout",
    "ps",
    "test-approval",
];

pub struct CustomCommandsOutcome {
    pub commands: Vec<CustomCommand>,
    pub errors: Vec<CustomCommandErrorInfo>,
}

struct ParsedFrontmatter {
    description: Option<String>,
    argument_hint: Option<String>,
    allowed_tools: Option<Vec<String>>,
    model: Option<String>,
    disable_model_invocation: Option<bool>,
    body: String,
}

pub async fn discover_custom_commands(cwd: &Path, config: &Config) -> CustomCommandsOutcome {
    let user_root = default_user_commands_root();
    discover_custom_commands_with_roots(cwd, config, user_root).await
}

async fn discover_custom_commands_with_roots(
    cwd: &Path,
    config: &Config,
    user_root: Option<PathBuf>,
) -> CustomCommandsOutcome {
    let project_root = find_project_root(cwd, config).await;
    let mut errors = Vec::new();

    let (user_commands, user_errors) =
        discover_commands_in_root(user_root.as_deref(), CustomCommandScope::User).await;
    errors.extend(user_errors);

    let project_commands_root = project_root
        .as_ref()
        .map(|root| root.join(".codex").join("commands"));
    let (project_commands, project_errors) = discover_commands_in_root(
        project_commands_root.as_deref(),
        CustomCommandScope::Project,
    )
    .await;
    errors.extend(project_errors);

    let mut project_by_name: HashMap<String, CustomCommand> = HashMap::new();
    for command in project_commands {
        project_by_name.insert(command.name.clone(), command);
    }

    let mut commands: Vec<CustomCommand> = Vec::new();
    commands.extend(project_by_name.values().cloned());
    for command in user_commands {
        if !project_by_name.contains_key(&command.name) {
            commands.push(command);
        }
    }

    commands.sort_by(|a, b| a.name.cmp(&b.name));
    errors.sort_by(|a, b| a.path.cmp(&b.path));

    CustomCommandsOutcome { commands, errors }
}

fn default_user_commands_root() -> Option<PathBuf> {
    crate::config::find_codex_home()
        .ok()
        .map(|home| home.join("commands"))
}

async fn find_project_root(cwd: &Path, config: &Config) -> Option<PathBuf> {
    let markers = project_root_markers_from_config(config);
    if markers.is_empty() {
        return Some(cwd.to_path_buf());
    }

    for ancestor in cwd.ancestors() {
        for marker in &markers {
            let marker_path = ancestor.join(marker);
            if fs::metadata(&marker_path).await.is_ok() {
                return Some(ancestor.to_path_buf());
            }
        }
    }
    Some(cwd.to_path_buf())
}

fn project_root_markers_from_config(config: &Config) -> Vec<String> {
    let merged = config.config_layer_stack.effective_config();
    let TomlValue::Table(table) = merged else {
        return DEFAULT_PROJECT_ROOT_MARKERS
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
    };

    let Some(markers_value) = table.get("project_root_markers") else {
        return DEFAULT_PROJECT_ROOT_MARKERS
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
    };

    let TomlValue::Array(markers) = markers_value else {
        return DEFAULT_PROJECT_ROOT_MARKERS
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
    };

    let mut out = Vec::new();
    for marker in markers {
        let Some(marker) = marker.as_str() else {
            return DEFAULT_PROJECT_ROOT_MARKERS
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
        };
        out.push(marker.to_string());
    }
    out
}

async fn discover_commands_in_root(
    root: Option<&Path>,
    scope: CustomCommandScope,
) -> (Vec<CustomCommand>, Vec<CustomCommandErrorInfo>) {
    let mut commands = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    let Some(root) = root else {
        return (commands, errors);
    };

    if !fs::metadata(root)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        return (commands, errors);
    }

    let mut queue = vec![root.to_path_buf()];
    while let Some(dir) = queue.pop() {
        let mut entries = match fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(err) => {
                errors.push(CustomCommandErrorInfo {
                    path: dir,
                    message: format!("failed to read commands directory: {err}"),
                });
                continue;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(err) => {
                    errors.push(CustomCommandErrorInfo {
                        path,
                        message: format!("failed to read command file type: {err}"),
                    });
                    continue;
                }
            };

            if file_type.is_dir() {
                queue.push(path);
                continue;
            }

            let mut is_file = file_type.is_file();
            if file_type.is_symlink() {
                let meta = match fs::metadata(&path).await {
                    Ok(meta) => meta,
                    Err(err) => {
                        errors.push(CustomCommandErrorInfo {
                            path,
                            message: format!("failed to resolve command symlink: {err}"),
                        });
                        continue;
                    }
                };
                if meta.is_dir() {
                    continue;
                }
                is_file = meta.is_file();
            }

            if !is_file {
                continue;
            }
            if !is_markdown(&path) {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                errors.push(CustomCommandErrorInfo {
                    path,
                    message: "command filename is not valid UTF-8".to_string(),
                });
                continue;
            };
            if RESERVED_COMMAND_NAMES.contains(&name.as_str()) {
                errors.push(CustomCommandErrorInfo {
                    path,
                    message: format!("`/{name}` conflicts with a built-in command name"),
                });
                continue;
            }
            if !seen.insert(name.clone()) {
                errors.push(CustomCommandErrorInfo {
                    path,
                    message: format!("duplicate command name `/{name}` in {scope:?} scope"),
                });
                continue;
            }

            let content = match fs::read_to_string(&path).await {
                Ok(content) => content,
                Err(err) => {
                    errors.push(CustomCommandErrorInfo {
                        path,
                        message: format!("failed to read command file: {err}"),
                    });
                    continue;
                }
            };

            let parsed = match parse_frontmatter(&content) {
                Ok(parsed) => parsed,
                Err(message) => {
                    errors.push(CustomCommandErrorInfo { path, message });
                    continue;
                }
            };

            let scope_subdir = scope_subdir(root, &path);
            commands.push(CustomCommand {
                name,
                path,
                content: parsed.body,
                description: parsed.description,
                argument_hint: parsed.argument_hint,
                allowed_tools: parsed.allowed_tools,
                model: parsed.model,
                disable_model_invocation: parsed.disable_model_invocation,
                scope,
                scope_subdir,
            });
        }
    }

    commands.sort_by(|a, b| a.name.cmp(&b.name));
    (commands, errors)
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn scope_subdir(root: &Path, path: &Path) -> Option<String> {
    let parent = path.parent().unwrap_or(root);
    let rel = parent.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(path_to_slash(rel))
}

fn path_to_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_frontmatter(content: &str) -> Result<ParsedFrontmatter, String> {
    let mut segments = content.split_inclusive('\n');
    let Some(first_segment) = segments.next() else {
        return Ok(ParsedFrontmatter {
            description: None,
            argument_hint: None,
            allowed_tools: None,
            model: None,
            disable_model_invocation: None,
            body: String::new(),
        });
    };
    let first_line = first_segment.trim_end_matches(['\r', '\n']);
    if first_line.trim() != "---" {
        return Ok(ParsedFrontmatter {
            description: None,
            argument_hint: None,
            allowed_tools: None,
            model: None,
            disable_model_invocation: None,
            body: content.to_string(),
        });
    }

    let mut frontmatter = String::new();
    let mut consumed = first_segment.len();
    let mut frontmatter_closed = false;

    for segment in segments {
        let line = segment.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed == "---" {
            frontmatter_closed = true;
            consumed += segment.len();
            break;
        }
        frontmatter.push_str(segment);
        consumed += segment.len();
    }

    if !frontmatter_closed {
        return Err("unterminated frontmatter block".to_string());
    }

    let body = if consumed >= content.len() {
        String::new()
    } else {
        content[consumed..].to_string()
    };

    let (description, argument_hint, allowed_tools, model, disable_model_invocation) =
        if frontmatter.trim().is_empty() {
            (None, None, None, None, None)
        } else {
            parse_frontmatter_fields(&frontmatter)?
        };

    Ok(ParsedFrontmatter {
        description,
        argument_hint,
        allowed_tools,
        model,
        disable_model_invocation,
        body,
    })
}

fn parse_frontmatter_fields(
    frontmatter: &str,
) -> Result<
    (
        Option<String>,
        Option<String>,
        Option<Vec<String>>,
        Option<String>,
        Option<bool>,
    ),
    String,
> {
    let yaml: YamlValue =
        serde_yaml::from_str(frontmatter).map_err(|err| format!("invalid frontmatter: {err}"))?;
    let mapping = match yaml {
        YamlValue::Mapping(map) => map,
        YamlValue::Null => return Ok((None, None, None, None, None)),
        _ => return Err("frontmatter must be a mapping".to_string()),
    };

    let mut description = None;
    let mut argument_hint = None;
    let mut allowed_tools = None;
    let mut model = None;
    let mut disable_model_invocation = None;

    for (key, value) in mapping {
        let key = match key {
            YamlValue::String(key) => key,
            _ => return Err("frontmatter keys must be strings".to_string()),
        };
        match key.as_str() {
            "description" => description = parse_optional_string(value, "description")?,
            "argument-hint" | "argument_hint" => {
                argument_hint = parse_optional_string(value, "argument-hint")?;
            }
            "allowed-tools" => allowed_tools = parse_string_list(value, "allowed-tools")?,
            "model" => model = parse_optional_string(value, "model")?,
            "disable-model-invocation" => {
                disable_model_invocation = parse_optional_bool(value, "disable-model-invocation")?;
            }
            _ => {
                return Err(format!("unsupported frontmatter field `{key}`"));
            }
        }
    }

    Ok((
        description,
        argument_hint,
        allowed_tools,
        model,
        disable_model_invocation,
    ))
}

fn parse_optional_string(value: YamlValue, field: &str) -> Result<Option<String>, String> {
    match value {
        YamlValue::Null => Ok(None),
        YamlValue::String(s) => Ok(Some(s)),
        _ => Err(format!("`{field}` must be a string")),
    }
}

fn parse_optional_bool(value: YamlValue, field: &str) -> Result<Option<bool>, String> {
    match value {
        YamlValue::Null => Ok(None),
        YamlValue::Bool(b) => Ok(Some(b)),
        _ => Err(format!("`{field}` must be a boolean")),
    }
}

fn parse_string_list(value: YamlValue, field: &str) -> Result<Option<Vec<String>>, String> {
    match value {
        YamlValue::Null => Ok(None),
        YamlValue::Sequence(items) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    YamlValue::String(s) => out.push(s),
                    _ => return Err(format!("`{field}` must be a list of strings")),
                }
            }
            Ok(Some(out))
        }
        _ => Err(format!("`{field}` must be a list of strings")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn project_commands_shadow_user_commands() {
        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let user_root = codex_home.join("commands");
        fs::create_dir_all(&user_root).expect("user commands");
        fs::write(user_root.join("hello.md"), "user").expect("user command");

        let project_root = tmp.path().join("project");
        fs::create_dir_all(project_root.join(".codex").join("commands")).expect("project commands");
        fs::write(project_root.join(".git"), "gitdir: here").expect("marker");
        fs::write(
            project_root
                .join(".codex")
                .join("commands")
                .join("hello.md"),
            "project",
        )
        .expect("project command");

        let cwd = project_root.join("child");
        fs::create_dir_all(&cwd).expect("cwd");

        let outcome = discover_custom_commands_with_roots(&cwd, &config, Some(user_root)).await;

        let names: Vec<String> = outcome
            .commands
            .iter()
            .map(|cmd| cmd.name.clone())
            .collect();
        assert_eq!(names, vec!["hello".to_string()]);
        assert_eq!(outcome.commands[0].content, "project".to_string());
    }

    #[tokio::test]
    async fn rejects_built_in_names() {
        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let project_root = tmp.path().join("project");
        let commands_root = project_root.join(".codex").join("commands");
        fs::create_dir_all(&commands_root).expect("commands root");
        fs::write(project_root.join(".git"), "gitdir: here").expect("marker");
        fs::write(commands_root.join("init.md"), "nope").expect("command");

        let outcome = discover_custom_commands_with_roots(&project_root, &config, None).await;

        assert_eq!(outcome.commands.len(), 0);
        assert_eq!(outcome.errors.len(), 1);
        assert!(
            outcome.errors[0]
                .message
                .contains("conflicts with a built-in command")
        );
    }

    #[tokio::test]
    async fn rejects_unknown_frontmatter_fields() {
        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let user_root = codex_home.join("commands");
        fs::create_dir_all(&user_root).expect("user commands");
        fs::write(user_root.join("bad.md"), "---\nfoo: bar\n---\nhello").expect("command");

        let outcome =
            discover_custom_commands_with_roots(tmp.path(), &config, Some(user_root)).await;

        assert_eq!(outcome.commands.len(), 0);
        assert_eq!(outcome.errors.len(), 1);
        assert!(
            outcome.errors[0]
                .message
                .contains("unsupported frontmatter field")
        );
    }

    #[tokio::test]
    async fn captures_scope_subdir() {
        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let user_root = codex_home.join("commands");
        let nested = user_root.join("frontend");
        fs::create_dir_all(&nested).expect("nested");
        fs::write(nested.join("widget.md"), "hi").expect("command");

        let outcome =
            discover_custom_commands_with_roots(tmp.path(), &config, Some(user_root)).await;

        assert_eq!(outcome.commands.len(), 1);
        assert_eq!(
            outcome.commands[0].scope_subdir,
            Some("frontend".to_string())
        );
    }

    #[tokio::test]
    async fn parses_frontmatter_fields() {
        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let user_root = codex_home.join("commands");
        fs::create_dir_all(&user_root).expect("user commands");
        fs::write(
            user_root.join("deploy.md"),
            "---\ndescription: Deploy\nargument-hint: env\nallowed-tools:\n  - shell\nmodel: gpt-4.1\ndisable-model-invocation: true\n---\nrun",
        )
        .expect("command");

        let outcome =
            discover_custom_commands_with_roots(tmp.path(), &config, Some(user_root)).await;

        assert_eq!(outcome.commands.len(), 1);
        let command = &outcome.commands[0];
        assert_eq!(command.description, Some("Deploy".to_string()));
        assert_eq!(command.argument_hint, Some("env".to_string()));
        assert_eq!(command.allowed_tools, Some(vec!["shell".to_string()]));
        assert_eq!(command.model, Some("gpt-4.1".to_string()));
        assert_eq!(command.disable_model_invocation, Some(true));
        assert_eq!(command.content, "run".to_string());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovers_symlinked_commands() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().expect("tempdir");
        let codex_home = tmp.path().join("codex_home");
        fs::create_dir_all(&codex_home).expect("codex home");
        let config = crate::config::test_config();

        let user_root = codex_home.join("commands");
        fs::create_dir_all(&user_root).expect("user commands");

        let target_root = tmp.path().join("targets");
        fs::create_dir_all(&target_root).expect("targets root");
        let target_path = target_root.join("source.md");
        fs::write(&target_path, "hello").expect("target");

        let link_path = user_root.join("linked.md");
        symlink(&target_path, &link_path).expect("symlink");

        let outcome =
            discover_custom_commands_with_roots(tmp.path(), &config, Some(user_root)).await;

        let names: Vec<String> = outcome
            .commands
            .iter()
            .map(|cmd| cmd.name.clone())
            .collect();
        assert_eq!(names, vec!["linked".to_string()]);
        assert_eq!(outcome.commands[0].content, "hello".to_string());
    }
}
