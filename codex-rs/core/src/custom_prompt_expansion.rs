use codex_protocol::custom_prompts::CustomPrompt;
use codex_protocol::custom_prompts::PROMPTS_CMD_PREFIX;
use once_cell::sync::Lazy;
use regex_lite::Regex;
use std::collections::HashMap;
use std::collections::HashSet;

static PROMPT_ARG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\$[A-Z][A-Z0-9_]*").unwrap_or_else(|_| std::process::abort()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptArgsError {
    MissingAssignment { token: String },
    MissingKey { token: String },
}

impl PromptArgsError {
    fn describe(&self, command: &str) -> String {
        match self {
            PromptArgsError::MissingAssignment { token } => format!(
                "Could not parse {command}: expected key=value but found '{token}'. Wrap values in double quotes if they contain spaces."
            ),
            PromptArgsError::MissingKey { token } => {
                format!("Could not parse {command}: expected a name before '=' in '{token}'.")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptExpansionError {
    Args {
        command: String,
        error: PromptArgsError,
    },
    MissingArgs {
        command: String,
        missing: Vec<String>,
    },
}

impl PromptExpansionError {
    pub fn user_message(&self) -> String {
        match self {
            PromptExpansionError::Args { command, error } => error.describe(command),
            PromptExpansionError::MissingArgs { command, missing } => {
                let list = missing.join(", ");
                format!(
                    "Missing required args for {command}: {list}. Provide as key=value (quote values with spaces)."
                )
            }
        }
    }
}

fn parse_slash_name(line: &str) -> Option<(&str, &str)> {
    let stripped = line.strip_prefix('/')?;
    let mut name_end = stripped.len();
    for (idx, ch) in stripped.char_indices() {
        if ch.is_whitespace() {
            name_end = idx;
            break;
        }
    }
    let name = &stripped[..name_end];
    if name.is_empty() {
        return None;
    }
    let rest_untrimmed = &stripped[name_end..];
    let rest = rest_untrimmed.trim_start();
    Some((name, rest))
}

fn parse_tokens(rest: &str) -> Vec<String> {
    shlex::split(rest).unwrap_or_else(|| rest.split_whitespace().map(ToString::to_string).collect())
}

fn parse_positional_args(rest: &str) -> Vec<String> {
    if rest.trim().is_empty() {
        return Vec::new();
    }
    parse_tokens(rest)
}

fn parse_prompt_inputs(rest: &str) -> Result<HashMap<String, String>, PromptArgsError> {
    let mut map = HashMap::new();
    if rest.trim().is_empty() {
        return Ok(map);
    }
    for token in parse_tokens(rest) {
        let Some((key, value)) = token.split_once('=') else {
            return Err(PromptArgsError::MissingAssignment { token });
        };
        if key.is_empty() {
            return Err(PromptArgsError::MissingKey { token });
        }
        map.insert(key.to_string(), value.to_string());
    }
    Ok(map)
}

fn prompt_argument_names(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for m in PROMPT_ARG_REGEX.find_iter(content) {
        if m.start() > 0 && content.as_bytes()[m.start() - 1] == b'$' {
            continue;
        }
        let name = &content[m.start() + 1..m.end()];
        if name == "ARGUMENTS" {
            continue;
        }
        if seen.insert(name.to_string()) {
            names.push(name.to_string());
        }
    }
    names
}

fn expand_named_placeholders(content: &str, args: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    for m in PROMPT_ARG_REGEX.find_iter(content) {
        let start = m.start();
        let end = m.end();
        if start > 0 && content.as_bytes()[start - 1] == b'$' {
            out.push_str(&content[cursor..end]);
            cursor = end;
            continue;
        }
        out.push_str(&content[cursor..start]);
        cursor = end;
        let key = &content[start + 1..end];
        if let Some(arg) = args.get(key) {
            out.push_str(arg);
        } else {
            out.push_str(&content[start..end]);
        }
    }
    out.push_str(&content[cursor..]);
    out
}

fn expand_numeric_placeholders(content: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while let Some(off) = content[i..].find('$') {
        let j = i + off;
        out.push_str(&content[i..j]);
        let rest = &content[j..];
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 {
            match bytes[1] {
                b'$' => {
                    out.push_str("$$");
                    i = j + 2;
                    continue;
                }
                b'1'..=b'9' => {
                    let idx = (bytes[1] - b'1') as usize;
                    if let Some(arg) = args.get(idx) {
                        out.push_str(arg);
                    }
                    i = j + 2;
                    continue;
                }
                _ => {}
            }
        }
        if rest.len() > "ARGUMENTS".len() && rest[1..].starts_with("ARGUMENTS") {
            if !args.is_empty() {
                out.push_str(&args.join(" "));
            }
            i = j + 1 + "ARGUMENTS".len();
            continue;
        }
        out.push('$');
        i = j + 1;
    }
    out.push_str(&content[i..]);
    out
}

/// Expand a message of the form `/prompts:name [value] [value] …` using a matching saved prompt.
///
/// Returns `Ok(None)` if the text does not start with `/prompts:` or no prompt matches.
pub fn expand_custom_prompt_text(
    text: &str,
    custom_prompts: &[CustomPrompt],
) -> Result<Option<String>, PromptExpansionError> {
    let trimmed = text.trim_start();
    let Some((name, rest)) = parse_slash_name(trimmed) else {
        return Ok(None);
    };
    let Some(prompt_name) = name.strip_prefix(&format!("{PROMPTS_CMD_PREFIX}:")) else {
        return Ok(None);
    };
    let prompt = match custom_prompts.iter().find(|p| p.name == prompt_name) {
        Some(prompt) => prompt,
        None => return Ok(None),
    };

    let required = prompt_argument_names(&prompt.content);
    if !required.is_empty() {
        let inputs = parse_prompt_inputs(rest).map_err(|error| PromptExpansionError::Args {
            command: format!("/{name}"),
            error,
        })?;
        let missing: Vec<String> = required
            .into_iter()
            .filter(|key| !inputs.contains_key(key))
            .collect();
        if !missing.is_empty() {
            return Err(PromptExpansionError::MissingArgs {
                command: format!("/{name}"),
                missing,
            });
        }
        return Ok(Some(expand_named_placeholders(&prompt.content, &inputs)));
    }

    let pos_args = parse_positional_args(rest);
    Ok(Some(expand_numeric_placeholders(
        &prompt.content,
        &pos_args,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn prompt(name: &str, content: &str) -> CustomPrompt {
        CustomPrompt {
            name: name.to_string(),
            path: "dummy".into(),
            content: content.to_string(),
            description: None,
            argument_hint: None,
        }
    }

    #[test]
    fn expands_named_placeholders() {
        let prompts = vec![prompt("review", "Review $USER on $BRANCH")];
        let out =
            expand_custom_prompt_text("/prompts:review USER=Alice BRANCH=main", &prompts).unwrap();
        assert_eq!(out, Some("Review Alice on main".to_string()));
    }

    #[test]
    fn expands_numeric_placeholders() {
        let prompts = vec![prompt("deploy", "First:$1 All:$ARGUMENTS")];
        let out = expand_custom_prompt_text("/prompts:deploy prod us-west", &prompts).unwrap();
        assert_eq!(out, Some("First:prod All:prod us-west".to_string()));
    }

    #[test]
    fn reports_missing_args() {
        let prompts = vec![prompt("review", "Review $USER on $BRANCH")];
        let err = expand_custom_prompt_text("/prompts:review USER=Alice", &prompts).unwrap_err();
        assert_eq!(
            err,
            PromptExpansionError::MissingArgs {
                command: "/prompts:review".to_string(),
                missing: vec!["BRANCH".to_string()],
            }
        );
    }

    #[test]
    fn preserves_escaped_placeholders() {
        let prompts = vec![prompt("note", "literal $$USER and $USER")];
        let out = expand_custom_prompt_text("/prompts:note USER=Bob", &prompts).unwrap();
        assert_eq!(out, Some("literal $$USER and Bob".to_string()));
    }

    #[test]
    fn ignores_unknown_prompt() {
        let prompts = vec![prompt("known", "hi")];
        let out = expand_custom_prompt_text("/prompts:missing", &prompts).unwrap();
        assert_eq!(out, None);
    }
}
