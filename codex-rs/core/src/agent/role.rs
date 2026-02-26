use crate::config::AgentRoleConfig;
use crate::config::Config;
use crate::config::ConfigOverrides;
use crate::config::deserialize_config_toml_with_base;
use crate::config_loader::ConfigLayerEntry;
use crate::config_loader::ConfigLayerStack;
use crate::config_loader::ConfigLayerStackOrdering;
use crate::config_loader::resolve_relative_paths_in_config_toml;
use codex_app_server_protocol::ConfigLayerSource;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use toml::Value as TomlValue;

const BUILT_IN_EXPLORER_CONFIG: &str = include_str!("builtins/explorer.toml");
const DEFAULT_ROLE_NAME: &str = "default";

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingRoleBehavior {
    Error,
    WarnAndContinue,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ApplyRoleOutcome {
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleConfigSource {
    BuiltIn,
    UserDefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRole {
    name: String,
    profile: Option<String>,
    config_file: Option<PathBuf>,
    source: RoleConfigSource,
}

/// Applies role-selected profile and config layers to a mutable config.
/// Layer order is: profile first, then config_file.
pub(crate) async fn apply_role_to_config(
    config: &mut Config,
    role_name: Option<&str>,
    missing_role_behavior: MissingRoleBehavior,
) -> Result<ApplyRoleOutcome, String> {
    let role_resolution_effective_config = config.config_layer_stack.effective_config();
    let role_resolution_agent_roles = config.agent_roles.clone();
    apply_role_to_config_with_role_resolution(
        config,
        role_name,
        missing_role_behavior,
        &role_resolution_agent_roles,
        &role_resolution_effective_config,
    )
    .await
}

/// Applies role-selected profile and config layers to `config`, while resolving
/// role/profile definitions from `resolution_config`.
pub(crate) async fn apply_role_to_config_with_resolution_config(
    config: &mut Config,
    role_name: Option<&str>,
    missing_role_behavior: MissingRoleBehavior,
    resolution_config: &Config,
) -> Result<ApplyRoleOutcome, String> {
    let role_resolution_effective_config = resolution_config.config_layer_stack.effective_config();
    let role_resolution_agent_roles = resolution_config.agent_roles.clone();
    apply_role_to_config_with_role_resolution(
        config,
        role_name,
        missing_role_behavior,
        &role_resolution_agent_roles,
        &role_resolution_effective_config,
    )
    .await
}

async fn apply_role_to_config_with_role_resolution(
    config: &mut Config,
    role_name: Option<&str>,
    missing_role_behavior: MissingRoleBehavior,
    role_resolution_agent_roles: &BTreeMap<String, AgentRoleConfig>,
    role_resolution_effective_config: &TomlValue,
) -> Result<ApplyRoleOutcome, String> {
    let role_name = role_name.unwrap_or(DEFAULT_ROLE_NAME);
    let mut outcome = ApplyRoleOutcome::default();
    let Some(role) = resolve_role(role_resolution_agent_roles, role_name) else {
        return match missing_role_behavior {
            MissingRoleBehavior::Error => Err(format!("unknown agent_type '{role_name}'")),
            MissingRoleBehavior::WarnAndContinue => {
                outcome.warnings.push(format!(
                    "unknown agent_type '{role_name}'; continuing without role"
                ));
                Ok(outcome)
            }
        };
    };

    if let Some(profile_name) = role.profile.as_deref() {
        apply_role_profile_layer(
            config,
            &role.name,
            profile_name,
            role_resolution_effective_config,
            &mut outcome.warnings,
        );
    }
    if let Some(config_file) = role.config_file.as_ref() {
        apply_role_config_file_layer(
            config,
            &role.name,
            config_file,
            role.source,
            &mut outcome.warnings,
        )
        .await;
    }

    Ok(outcome)
}

fn resolve_role(
    role_resolution_agent_roles: &BTreeMap<String, AgentRoleConfig>,
    role_name: &str,
) -> Option<ResolvedRole> {
    if let Some(role) = role_resolution_agent_roles.get(role_name) {
        return Some(ResolvedRole {
            name: role_name.to_string(),
            profile: role.profile.clone(),
            config_file: role.config_file.clone(),
            source: RoleConfigSource::UserDefined,
        });
    }

    built_in::configs().get(role_name).map(|role| ResolvedRole {
        name: role_name.to_string(),
        profile: role.profile.clone(),
        config_file: role.config_file.clone(),
        source: RoleConfigSource::BuiltIn,
    })
}

fn apply_role_profile_layer(
    config: &mut Config,
    role_name: &str,
    profile_name: &str,
    role_resolution_effective_config: &TomlValue,
    warnings: &mut Vec<String>,
) {
    let (profile_layer, profile_warnings) = match role_profile_layer(
        role_resolution_effective_config,
        profile_name,
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            warnings.push(format!(
                "Role `{role_name}` references profile `{profile_name}` that could not be used: {err}. Continuing."
            ));
            return;
        }
    };
    warnings.extend(profile_warnings);

    if profile_layer
        .as_table()
        .is_some_and(toml::map::Map::is_empty)
    {
        return;
    }

    if let Err(err) = apply_config_layer(config, profile_layer) {
        warnings.push(format!(
            "Role `{role_name}` profile `{profile_name}` could not be applied: {err}. Continuing."
        ));
    }
}

fn role_profile_layer(
    effective_config: &TomlValue,
    profile_name: &str,
) -> Result<(TomlValue, Vec<String>), String> {
    let profile_table = effective_config
        .as_table()
        .and_then(|table| table.get("profiles"))
        .and_then(TomlValue::as_table)
        .and_then(|profiles| profiles.get(profile_name))
        .and_then(TomlValue::as_table)
        .ok_or_else(|| "profile not found".to_string())?;

    let mut layer_table = toml::map::Map::new();
    let mut warnings = Vec::new();

    for (key, value) in profile_table {
        match key.as_str() {
            "tools_web_search" | "tools_view_image" => {
                let tools_entry = layer_table
                    .entry("tools".to_string())
                    .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
                if let Some(tools_table) = tools_entry.as_table_mut() {
                    let tools_key = if key == "tools_web_search" {
                        "web_search"
                    } else {
                        "view_image"
                    };
                    tools_table.insert(tools_key.to_string(), value.clone());
                }
            }
            "model"
            | "model_provider"
            | "approval_policy"
            | "sandbox_mode"
            | "model_reasoning_effort"
            | "model_reasoning_summary"
            | "model_verbosity"
            | "personality"
            | "chatgpt_base_url"
            | "model_instructions_file"
            | "js_repl_node_path"
            | "js_repl_node_module_dirs"
            | "zsh_path"
            | "experimental_instructions_file"
            | "experimental_compact_prompt_file"
            | "experimental_use_unified_exec_tool"
            | "experimental_use_freeform_apply_patch"
            | "web_search"
            | "analytics"
            | "windows"
            | "features"
            | "oss_provider" => {
                layer_table.insert(key.clone(), value.clone());
            }
            unsupported_key => warnings.push(format!(
                "Role profile `{profile_name}` key `{unsupported_key}` is not supported and was ignored."
            )),
        }
    }

    Ok((TomlValue::Table(layer_table), warnings))
}

async fn apply_role_config_file_layer(
    config: &mut Config,
    role_name: &str,
    config_file: &Path,
    source: RoleConfigSource,
    warnings: &mut Vec<String>,
) {
    let role_layer_toml = match load_role_config_layer(config, config_file, source).await {
        Ok(layer_toml) => layer_toml,
        Err(err) => {
            warnings.push(format!(
                "Role `{role_name}` config_file `{}` could not be loaded: {err}. Continuing.",
                config_file.display()
            ));
            return;
        }
    };
    if let Err(err) = apply_config_layer(config, role_layer_toml) {
        warnings.push(format!(
            "Role `{role_name}` config_file `{}` could not be applied: {err}. Continuing.",
            config_file.display()
        ));
    }
}

async fn load_role_config_layer(
    config: &Config,
    config_file: &Path,
    source: RoleConfigSource,
) -> Result<TomlValue, String> {
    let (role_config_contents, role_config_base) = if source == RoleConfigSource::BuiltIn {
        (
            built_in::config_file_contents(config_file)
                .map(str::to_owned)
                .ok_or_else(|| "built-in config file is unavailable".to_string())?,
            config.codex_home.as_path(),
        )
    } else {
        (
            tokio::fs::read_to_string(config_file)
                .await
                .map_err(|err| err.to_string())?,
            config_file
                .parent()
                .ok_or_else(|| "role config file has no parent directory".to_string())?,
        )
    };

    let role_config_toml: TomlValue =
        toml::from_str(&role_config_contents).map_err(|err| err.to_string())?;
    deserialize_config_toml_with_base(role_config_toml.clone(), role_config_base)
        .map_err(|err| err.to_string())?;
    resolve_relative_paths_in_config_toml(role_config_toml, role_config_base)
        .map_err(|err| err.to_string())
}

fn apply_config_layer(config: &mut Config, layer_toml: TomlValue) -> Result<(), String> {
    let mut layers: Vec<ConfigLayerEntry> = config
        .config_layer_stack
        .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
        .into_iter()
        .cloned()
        .collect();
    let layer = ConfigLayerEntry::new(ConfigLayerSource::SessionFlags, layer_toml);
    let insertion_index =
        layers.partition_point(|existing_layer| existing_layer.name <= layer.name);
    layers.insert(insertion_index, layer);

    let config_layer_stack = ConfigLayerStack::new(
        layers,
        config.config_layer_stack.requirements().clone(),
        config.config_layer_stack.requirements_toml().clone(),
    )
    .map_err(|err| err.to_string())?;

    let merged_toml = config_layer_stack.effective_config();
    let merged_config = deserialize_config_toml_with_base(merged_toml, &config.codex_home)
        .map_err(|err| err.to_string())?;
    let next_config = Config::load_config_with_layer_stack(
        merged_config,
        ConfigOverrides {
            cwd: Some(config.cwd.clone()),
            codex_linux_sandbox_exe: config.codex_linux_sandbox_exe.clone(),
            js_repl_node_path: config.js_repl_node_path.clone(),
            ..Default::default()
        },
        config.codex_home.clone(),
        config_layer_stack,
    )
    .map_err(|err| err.to_string())?;
    *config = next_config;

    Ok(())
}

pub(crate) mod spawn_tool_spec {
    use super::*;

    /// Builds the spawn-agent tool description text from built-in and configured roles.
    pub(crate) fn build(user_defined_agent_roles: &BTreeMap<String, AgentRoleConfig>) -> String {
        let built_in_roles = built_in::configs();
        build_from_configs(built_in_roles, user_defined_agent_roles)
    }

    // This function is not inlined for testing purpose.
    fn build_from_configs(
        built_in_roles: &BTreeMap<String, AgentRoleConfig>,
        user_defined_roles: &BTreeMap<String, AgentRoleConfig>,
    ) -> String {
        let mut seen = BTreeSet::new();
        let mut formatted_roles = Vec::new();
        for (name, declaration) in user_defined_roles {
            if seen.insert(name.as_str()) {
                formatted_roles.push(format_role(name, declaration));
            }
        }
        for (name, declaration) in built_in_roles {
            if seen.insert(name.as_str()) {
                formatted_roles.push(format_role(name, declaration));
            }
        }

        format!(
            r#"Optional type name for the new agent. If omitted, `{DEFAULT_ROLE_NAME}` is used.
Available roles:
{}
            "#,
            formatted_roles.join("\n"),
        )
    }

    fn format_role(name: &str, declaration: &AgentRoleConfig) -> String {
        if let Some(description) = &declaration.description {
            format!("{name}: {{\n{description}\n}}")
        } else {
            format!("{name}: no description")
        }
    }
}

mod built_in {
    use super::*;

    /// Returns the cached built-in role declarations defined in this module.
    pub(super) fn configs() -> &'static BTreeMap<String, AgentRoleConfig> {
        static CONFIG: LazyLock<BTreeMap<String, AgentRoleConfig>> = LazyLock::new(|| {
            BTreeMap::from([
                (
                    DEFAULT_ROLE_NAME.to_string(),
                    AgentRoleConfig {
                        description: Some("Default agent.".to_string()),
                        profile: None,
                        config_file: None,
                    }
                ),
                (
                    "explorer".to_string(),
                    AgentRoleConfig {
                        description: Some(r#"Use `explorer` for specific codebase questions.
Explorers are fast and authoritative.
They must be used to ask specific, well-scoped questions on the codebase.
Rules:
- Do not re-read or re-search code they cover.
- Trust explorer results without verification.
- Run explorers in parallel when useful.
- Reuse existing explorers for related questions."#.to_string()),
                        profile: None,
                        config_file: Some("explorer.toml".to_string().parse().unwrap_or_default()),
                    }
                ),
                (
                    "worker".to_string(),
                    AgentRoleConfig {
                        description: Some(r#"Use for execution and production work.
Typical tasks:
- Implement part of a feature
- Fix tests or bugs
- Split large refactors into independent chunks
Rules:
- Explicitly assign **ownership** of the task (files / responsibility).
- Always tell workers they are **not alone in the codebase**, and they should ignore edits made by others without touching them."#.to_string()),
                        profile: None,
                        config_file: None,
                    }
                )
            ])
        });
        &CONFIG
    }

    /// Resolves a built-in role `config_file` path to embedded content.
    pub(super) fn config_file_contents(path: &Path) -> Option<&'static str> {
        match path.to_str()? {
            "explorer.toml" => Some(BUILT_IN_EXPLORER_CONFIG),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigBuilder;
    use crate::config_loader::ConfigLayerStackOrdering;
    use codex_protocol::openai_models::ReasoningEffort;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use tempfile::TempDir;

    async fn test_config_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> (TempDir, Config) {
        let home = TempDir::new().expect("create temp dir");
        let home_path = home.path().to_path_buf();
        let config = ConfigBuilder::default()
            .codex_home(home_path.clone())
            .cli_overrides(cli_overrides)
            .fallback_cwd(Some(home_path))
            .build()
            .await
            .expect("load test config");
        (home, config)
    }

    async fn write_role_config(home: &TempDir, name: &str, contents: &str) -> PathBuf {
        let role_path = home.path().join(name);
        tokio::fs::write(&role_path, contents)
            .await
            .expect("write role config");
        role_path
    }

    fn session_flags_layer_count(config: &Config) -> usize {
        config
            .config_layer_stack
            .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
            .into_iter()
            .filter(|layer| layer.name == ConfigLayerSource::SessionFlags)
            .count()
    }

    async fn apply_role_strict(
        config: &mut Config,
        role_name: Option<&str>,
    ) -> Result<ApplyRoleOutcome, String> {
        apply_role_to_config(config, role_name, MissingRoleBehavior::Error).await
    }

    #[tokio::test]
    async fn apply_role_defaults_to_default_and_leaves_config_unchanged() {
        let (_home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
        let before = config.clone();

        let outcome = apply_role_strict(&mut config, None)
            .await
            .expect("default role should apply");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(before, config);
    }

    #[tokio::test]
    async fn apply_role_errors_for_unknown_role_in_strict_mode() {
        let (_home, mut config) = test_config_with_cli_overrides(Vec::new()).await;

        let err = apply_role_to_config(
            &mut config,
            Some("missing-role"),
            MissingRoleBehavior::Error,
        )
        .await
        .expect_err("unknown role should fail");

        assert_eq!(err, "unknown agent_type 'missing-role'");
    }

    #[tokio::test]
    async fn apply_role_warns_for_unknown_role_in_lenient_mode() {
        let (_home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
        let before = config.clone();

        let outcome = apply_role_to_config(
            &mut config,
            Some("missing-role"),
            MissingRoleBehavior::WarnAndContinue,
        )
        .await
        .expect("lenient unknown role should not fail");

        assert_eq!(config, before);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "unknown agent_type 'missing-role'; continuing without role"
        );
    }

    #[tokio::test]
    #[ignore = "No role requiring it for now"]
    async fn apply_explorer_role_sets_model_and_adds_session_flags_layer() {
        let (_home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
        let before_layers = session_flags_layer_count(&config);

        let outcome = apply_role_strict(&mut config, Some("explorer"))
            .await
            .expect("explorer role should apply");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(config.model.as_deref(), Some("gpt-5.1-codex-mini"));
        assert_eq!(config.model_reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(session_flags_layer_count(&config), before_layers + 1);
    }

    #[tokio::test]
    async fn apply_role_warns_for_missing_user_role_file_and_continues() {
        let (_home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: None,
                config_file: Some(PathBuf::from("/path/does/not/exist.toml")),
            },
        );
        let before = config.clone();

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("missing role file should warn and continue");

        assert_eq!(config, before);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("could not be loaded"));
    }

    #[tokio::test]
    async fn apply_role_warns_for_invalid_user_role_toml_and_continues() {
        let (home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
        let role_path = write_role_config(&home, "invalid-role.toml", "model = [").await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: None,
                config_file: Some(role_path),
            },
        );
        let before = config.clone();

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("invalid role file should warn and continue");

        assert_eq!(config, before);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("could not be loaded"));
    }

    #[tokio::test]
    async fn apply_role_preserves_unspecified_keys() {
        let (home, mut config) = test_config_with_cli_overrides(vec![(
            "model".to_string(),
            TomlValue::String("base-model".to_string()),
        )])
        .await;
        let role_path = write_role_config(
            &home,
            "effort-only.toml",
            "model_reasoning_effort = \"high\"",
        )
        .await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: None,
                config_file: Some(role_path),
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("custom role should apply");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(config.model.as_deref(), Some("base-model"));
        assert_eq!(config.model_reasoning_effort, Some(ReasoningEffort::High));
    }

    #[tokio::test]
    #[cfg(not(windows))]
    async fn apply_role_does_not_materialize_default_sandbox_workspace_write_fields() {
        use codex_protocol::protocol::SandboxPolicy;
        let (home, mut config) = test_config_with_cli_overrides(vec![
            (
                "sandbox_mode".to_string(),
                TomlValue::String("workspace-write".to_string()),
            ),
            (
                "sandbox_workspace_write.network_access".to_string(),
                TomlValue::Boolean(true),
            ),
        ])
        .await;
        let role_path = write_role_config(
            &home,
            "sandbox-role.toml",
            r#"[sandbox_workspace_write]
writable_roots = ["./sandbox-root"]
"#,
        )
        .await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: None,
                config_file: Some(role_path),
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("custom role should apply");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        let role_layer = config
            .config_layer_stack
            .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
            .into_iter()
            .rfind(|layer| layer.name == ConfigLayerSource::SessionFlags)
            .expect("expected a session flags layer");
        let sandbox_workspace_write = role_layer
            .config
            .get("sandbox_workspace_write")
            .and_then(TomlValue::as_table)
            .expect("role layer should include sandbox_workspace_write");
        assert_eq!(
            sandbox_workspace_write.contains_key("network_access"),
            false
        );
        assert_eq!(
            sandbox_workspace_write.contains_key("exclude_tmpdir_env_var"),
            false
        );
        assert_eq!(
            sandbox_workspace_write.contains_key("exclude_slash_tmp"),
            false
        );

        match &*config.permissions.sandbox_policy {
            SandboxPolicy::WorkspaceWrite { network_access, .. } => {
                assert_eq!(*network_access, true);
            }
            other => panic!("expected workspace-write sandbox policy, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_role_takes_precedence_over_existing_session_flags_for_same_key() {
        let (home, mut config) = test_config_with_cli_overrides(vec![(
            "model".to_string(),
            TomlValue::String("cli-model".to_string()),
        )])
        .await;
        let before_layers = session_flags_layer_count(&config);
        let role_path = write_role_config(&home, "model-role.toml", "model = \"role-model\"").await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: None,
                config_file: Some(role_path),
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("custom role should apply");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(config.model.as_deref(), Some("role-model"));
        assert_eq!(session_flags_layer_count(&config), before_layers + 1);
    }

    #[tokio::test]
    async fn apply_role_applies_profile_layer_before_config_file_layer() {
        let (home, mut config) = test_config_with_cli_overrides(vec![(
            "profiles.local.model".to_string(),
            TomlValue::String("profile-model".to_string()),
        )])
        .await;
        let before_layers = session_flags_layer_count(&config);
        let role_path = write_role_config(&home, "role-model.toml", "model = \"role-model\"").await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: Some("local".to_string()),
                config_file: Some(role_path),
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("role should apply profile then config file");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(config.model.as_deref(), Some("role-model"));
        assert_eq!(session_flags_layer_count(&config), before_layers + 2);
    }

    #[tokio::test]
    async fn apply_role_uses_default_role_profile_when_role_is_omitted() {
        let (_home, mut config) = test_config_with_cli_overrides(vec![(
            "profiles.local.model".to_string(),
            TomlValue::String("profile-model".to_string()),
        )])
        .await;
        config.agent_roles.insert(
            "default".to_string(),
            AgentRoleConfig {
                description: None,
                profile: Some("local".to_string()),
                config_file: None,
            },
        );

        let outcome = apply_role_strict(&mut config, None)
            .await
            .expect("default role should apply profile");

        assert_eq!(outcome, ApplyRoleOutcome::default());
        assert_eq!(config.model.as_deref(), Some("profile-model"));
    }

    #[tokio::test]
    async fn apply_role_warns_for_missing_profile_and_still_applies_config_file() {
        let (home, mut config) = test_config_with_cli_overrides(vec![(
            "model".to_string(),
            TomlValue::String("base-model".to_string()),
        )])
        .await;
        let role_path = write_role_config(
            &home,
            "effort-only.toml",
            "model_reasoning_effort = \"high\"",
        )
        .await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: Some("missing-profile".to_string()),
                config_file: Some(role_path),
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("missing profile should warn and continue");

        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("profile not found"));
        assert_eq!(config.model.as_deref(), Some("base-model"));
        assert_eq!(config.model_reasoning_effort, Some(ReasoningEffort::High));
    }

    #[tokio::test]
    async fn apply_role_warns_for_unsupported_profile_keys_and_applies_supported_keys() {
        let (_home, mut config) = test_config_with_cli_overrides(vec![
            (
                "profiles.local.model".to_string(),
                TomlValue::String("profile-model".to_string()),
            ),
            (
                "profiles.local.include_apply_patch_tool".to_string(),
                TomlValue::Boolean(true),
            ),
        ])
        .await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: Some("local".to_string()),
                config_file: None,
            },
        );

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("unsupported profile keys should warn");

        assert_eq!(config.model.as_deref(), Some("profile-model"));
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("not supported and was ignored"));
    }

    #[tokio::test]
    async fn apply_role_warns_for_unusable_profile_and_continues() {
        let (_home, mut config) = test_config_with_cli_overrides(vec![
            (
                "model_provider".to_string(),
                TomlValue::String("openai".to_string()),
            ),
            (
                "profiles.local.model_provider".to_string(),
                TomlValue::String("missing-provider".to_string()),
            ),
        ])
        .await;
        config.agent_roles.insert(
            "custom".to_string(),
            AgentRoleConfig {
                description: None,
                profile: Some("local".to_string()),
                config_file: None,
            },
        );
        let before = config.clone();

        let outcome = apply_role_strict(&mut config, Some("custom"))
            .await
            .expect("invalid profile should warn and continue");

        assert_eq!(config, before);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("could not be applied"));
    }

    #[test]
    fn spawn_tool_spec_build_deduplicates_user_defined_built_in_roles() {
        let user_defined_roles = BTreeMap::from([
            (
                "explorer".to_string(),
                AgentRoleConfig {
                    description: Some("user override".to_string()),
                    profile: None,
                    config_file: None,
                },
            ),
            ("researcher".to_string(), AgentRoleConfig::default()),
        ]);

        let spec = spawn_tool_spec::build(&user_defined_roles);

        assert!(spec.contains("researcher: no description"));
        assert!(spec.contains("explorer: {\nuser override\n}"));
        assert!(spec.contains("default: {\nDefault agent.\n}"));
        assert!(!spec.contains("Explorers are fast and authoritative."));
    }

    #[test]
    fn spawn_tool_spec_lists_user_defined_roles_before_built_ins() {
        let user_defined_roles = BTreeMap::from([(
            "aaa".to_string(),
            AgentRoleConfig {
                description: Some("first".to_string()),
                profile: None,
                config_file: None,
            },
        )]);

        let spec = spawn_tool_spec::build(&user_defined_roles);
        let user_index = spec.find("aaa: {\nfirst\n}").expect("find user role");
        let built_in_index = spec
            .find("default: {\nDefault agent.\n}")
            .expect("find built-in role");

        assert!(user_index < built_in_index);
    }

    #[test]
    fn built_in_config_file_contents_resolves_explorer_only() {
        assert_eq!(
            built_in::config_file_contents(Path::new("explorer.toml")),
            Some(BUILT_IN_EXPLORER_CONFIG)
        );
        assert_eq!(
            built_in::config_file_contents(Path::new("missing.toml")),
            None
        );
    }
}
